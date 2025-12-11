use std::collections::HashMap;

use kagzi_proto::kagzi::worker_service_client::WorkerServiceClient;
use kagzi_proto::kagzi::workflow_service_client::WorkflowServiceClient;
use kagzi_proto::kagzi::{
    CompleteWorkflowRequest, Payload, PollTaskRequest, RegisterRequest, StartWorkflowRequest,
};
use tests::common::TestHarness;
use tonic::Request;
use uuid::Uuid;

fn payload_bytes<T: serde::Serialize>(val: &T) -> Vec<u8> {
    serde_json::to_vec(val).expect("serialize payload")
}

#[tokio::test]
async fn register_returns_worker_id() -> anyhow::Result<()> {
    let harness = TestHarness::new().await;
    let mut client = WorkerServiceClient::connect(harness.server_url.clone()).await?;

    let resp = client
        .register(Request::new(RegisterRequest {
            namespace_id: "default".to_string(),
            task_queue: "svc-worker".to_string(),
            workflow_types: vec!["TypeA".to_string()],
            hostname: "host".to_string(),
            pid: 1234,
            version: "v1".to_string(),
            max_concurrent: 2,
            labels: HashMap::new(),
            queue_concurrency_limit: None,
            workflow_type_concurrency: vec![],
        }))
        .await?
        .into_inner();

    assert!(!resp.worker_id.is_empty());
    let db_status = harness
        .db_worker_status(&Uuid::parse_str(&resp.worker_id)?)
        .await?;
    assert_eq!(db_status, "ONLINE");
    harness.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn poll_requires_registered_worker() -> anyhow::Result<()> {
    let harness = TestHarness::new().await;
    let mut client = WorkerServiceClient::connect(harness.server_url.clone()).await?;

    let result = client
        .poll_task(Request::new(PollTaskRequest {
            task_queue: "svc-worker-poll".to_string(),
            worker_id: Uuid::now_v7().to_string(),
            namespace_id: "default".to_string(),
            workflow_types: vec!["TypeA".to_string()],
        }))
        .await;

    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code(), tonic::Code::FailedPrecondition);
    harness.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn poll_filters_by_workflow_types() -> anyhow::Result<()> {
    let harness = TestHarness::new().await;
    let mut worker_client = WorkerServiceClient::connect(harness.server_url.clone()).await?;
    let mut workflow_client = WorkflowServiceClient::connect(harness.server_url.clone()).await?;

    let run_a_id = workflow_client
        .start_workflow(Request::new(StartWorkflowRequest {
            external_id: Uuid::now_v7().to_string(),
            task_queue: "svc-worker-filter".to_string(),
            workflow_type: "TypeA".to_string(),
            input: Some(Payload {
                data: payload_bytes(&serde_json::json!({})),
                metadata: HashMap::new(),
            }),
            namespace_id: "default".to_string(),
            context: None,
            deadline_at: None,
            version: "v1".to_string(),
            retry_policy: None,
        }))
        .await?
        .into_inner()
        .run_id;

    let run_b_id = workflow_client
        .start_workflow(Request::new(StartWorkflowRequest {
            external_id: Uuid::now_v7().to_string(),
            task_queue: "svc-worker-filter".to_string(),
            workflow_type: "TypeB".to_string(),
            input: Some(Payload {
                data: payload_bytes(&serde_json::json!({})),
                metadata: HashMap::new(),
            }),
            namespace_id: "default".to_string(),
            context: None,
            deadline_at: None,
            version: "v1".to_string(),
            retry_policy: None,
        }))
        .await?
        .into_inner()
        .run_id;

    let worker = worker_client
        .register(Request::new(RegisterRequest {
            namespace_id: "default".to_string(),
            task_queue: "svc-worker-filter".to_string(),
            workflow_types: vec!["TypeA".to_string()],
            hostname: "host".to_string(),
            pid: 2222,
            version: "v1".to_string(),
            max_concurrent: 2,
            labels: HashMap::new(),
            queue_concurrency_limit: None,
            workflow_type_concurrency: vec![],
        }))
        .await?
        .into_inner();

    let first = worker_client
        .poll_task(Request::new(PollTaskRequest {
            task_queue: "svc-worker-filter".to_string(),
            worker_id: worker.worker_id.clone(),
            namespace_id: "default".to_string(),
            workflow_types: vec!["TypeA".to_string()],
        }))
        .await?
        .into_inner();
    assert_eq!(first.run_id, run_a_id);
    assert_eq!(first.workflow_type, "TypeA");

    worker_client
        .complete_workflow(Request::new(CompleteWorkflowRequest {
            run_id: first.run_id.clone(),
            output: Some(Payload {
                data: payload_bytes(&serde_json::json!({ "ok": true })),
                metadata: HashMap::new(),
            }),
        }))
        .await?;

    let worker_b = worker_client
        .register(Request::new(RegisterRequest {
            namespace_id: "default".to_string(),
            task_queue: "svc-worker-filter".to_string(),
            workflow_types: vec!["TypeB".to_string()],
            hostname: "host".to_string(),
            pid: 3333,
            version: "v1".to_string(),
            max_concurrent: 2,
            labels: HashMap::new(),
            queue_concurrency_limit: None,
            workflow_type_concurrency: vec![],
        }))
        .await?
        .into_inner();

    let second = worker_client
        .poll_task(Request::new(PollTaskRequest {
            task_queue: "svc-worker-filter".to_string(),
            worker_id: worker_b.worker_id.clone(),
            namespace_id: "default".to_string(),
            workflow_types: vec!["TypeB".to_string()],
        }))
        .await?
        .into_inner();

    assert_eq!(
        second.workflow_type, "TypeB",
        "TypeB worker should receive TypeB"
    );
    assert_eq!(
        second.run_id, run_b_id,
        "TypeB run should be delivered to matching worker"
    );
    worker_client
        .complete_workflow(Request::new(CompleteWorkflowRequest {
            run_id: second.run_id,
            output: Some(Payload {
                data: payload_bytes(&serde_json::json!({ "ok": true })),
                metadata: HashMap::new(),
            }),
        }))
        .await?;
    harness.shutdown().await?;
    Ok(())
}
