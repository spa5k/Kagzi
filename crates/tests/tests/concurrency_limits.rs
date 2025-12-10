mod common;

use common::{TestHarness, json_bytes, make_request};
use kagzi_proto::kagzi::worker_service_server::WorkerService;
use kagzi_proto::kagzi::workflow_service_server::WorkflowService;
use kagzi_proto::kagzi::{
    CompleteWorkflowRequest, Payload, PollTaskRequest, RegisterRequest, StartWorkflowRequest,
};
use uuid::Uuid;

const NAMESPACE: &str = "default";
const TASK_QUEUE: &str = "default";

#[tokio::test]
/// Queue-level caps are currently enforced via reconciliation, not hot path.
/// This test documents that two workflows can be claimed back-to-back even when
/// a worker declares a queue limit of 1.
async fn queue_concurrency_limit_is_not_enforced_in_hot_path() {
    let harness = TestHarness::new().await;

    // Worker declares queue cap = 1 so only one workflow can RUN at a time.
    let worker_id = harness
        .worker_service
        .register(make_request(RegisterRequest {
            namespace_id: NAMESPACE.to_string(),
            task_queue: TASK_QUEUE.to_string(),
            workflow_types: vec!["TypeA".to_string()],
            hostname: "limited-host".to_string(),
            pid: 1234,
            version: "test".to_string(),
            max_concurrent: 5,
            labels: Default::default(),
            queue_concurrency_limit: Some(1),
            workflow_type_concurrency: vec![],
        }))
        .await
        .expect("register_worker should succeed")
        .into_inner()
        .worker_id;

    let run_a = start_workflow(&harness, "TypeA").await;
    let run_b = start_workflow(&harness, "TypeA").await;

    let first = harness
        .worker_service
        .poll_task(make_request(PollTaskRequest {
            worker_id: worker_id.clone(),
            namespace_id: NAMESPACE.to_string(),
            task_queue: TASK_QUEUE.to_string(),
            workflow_types: vec!["TypeA".to_string()],
        }))
        .await
        .expect("first workflow should be claimed")
        .into_inner();

    assert_eq!(first.run_id.to_string(), run_a);

    // Current behaviour: second claim is allowed immediately despite queue limit.
    let second = harness
        .worker_service
        .poll_task(make_request(PollTaskRequest {
            worker_id: worker_id.clone(),
            namespace_id: NAMESPACE.to_string(),
            task_queue: TASK_QUEUE.to_string(),
            workflow_types: vec!["TypeA".to_string()],
        }))
        .await
        .expect("poll_task should succeed")
        .into_inner();
    assert_eq!(
        second.run_id.to_string(),
        run_b,
        "queue cap is currently not enforced in the hot path"
    );

    // Complete workflows to satisfy harness cleanup.
    harness
        .worker_service
        .complete_workflow(make_request(CompleteWorkflowRequest {
            run_id: run_a.clone(),
            output: Some(Payload {
                data: json_bytes(&serde_json::json!({"ok": true})),
                metadata: Default::default(),
            }),
        }))
        .await
        .unwrap();

    harness
        .worker_service
        .complete_workflow(make_request(CompleteWorkflowRequest {
            run_id: run_b.clone(),
            output: Some(Payload {
                data: json_bytes(&serde_json::json!({"ok": true})),
                metadata: Default::default(),
            }),
        }))
        .await
        .unwrap();
}

async fn start_workflow(harness: &TestHarness, workflow_type: &str) -> String {
    let input_payload = Some(Payload {
        data: json_bytes(&serde_json::json!({})),
        metadata: Default::default(),
    });

    harness
        .workflow_service
        .start_workflow(make_request(StartWorkflowRequest {
            external_id: format!("{}-{}", workflow_type, Uuid::now_v7()),
            task_queue: TASK_QUEUE.to_string(),
            workflow_type: workflow_type.to_string(),
            input: input_payload,
            namespace_id: NAMESPACE.to_string(),
            context: None,
            deadline_at: None,
            version: "1.0".to_string(),
            retry_policy: None,
        }))
        .await
        .expect("start_workflow should succeed")
        .into_inner()
        .run_id
}
