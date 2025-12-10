use std::collections::HashMap;

use kagzi_proto::kagzi::workflow_service_client::WorkflowServiceClient;
use kagzi_proto::kagzi::{GetWorkflowRequest, Payload, StartWorkflowRequest, WorkflowStatus};
use tests::common::TestHarness;
use tonic::Request;
use uuid::Uuid;

fn payload_bytes<T: serde::Serialize>(val: &T) -> Vec<u8> {
    serde_json::to_vec(val).expect("serialize payload")
}

#[tokio::test]
async fn start_workflow_returns_run_id() -> anyhow::Result<()> {
    let harness = TestHarness::new().await;
    let mut client = WorkflowServiceClient::connect(harness.server_url.clone()).await?;

    let resp = client
        .start_workflow(Request::new(StartWorkflowRequest {
            external_id: Uuid::now_v7().to_string(),
            task_queue: "svc-workflow".to_string(),
            workflow_type: "TypeA".to_string(),
            input: Some(Payload {
                data: payload_bytes(&serde_json::json!({"hello": "world"})),
                metadata: HashMap::new(),
            }),
            namespace_id: "default".to_string(),
            context: None,
            deadline_at: None,
            version: "v1".to_string(),
            retry_policy: None,
        }))
        .await?
        .into_inner();

    assert!(!resp.run_id.is_empty(), "run_id should be returned");
    Ok(())
}

#[tokio::test]
async fn get_workflow_returns_correct_state() -> anyhow::Result<()> {
    let harness = TestHarness::new().await;
    let mut client = WorkflowServiceClient::connect(harness.server_url.clone()).await?;

    let start = client
        .start_workflow(Request::new(StartWorkflowRequest {
            external_id: Uuid::now_v7().to_string(),
            task_queue: "svc-workflow-state".to_string(),
            workflow_type: "TypeState".to_string(),
            input: Some(Payload {
                data: payload_bytes(&serde_json::json!({"x": 1})),
                metadata: HashMap::new(),
            }),
            namespace_id: "default".to_string(),
            context: None,
            deadline_at: None,
            version: "v1".to_string(),
            retry_policy: None,
        }))
        .await?
        .into_inner();

    let fetched = client
        .get_workflow(Request::new(GetWorkflowRequest {
            run_id: start.run_id.clone(),
            namespace_id: "default".to_string(),
        }))
        .await?
        .into_inner()
        .workflow
        .expect("workflow should exist");

    assert_eq!(fetched.run_id, start.run_id);
    assert_eq!(fetched.namespace_id, "default");
    assert_eq!(fetched.status, WorkflowStatus::Pending as i32);
    let db_status = harness
        .db_workflow_status(&Uuid::parse_str(&start.run_id)?)
        .await?;
    assert_eq!(db_status, "PENDING");
    Ok(())
}

#[tokio::test]
async fn start_workflow_rejects_oversized_input() -> anyhow::Result<()> {
    let harness = TestHarness::new().await;
    let mut client = WorkflowServiceClient::connect(harness.server_url.clone()).await?;
    let too_large = vec![0u8; 3 * 1024 * 1024];

    let result = client
        .start_workflow(Request::new(StartWorkflowRequest {
            external_id: Uuid::now_v7().to_string(),
            task_queue: "svc-workflow-oversize".to_string(),
            workflow_type: "TypeB".to_string(),
            input: Some(Payload {
                data: too_large,
                metadata: HashMap::new(),
            }),
            namespace_id: "default".to_string(),
            context: None,
            deadline_at: None,
            version: "v1".to_string(),
            retry_policy: None,
        }))
        .await;

    assert!(result.is_err(), "expected payload rejection");
    let status = result.unwrap_err();
    assert_eq!(status.code(), tonic::Code::InvalidArgument);
    Ok(())
}
