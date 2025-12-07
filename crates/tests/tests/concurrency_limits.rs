#![cfg(feature = "legacy-service")]

mod common;

use common::{TestHarness, json_bytes, make_request};
use kagzi_proto::kagzi::workflow_service_server::WorkflowService;
use kagzi_proto::kagzi::{CompleteWorkflowRequest, RegisterWorkerRequest, StartWorkflowRequest};
use std::time::Duration;
use uuid::Uuid;

const NAMESPACE: &str = "test-ns";
const TASK_QUEUE: &str = "default";

/// Ensures queue-level concurrency cap blocks additional claims until a slot frees.
#[tokio::test]
async fn queue_concurrency_limit_is_enforced() {
    let harness = TestHarness::new().await;

    // Worker declares queue cap = 1 so only one workflow can RUN at a time.
    let worker_id = harness
        .service
        .register_worker(make_request(RegisterWorkerRequest {
            namespace_id: NAMESPACE.to_string(),
            task_queue: TASK_QUEUE.to_string(),
            workflow_types: vec!["TypeA".to_string()],
            hostname: "limited-host".to_string(),
            pid: 1234,
            version: "test".to_string(),
            max_concurrent: 5,
            labels: Default::default(),
            queue_concurrency_limit: 1,
            workflow_type_concurrency: vec![],
        }))
        .await
        .expect("register_worker should succeed")
        .into_inner()
        .worker_id;

    let run_a = start_workflow(&harness, "TypeA").await;
    let run_b = start_workflow(&harness, "TypeA").await;

    let first = harness
        .service
        .work_distributor
        .wait_for_work(
            TASK_QUEUE,
            NAMESPACE,
            &worker_id,
            &[String::from("TypeA")],
            Duration::from_secs(2),
        )
        .await
        .expect("first workflow should be claimed");

    assert_eq!(first.run_id.to_string(), run_a);

    // Second claim should be blocked by queue limit until slot frees.
    let none = harness
        .service
        .work_distributor
        .wait_for_work(
            TASK_QUEUE,
            NAMESPACE,
            &worker_id,
            &[String::from("TypeA")],
            Duration::from_millis(200),
        )
        .await;

    assert!(none.is_none(), "queue cap should prevent second claim");

    // Complete first workflow to free slot.
    harness
        .service
        .complete_workflow(make_request(CompleteWorkflowRequest {
            run_id: run_a.clone(),
            output: json_bytes(&serde_json::json!({"ok": true})),
        }))
        .await
        .unwrap();

    let second = harness
        .service
        .work_distributor
        .wait_for_work(
            TASK_QUEUE,
            NAMESPACE,
            &worker_id,
            &[String::from("TypeA")],
            Duration::from_secs(2),
        )
        .await
        .expect("second workflow should be claimable after slot frees");

    assert_eq!(second.run_id.to_string(), run_b);
}

async fn start_workflow(harness: &TestHarness, workflow_type: &str) -> String {
    harness
        .service
        .start_workflow(make_request(StartWorkflowRequest {
            external_id: format!("{}-{}", workflow_type, Uuid::new_v4()),
            task_queue: TASK_QUEUE.to_string(),
            workflow_type: workflow_type.to_string(),
            input: json_bytes(&serde_json::json!({})),
            namespace_id: NAMESPACE.to_string(),
            context: vec![],
            deadline_at: None,
            version: "1.0".to_string(),
            retry_policy: None,
        }))
        .await
        .expect("start_workflow should succeed")
        .into_inner()
        .run_id
}
