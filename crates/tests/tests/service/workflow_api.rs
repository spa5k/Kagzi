use std::collections::HashMap;

use kagzi_proto::kagzi::workflow_service_client::WorkflowServiceClient;
use kagzi_proto::kagzi::{
    GetWorkflowRequest, ListWorkflowsRequest, PageRequest, Payload, StartWorkflowRequest,
    WorkflowStatus,
};
use tests::common::{TestHarness, payload_bytes};
use tonic::Request;
use uuid::Uuid;

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
            deadline_at: None,
            version: "v1".to_string(),
            retry_policy: None,
        }))
        .await?
        .into_inner();

    assert!(!resp.run_id.is_empty(), "run_id should be returned");
    harness.shutdown().await?;
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
    harness.shutdown().await?;
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
            deadline_at: None,
            version: "v1".to_string(),
            retry_policy: None,
        }))
        .await;

    assert!(result.is_err(), "expected payload rejection");
    let status = result.unwrap_err();
    assert_eq!(status.code(), tonic::Code::InvalidArgument);
    harness.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn start_workflow_accepts_large_but_within_limit_input() -> anyhow::Result<()> {
    let harness = TestHarness::new().await;
    let mut client = WorkflowServiceClient::connect(harness.server_url.clone()).await?;
    // 512 KB payload should be accepted (well below the 3MB rejection threshold).
    let payload = vec![1u8; 512 * 1024];

    let resp = client
        .start_workflow(Request::new(StartWorkflowRequest {
            external_id: Uuid::now_v7().to_string(),
            task_queue: "svc-workflow-accepts".to_string(),
            workflow_type: "TypePayloadOk".to_string(),
            input: Some(Payload {
                data: payload,
                metadata: HashMap::new(),
            }),
            namespace_id: "default".to_string(),
            deadline_at: None,
            version: "v1".to_string(),
            retry_policy: None,
        }))
        .await?
        .into_inner();

    assert!(
        !resp.run_id.is_empty(),
        "run_id should be returned for acceptable payload size"
    );
    harness.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn list_workflows_paginates_results() -> anyhow::Result<()> {
    let harness = TestHarness::new().await;
    let mut client = WorkflowServiceClient::connect(harness.server_url.clone()).await?;

    // Seed multiple workflows.
    for _ in 0..5 {
        let _ = client
            .start_workflow(Request::new(StartWorkflowRequest {
                external_id: Uuid::now_v7().to_string(),
                task_queue: "svc-workflow-page".to_string(),
                workflow_type: "TypePaged".to_string(),
                input: Some(Payload {
                    data: payload_bytes(&serde_json::json!({ "v": 1 })),
                    metadata: HashMap::new(),
                }),
                namespace_id: "default".to_string(),
                deadline_at: None,
                version: "v1".to_string(),
                retry_policy: None,
            }))
            .await?;
    }

    let first = client
        .list_workflows(Request::new(ListWorkflowsRequest {
            namespace_id: "default".to_string(),
            status_filter: None,
            page: Some(PageRequest {
                page_size: 2,
                page_token: "".into(),
                include_total_count: true,
            }),
        }))
        .await?
        .into_inner();

    assert_eq!(first.workflows.len(), 2, "page_size=2 should cap results");
    let page_info = first.page.expect("page info should be present");
    assert!(page_info.has_more, "should indicate more pages");
    assert!(
        !page_info.next_page_token.is_empty(),
        "next_page_token should be present"
    );
    assert!(
        page_info.total_count >= 5,
        "total_count should reflect seeded workflows"
    );

    let second = client
        .list_workflows(Request::new(ListWorkflowsRequest {
            namespace_id: "default".to_string(),
            status_filter: None,
            page: Some(PageRequest {
                page_size: 2,
                page_token: page_info.next_page_token.clone(),
                include_total_count: false,
            }),
        }))
        .await?
        .into_inner();

    assert!(
        !second.workflows.is_empty(),
        "second page should return workflows"
    );
    // Ensure no overlap between first and second page by run_id.
    let first_ids: std::collections::HashSet<_> =
        first.workflows.iter().map(|w| w.run_id.clone()).collect();
    for wf in &second.workflows {
        assert!(
            !first_ids.contains(&wf.run_id),
            "run_id {} should not repeat across pages",
            wf.run_id
        );
    }

    harness.shutdown().await?;
    Ok(())
}
