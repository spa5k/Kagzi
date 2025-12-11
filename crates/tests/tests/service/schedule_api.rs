use std::collections::HashMap;

use chrono::{Duration, Utc};
use kagzi_proto::kagzi::workflow_schedule_service_client::WorkflowScheduleServiceClient;
use kagzi_proto::kagzi::{CreateWorkflowScheduleRequest, Payload, UpdateWorkflowScheduleRequest};
use tests::common::{TestHarness, payload_bytes};
use tonic::Request;
use uuid::Uuid;

#[tokio::test]
async fn create_schedule_computes_next_fire() -> anyhow::Result<()> {
    let harness = TestHarness::new().await;
    let mut client = WorkflowScheduleServiceClient::connect(harness.server_url.clone()).await?;

    let created = client
        .create_workflow_schedule(Request::new(CreateWorkflowScheduleRequest {
            namespace_id: "default".to_string(),
            task_queue: "svc-schedule".to_string(),
            workflow_type: "TypeSched".to_string(),
            cron_expr: "*/1 * * * * *".to_string(),
            input: Some(Payload {
                data: payload_bytes(&serde_json::json!({})),
                metadata: HashMap::new(),
            }),
            context: None,
            enabled: Some(true),
            max_catchup: Some(5),
            version: Some("v1".to_string()),
        }))
        .await?
        .into_inner()
        .schedule
        .expect("schedule should exist");

    assert!(created.next_fire_at.is_some(), "next_fire_at should be set");
    let next = harness
        .db_schedule_next_fire(&Uuid::parse_str(&created.schedule_id)?)
        .await?;
    let now = Utc::now();
    assert!(
        next > now && next < now + Duration::seconds(5),
        "next_fire_at should be within 5 seconds from now"
    );
    harness.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn update_schedule_recomputes_next_fire() -> anyhow::Result<()> {
    let harness = TestHarness::new().await;
    let mut client = WorkflowScheduleServiceClient::connect(harness.server_url.clone()).await?;

    let created = client
        .create_workflow_schedule(Request::new(CreateWorkflowScheduleRequest {
            namespace_id: "default".to_string(),
            task_queue: "svc-schedule-upd".to_string(),
            workflow_type: "TypeSched".to_string(),
            cron_expr: "0 */10 * * * *".to_string(),
            input: Some(Payload {
                data: payload_bytes(&serde_json::json!({})),
                metadata: HashMap::new(),
            }),
            context: None,
            enabled: Some(true),
            max_catchup: Some(5),
            version: Some("v1".to_string()),
        }))
        .await?
        .into_inner()
        .schedule
        .expect("schedule should exist");

    let updated = client
        .update_workflow_schedule(Request::new(UpdateWorkflowScheduleRequest {
            schedule_id: created.schedule_id.clone(),
            namespace_id: "default".to_string(),
            task_queue: None,
            workflow_type: None,
            cron_expr: Some("*/1 * * * * *".to_string()),
            input: None,
            context: None,
            enabled: None,
            max_catchup: None,
            next_fire_at: None,
            version: None,
        }))
        .await?
        .into_inner()
        .schedule
        .expect("schedule should exist");

    assert!(updated.next_fire_at.is_some());
    assert!(
        created.next_fire_at.is_some(),
        "created schedule should have next_fire_at"
    );
    assert_ne!(
        updated.next_fire_at.unwrap().seconds,
        created.next_fire_at.unwrap().seconds
    );
    let db_next = harness
        .db_schedule_next_fire(&Uuid::parse_str(&updated.schedule_id)?)
        .await?;
    assert!(db_next.timestamp() > 0, "db next_fire_at should update");
    harness.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn create_schedule_rejects_invalid_cron() -> anyhow::Result<()> {
    let harness = TestHarness::new().await;
    let mut client = WorkflowScheduleServiceClient::connect(harness.server_url.clone()).await?;

    let err = client
        .create_workflow_schedule(Request::new(CreateWorkflowScheduleRequest {
            namespace_id: "default".to_string(),
            task_queue: "svc-schedule-bad".to_string(),
            workflow_type: "TypeSched".to_string(),
            cron_expr: "not a cron".to_string(),
            input: Some(Payload {
                data: payload_bytes(&serde_json::json!({})),
                metadata: HashMap::new(),
            }),
            context: None,
            enabled: Some(true),
            max_catchup: Some(5),
            version: Some("v1".to_string()),
        }))
        .await;

    assert!(err.is_err(), "invalid cron should fail");
    assert_eq!(err.unwrap_err().code(), tonic::Code::InvalidArgument);
    harness.shutdown().await?;
    Ok(())
}
