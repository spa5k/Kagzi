use std::collections::HashMap;

use kagzi_proto::kagzi::admin_service_client::AdminServiceClient;
use kagzi_proto::kagzi::worker_service_client::WorkerServiceClient;
use kagzi_proto::kagzi::{ListWorkersRequest, PageRequest, RegisterRequest};
use tests::common::TestHarness;
use tonic::Request;

#[tokio::test]
async fn list_workers_returns_registered() -> anyhow::Result<()> {
    let harness = TestHarness::new().await;
    let mut worker_client = WorkerServiceClient::connect(harness.server_url.clone()).await?;

    for pid in [1111, 2222] {
        worker_client
            .register(Request::new(RegisterRequest {
                namespace_id: "default".to_string(),
                task_queue: "svc-admin".to_string(),
                workflow_types: vec!["TypeA".to_string()],
                hostname: "host".to_string(),
                pid,
                version: "v1".to_string(),
                max_concurrent: 2,
                labels: HashMap::new(),
                queue_concurrency_limit: None,
                workflow_type_concurrency: vec![],
            }))
            .await?;
    }

    let mut admin_client = AdminServiceClient::connect(harness.server_url.clone()).await?;
    let resp = admin_client
        .list_workers(Request::new(ListWorkersRequest {
            namespace_id: "default".to_string(),
            task_queue: None,
            status_filter: None,
            page: Some(PageRequest {
                page_size: 10,
                page_token: String::new(),
                include_total_count: false,
            }),
        }))
        .await?
        .into_inner();

    assert!(
        resp.workers.len() >= 2,
        "expected at least two workers, got {}",
        resp.workers.len()
    );
    Ok(())
}
