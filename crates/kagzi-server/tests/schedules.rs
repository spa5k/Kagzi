mod common;

use common::{TestHarness, json_bytes, make_request};
use kagzi_proto::kagzi::workflow_service_server::WorkflowService;
use kagzi_proto::kagzi::{
    CreateScheduleRequest, GetScheduleRequest, ListSchedulesRequest, UpdateScheduleRequest,
};
use tonic::Code;

const NAMESPACE: &str = "test-ns";
const TASK_QUEUE: &str = "default";

#[tokio::test]
async fn create_get_and_list_schedule() {
    let harness = TestHarness::new().await;
    let cron_expr = "0 */5 * * * *";

    let created = harness
        .service
        .create_schedule(make_request(CreateScheduleRequest {
            namespace_id: NAMESPACE.to_string(),
            task_queue: TASK_QUEUE.to_string(),
            workflow_type: "TypeA".to_string(),
            cron_expr: cron_expr.to_string(),
            input: json_bytes(&serde_json::json!({"hello": "world"})),
            context: vec![],
            enabled: Some(true),
            max_catchup: 5,
            version: "v1".to_string(),
        }))
        .await
        .expect("create_schedule should succeed")
        .into_inner()
        .schedule
        .expect("schedule should be present");

    assert_eq!(created.cron_expr, cron_expr);
    assert!(created.next_fire_at.is_some());

    let fetched = harness
        .service
        .get_schedule(make_request(GetScheduleRequest {
            schedule_id: created.schedule_id.clone(),
            namespace_id: NAMESPACE.to_string(),
        }))
        .await
        .expect("get_schedule should succeed")
        .into_inner()
        .schedule
        .expect("schedule should be present");

    assert_eq!(fetched.schedule_id, created.schedule_id);
    assert_eq!(fetched.workflow_type, "TypeA");

    let listed = harness
        .service
        .list_schedules(make_request(ListSchedulesRequest {
            namespace_id: NAMESPACE.to_string(),
            task_queue: TASK_QUEUE.to_string(),
            limit: 10,
        }))
        .await
        .expect("list_schedules should succeed")
        .into_inner()
        .schedules;

    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].schedule_id, created.schedule_id);
}

#[tokio::test]
async fn update_schedule_cron_recomputes_next_fire() {
    let harness = TestHarness::new().await;

    let created = harness
        .service
        .create_schedule(make_request(CreateScheduleRequest {
            namespace_id: NAMESPACE.to_string(),
            task_queue: TASK_QUEUE.to_string(),
            workflow_type: "TypeA".to_string(),
            cron_expr: "0 */10 * * * *".to_string(),
            input: json_bytes(&serde_json::json!({})),
            context: vec![],
            enabled: Some(true),
            max_catchup: 5,
            version: "v1".to_string(),
        }))
        .await
        .expect("create_schedule should succeed")
        .into_inner()
        .schedule
        .expect("schedule should be present");

    let updated = harness
        .service
        .update_schedule(make_request(UpdateScheduleRequest {
            schedule_id: created.schedule_id.clone(),
            namespace_id: NAMESPACE.to_string(),
            task_queue: None,
            workflow_type: None,
            cron_expr: Some("0 */1 * * * *".to_string()),
            input: None,
            context: None,
            enabled: None,
            max_catchup: None,
            next_fire_at: None,
            version: None,
        }))
        .await
        .expect("update_schedule should succeed")
        .into_inner()
        .schedule
        .expect("schedule should be present");

    assert_eq!(updated.cron_expr, "*/1 * * * *");
    assert!(updated.next_fire_at.is_some());
    assert_ne!(
        updated.next_fire_at.unwrap().seconds,
        created.next_fire_at.unwrap().seconds
    );
}

#[tokio::test]
async fn create_schedule_rejects_bad_cron() {
    let harness = TestHarness::new().await;
    let err = harness
        .service
        .create_schedule(make_request(CreateScheduleRequest {
            namespace_id: NAMESPACE.to_string(),
            task_queue: TASK_QUEUE.to_string(),
            workflow_type: "TypeA".to_string(),
            cron_expr: "invalid cron".to_string(),
            input: json_bytes(&serde_json::json!({})),
            context: vec![],
            enabled: Some(true),
            max_catchup: 5,
            version: "v1".to_string(),
        }))
        .await
        .unwrap_err();

    assert_eq!(err.code(), Code::InvalidArgument);
}
