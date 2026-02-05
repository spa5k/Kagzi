use kagzi_proto::kagzi::claim_task_response;
use kagzi_proto::kagzi::worker_service_client::WorkerServiceClient;
use kagzi_proto::kagzi::workflow_service_client::WorkflowServiceClient;
use kagzi_proto::kagzi::{ClaimTaskRequest, DeregisterRequest, HeartbeatRequest, RegisterRequest};
use tests::common::TestHarness;
use tonic::Request;
use uuid::Uuid;

#[tokio::test]
async fn claim_task_rejects_when_draining() -> anyhow::Result<()> {
    let harness = TestHarness::new().await;

    let mut client = WorkerServiceClient::connect(harness.server_url.clone()).await?;

    let register = client
        .register(RegisterRequest {
            namespace: "default".to_string(),
            task_queue: Some("process-order".to_string()),
            workflow_types: vec!["process-order".to_string()],
            hostname: "test-worker".to_string(),
            pid: 1,
            version: "test".to_string(),
            labels: Default::default(),
            queue_concurrency_limit: None,
            workflow_type_concurrency: vec![],
        })
        .await?
        .into_inner();

    // Create runnable work.
    let kagzi = harness.client().await;
    let _run = kagzi.start("process-order").send().await?;

    // Mark draining.
    client
        .deregister(DeregisterRequest {
            worker_id: register.worker_id.clone(),
            drain: true,
        })
        .await?;

    // Draining workers must not claim new tasks.
    let err = client
        .claim_task(ClaimTaskRequest {
            namespace: "default".to_string(),
            worker_id: register.worker_id,
            task_queue: "process-order".to_string(),
            workflow_types: vec!["process-order".to_string()],
        })
        .await
        .unwrap_err();

    assert_eq!(err.code(), tonic::Code::FailedPrecondition);
    Ok(())
}

#[tokio::test]
async fn claim_task_allows_empty_workflow_types_as_no_filter() -> anyhow::Result<()> {
    let harness = TestHarness::new().await;

    let mut client = WorkerServiceClient::connect(harness.server_url.clone()).await?;

    let register = client
        .register(RegisterRequest {
            namespace: "default".to_string(),
            task_queue: Some("process-order".to_string()),
            workflow_types: vec!["process-order".to_string()],
            hostname: "test-worker".to_string(),
            pid: 1,
            version: "test".to_string(),
            labels: Default::default(),
            queue_concurrency_limit: None,
            workflow_type_concurrency: vec![],
        })
        .await?
        .into_inner();

    // Create runnable work in the same task queue.
    let mut wf_client = WorkflowServiceClient::connect(harness.server_url.clone()).await?;
    wf_client
        .start_workflow(Request::new(kagzi_proto::kagzi::StartWorkflowRequest {
            external_id: Uuid::now_v7().to_string(),
            task_queue: Some("process-order".to_string()),
            workflow_type: "process-order".to_string(),
            input: None,
            namespace: "default".to_string(),
            version: String::default(),
            retry_policy: None,
        }))
        .await?;

    // Prevent the coordinator from marking the worker stale/offline during the claim loop.
    client
        .heartbeat(HeartbeatRequest {
            worker_id: register.worker_id.clone(),
        })
        .await?;

    let resp = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        let mut last_heartbeat = std::time::Instant::now();
        loop {
            if last_heartbeat.elapsed() >= std::time::Duration::from_secs(1) {
                client
                    .heartbeat(HeartbeatRequest {
                        worker_id: register.worker_id.clone(),
                    })
                    .await?;
                last_heartbeat = std::time::Instant::now();
            }

            let resp = client
                .claim_task(ClaimTaskRequest {
                    namespace: "default".to_string(),
                    worker_id: register.worker_id.clone(),
                    task_queue: "process-order".to_string(),
                    workflow_types: vec![],
                })
                .await?
                .into_inner();

            if matches!(resp.result, Some(claim_task_response::Result::Task(_))) {
                return Ok::<_, tonic::Status>(resp);
            }

            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .map_err(|_| anyhow::anyhow!("Timed out waiting to claim a task"))??;

    let Some(claim_task_response::Result::Task(task)) = resp.result else {
        anyhow::bail!("Expected a claimed task");
    };

    assert_eq!(task.workflow_type, "process-order");
    Ok(())
}

#[tokio::test]
async fn claim_task_rejects_unregistered_workflow_type() -> anyhow::Result<()> {
    let harness = TestHarness::new().await;

    let mut client = WorkerServiceClient::connect(harness.server_url.clone()).await?;

    let register = client
        .register(RegisterRequest {
            namespace: "default".to_string(),
            task_queue: Some("type-a".to_string()),
            workflow_types: vec!["type-a".to_string()],
            hostname: "test-worker".to_string(),
            pid: 1,
            version: "test".to_string(),
            labels: Default::default(),
            queue_concurrency_limit: None,
            workflow_type_concurrency: vec![],
        })
        .await?
        .into_inner();

    // Requested workflow_types that are not registered must be rejected.
    let err = client
        .claim_task(ClaimTaskRequest {
            namespace: "default".to_string(),
            worker_id: register.worker_id,
            task_queue: "type-a".to_string(),
            workflow_types: vec!["type-b".to_string()],
        })
        .await
        .unwrap_err();

    assert_eq!(err.code(), tonic::Code::FailedPrecondition);
    Ok(())
}

#[tokio::test]
async fn claim_task_rejects_wrong_task_queue() -> anyhow::Result<()> {
    let harness = TestHarness::new().await;

    let mut client = WorkerServiceClient::connect(harness.server_url.clone()).await?;

    let register = client
        .register(RegisterRequest {
            namespace: "default".to_string(),
            task_queue: Some("queue-a".to_string()),
            workflow_types: vec!["queue-a".to_string()],
            hostname: "test-worker".to_string(),
            pid: 1,
            version: "test".to_string(),
            labels: Default::default(),
            queue_concurrency_limit: None,
            workflow_type_concurrency: vec![],
        })
        .await?
        .into_inner();

    let err = client
        .claim_task(ClaimTaskRequest {
            namespace: "default".to_string(),
            worker_id: register.worker_id,
            task_queue: "queue-b".to_string(),
            workflow_types: vec!["queue-a".to_string()],
        })
        .await
        .unwrap_err();

    assert_eq!(err.code(), tonic::Code::FailedPrecondition);
    Ok(())
}
