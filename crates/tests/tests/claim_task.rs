use kagzi_proto::kagzi::worker_service_client::WorkerServiceClient;
use kagzi_proto::kagzi::{ClaimTaskRequest, DeregisterRequest, RegisterRequest};
use tests::common::TestHarness;

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
