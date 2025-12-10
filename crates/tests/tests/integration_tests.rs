//! Integration tests for worker lifecycle + workflow execution under the new
//! worker registry/heartbeat model.

mod common;

use std::time::Duration;

use common::{TestHarness, json_bytes, make_request};
use kagzi_proto::kagzi::admin_service_server::AdminService;
use kagzi_proto::kagzi::worker_service_server::WorkerService;
use kagzi_proto::kagzi::workflow_service_server::WorkflowService;
use kagzi_proto::kagzi::{
    CompleteWorkflowRequest, DeregisterRequest, ErrorCode, ErrorDetail, GetWorkflowRequest,
    HeartbeatRequest, ListWorkersRequest, PageRequest, Payload, PollTaskRequest, RegisterRequest,
    StartWorkflowRequest, WorkerStatus, WorkflowStatus,
};
use kagzi_store::{PgStore, WorkflowRepository};
use prost::Message;
use tonic::Code;
use uuid::Uuid;

const NAMESPACE: &str = "default";
const TASK_QUEUE: &str = "default";

async fn register_worker(harness: &TestHarness, workflow_types: Vec<&str>) -> String {
    let host = format!("test-host-{}", Uuid::now_v7());
    let pid = ((Uuid::now_v7().as_u128() % 30_000) as i32).max(1);

    harness
        .worker_service
        .register(make_request(RegisterRequest {
            namespace_id: NAMESPACE.to_string(),
            task_queue: TASK_QUEUE.to_string(),
            workflow_types: workflow_types.into_iter().map(|s| s.to_string()).collect(),
            hostname: host,
            pid,
            version: "test".to_string(),
            max_concurrent: 5,
            labels: Default::default(),
            queue_concurrency_limit: None,
            workflow_type_concurrency: vec![],
        }))
        .await
        .expect("register_worker should succeed")
        .into_inner()
        .worker_id
}

async fn start_workflow(
    harness: &TestHarness,
    workflow_type: &str,
    input: serde_json::Value,
) -> String {
    let input_payload = Some(Payload {
        data: json_bytes(&input),
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

fn poll_request(worker_id: &str, supported_types: &[&str]) -> PollTaskRequest {
    PollTaskRequest {
        task_queue: TASK_QUEUE.to_string(),
        worker_id: worker_id.to_string(),
        namespace_id: NAMESPACE.to_string(),
        workflow_types: supported_types.iter().map(|s| s.to_string()).collect(),
    }
}

#[tokio::test]
async fn workflow_happy_path_with_registered_worker() {
    let harness = TestHarness::new().await;
    let worker_id = register_worker(&harness, vec!["TypeA"]).await;
    let run_id = start_workflow(&harness, "TypeA", serde_json::json!({"user_id": 123})).await;

    // Sanity check workflow state before polling using service API.
    let workflow = harness
        .workflow_service
        .get_workflow(make_request(GetWorkflowRequest {
            run_id: run_id.clone(),
            namespace_id: NAMESPACE.to_string(),
        }))
        .await
        .expect("workflow should exist")
        .into_inner()
        .workflow
        .expect("workflow should be present");

    assert_eq!(workflow.workflow_type, "TypeA");
    assert_eq!(workflow.namespace_id, NAMESPACE);
    assert_eq!(workflow.task_queue, TASK_QUEUE);
    assert_eq!(workflow.status, WorkflowStatus::Pending as i32);

    let work = harness
        .worker_service
        .poll_task(make_request(poll_request(&worker_id, &["TypeA"])))
        .await
        .expect("poll_task should succeed")
        .into_inner();

    assert_eq!(work.run_id.to_string(), run_id);
    assert_eq!(work.workflow_type, "TypeA");

    harness
        .worker_service
        .complete_workflow(make_request(CompleteWorkflowRequest {
            run_id: run_id.clone(),
            output: Some(Payload {
                data: json_bytes(&serde_json::json!({"ok": true})),
                metadata: Default::default(),
            }),
        }))
        .await
        .expect("complete_workflow should succeed");

    let workflow = harness
        .workflow_service
        .get_workflow(make_request(GetWorkflowRequest {
            run_id: run_id.clone(),
            namespace_id: NAMESPACE.to_string(),
        }))
        .await
        .expect("get_workflow_run should succeed")
        .into_inner()
        .workflow
        .expect("workflow should exist");

    assert_eq!(workflow.status, WorkflowStatus::Completed as i32);
}

#[tokio::test]
async fn poll_requires_registered_online_worker() {
    let harness = TestHarness::new().await;
    let run_id = start_workflow(&harness, "TypeA", serde_json::json!({})).await;

    let fake_worker = Uuid::now_v7().to_string();
    let result = harness
        .worker_service
        .poll_task(make_request(poll_request(&fake_worker, &["TypeA"])))
        .await;

    assert!(result.is_err(), "unregistered worker should be rejected");
    assert_eq!(result.unwrap_err().code(), Code::FailedPrecondition);

    let worker_id = register_worker(&harness, vec!["TypeA"]).await;
    let activity = harness
        .worker_service
        .poll_task(make_request(poll_request(&worker_id, &["TypeA"])))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(activity.run_id, run_id);
}

#[tokio::test]
async fn poll_filters_by_supported_workflow_types() {
    let harness = TestHarness::new().await;
    let run_a = start_workflow(&harness, "TypeA", serde_json::json!({})).await;
    let _run_b = start_workflow(&harness, "TypeB", serde_json::json!({})).await;

    let worker_id = register_worker(&harness, vec!["TypeA"]).await;

    let activity = harness
        .worker_service
        .poll_task(make_request(poll_request(&worker_id, &["TypeA"])))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(activity.run_id, run_a);
    assert_eq!(activity.workflow_type, "TypeA");

    let empty = harness
        .worker_service
        .poll_task(make_request(poll_request(&worker_id, &["TypeA"])))
        .await
        .unwrap()
        .into_inner();

    assert!(
        empty.run_id.is_empty(),
        "no compatible work should be handed out"
    );
}

#[tokio::test]
async fn poll_rejects_queue_or_namespace_mismatch() {
    let harness = TestHarness::new().await;
    let worker_id = register_worker(&harness, vec!["TypeA"]).await;

    // Wrong queue
    let mut wrong_queue = poll_request(&worker_id, &["TypeA"]);
    wrong_queue.task_queue = "other-queue".to_string();

    let err = harness
        .worker_service
        .poll_task(make_request(wrong_queue))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::FailedPrecondition);

    // Wrong namespace
    let mut wrong_ns = poll_request(&worker_id, &["TypeA"]);
    wrong_ns.namespace_id = "other-ns".to_string();

    let err = harness
        .worker_service
        .poll_task(make_request(wrong_ns))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::FailedPrecondition);
}

#[tokio::test]
async fn poll_rejects_unregistered_workflow_types() {
    let harness = TestHarness::new().await;
    let _run_a = start_workflow(&harness, "TypeA", serde_json::json!({})).await;
    let _run_b = start_workflow(&harness, "TypeB", serde_json::json!({})).await;

    let worker_id = register_worker(&harness, vec!["TypeA"]).await;

    let err = harness
        .worker_service
        .poll_task(make_request(poll_request(&worker_id, &["TypeB"])))
        .await
        .unwrap_err();

    assert_eq!(err.code(), Code::FailedPrecondition);
}

#[tokio::test]
async fn worker_drain_blocks_new_work() {
    let harness = TestHarness::new().await;
    let worker_id = register_worker(&harness, vec!["TypeA"]).await;

    harness
        .worker_service
        .deregister(make_request(DeregisterRequest {
            worker_id: worker_id.clone(),
            drain: true,
        }))
        .await
        .expect("start drain should succeed");

    let result = harness
        .worker_service
        .poll_task(make_request(poll_request(&worker_id, &["TypeA"])))
        .await;

    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code(), Code::FailedPrecondition);
}

#[tokio::test]
async fn error_details_include_codes() {
    let harness = TestHarness::new().await;

    let status = harness
        .workflow_service
        .get_workflow(make_request(GetWorkflowRequest {
            run_id: "not-a-uuid".to_string(),
            namespace_id: NAMESPACE.to_string(),
        }))
        .await
        .unwrap_err();

    let detail = ErrorDetail::decode(status.details()).expect("error detail should decode");
    assert_eq!(detail.code, ErrorCode::InvalidArgument as i32);
    assert!(detail.non_retryable);
}

#[tokio::test]
async fn worker_heartbeat_accepts_and_reports_drain_flag() {
    let harness = TestHarness::new().await;
    let worker_id = register_worker(&harness, vec!["TypeA"]).await;

    let resp = harness
        .worker_service
        .heartbeat(make_request(HeartbeatRequest {
            worker_id: worker_id.clone(),
            active_count: 2,
            completed_delta: 1,
            failed_delta: 0,
        }))
        .await
        .expect("heartbeat should succeed")
        .into_inner();

    assert!(resp.accepted);
    assert!(!resp.should_drain);
}

#[tokio::test]
async fn list_workers_returns_registry() {
    let harness = TestHarness::new().await;
    let worker_a = register_worker(&harness, vec!["TypeA"]).await;
    let worker_b = register_worker(&harness, vec!["TypeB"]).await;

    let resp = harness
        .admin_service
        .list_workers(make_request(ListWorkersRequest {
            namespace_id: NAMESPACE.to_string(),
            task_queue: None,
            status_filter: None,
            page: Some(PageRequest {
                page_size: 10,
                page_token: String::new(),
                include_total_count: false,
            }),
        }))
        .await
        .expect("list_workers should succeed")
        .into_inner();

    assert!(resp.workers.len() >= 2);
    let ids: Vec<_> = resp.workers.iter().map(|w| w.worker_id.as_str()).collect();
    assert!(ids.contains(&worker_a.as_str()));
    assert!(ids.contains(&worker_b.as_str()));
    assert!(
        resp.workers
            .iter()
            .all(|w| w.status == WorkerStatus::Online as i32)
    );
}

#[tokio::test]
async fn workflow_input_over_limit_is_rejected() {
    let harness = TestHarness::new().await;
    let too_large = vec![0u8; 3 * 1024 * 1024]; // 3MB > 2MB limit

    let result = harness
        .workflow_service
        .start_workflow(make_request(StartWorkflowRequest {
            external_id: format!("TypeA-{}", Uuid::now_v7()),
            task_queue: TASK_QUEUE.to_string(),
            workflow_type: "TypeA".to_string(),
            input: Some(Payload {
                data: too_large,
                metadata: Default::default(),
            }),
            namespace_id: NAMESPACE.to_string(),
            context: None,
            deadline_at: None,
            version: "1.0".to_string(),
            retry_policy: None,
        }))
        .await;

    assert!(result.is_err(), "expected payload size rejection");
    assert_eq!(result.unwrap_err().code(), Code::InvalidArgument);
}

#[tokio::test]
async fn workflow_output_over_limit_is_rejected() {
    let harness = TestHarness::new().await;
    let worker_id = register_worker(&harness, vec!["TypeA"]).await;
    let run_id = start_workflow(&harness, "TypeA", serde_json::json!({})).await;

    let work = harness
        .worker_service
        .poll_task(make_request(poll_request(&worker_id, &["TypeA"])))
        .await
        .expect("poll_task should succeed")
        .into_inner();

    assert_eq!(work.run_id, run_id);

    let too_large = vec![0u8; 3 * 1024 * 1024]; // 3MB > 2MB limit
    let result = harness
        .worker_service
        .complete_workflow(make_request(CompleteWorkflowRequest {
            run_id: run_id.clone(),
            output: Some(Payload {
                data: too_large,
                metadata: Default::default(),
            }),
        }))
        .await;

    assert!(result.is_err(), "expected payload size rejection");
    assert_eq!(result.unwrap_err().code(), Code::InvalidArgument);
}

#[tokio::test]
async fn claim_workflow_batch_returns_requested_amount() {
    let harness = TestHarness::new().await;
    let store = PgStore::new(harness.pool.clone(), kagzi_store::StoreConfig::default());

    for _ in 0..3 {
        let _ = start_workflow(&harness, "TypeA", serde_json::json!({})).await;
    }

    let claimed = store
        .workflows()
        .claim_workflow_batch(TASK_QUEUE, NAMESPACE, "worker-batch", &[], 3, 30)
        .await
        .expect("batch claim should succeed");

    assert_eq!(claimed.len(), 3);
}

#[tokio::test]
async fn wait_for_new_work_notifies_on_insert() {
    let harness = TestHarness::new().await;
    let store = PgStore::new(harness.pool.clone(), kagzi_store::StoreConfig::default());

    let waiter_store = store.clone();
    let notified = tokio::spawn(async move {
        waiter_store
            .workflows()
            .wait_for_new_work(TASK_QUEUE, NAMESPACE, Duration::from_secs(2))
            .await
            .expect("wait_for_new_work should complete")
    });

    tokio::time::sleep(Duration::from_millis(500)).await;
    let _ = start_workflow(&harness, "TypeA", serde_json::json!({})).await;

    assert!(notified.await.expect("join waiter"), "should be notified");
}
