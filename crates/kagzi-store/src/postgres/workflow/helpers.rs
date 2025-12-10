use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::error::StoreError;
use crate::models::WorkflowRun;

#[derive(sqlx::FromRow)]
pub(super) struct WorkflowRunRow {
    pub run_id: Uuid,
    pub namespace_id: String,
    pub external_id: String,
    pub task_queue: String,
    pub workflow_type: String,
    pub status: String,
    pub input: Vec<u8>,
    pub output: Option<Vec<u8>>,
    pub context: Option<serde_json::Value>,
    pub locked_by: Option<String>,
    pub attempts: i32,
    pub error: Option<String>,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    pub started_at: Option<chrono::DateTime<chrono::Utc>>,
    pub finished_at: Option<chrono::DateTime<chrono::Utc>>,
    pub wake_up_at: Option<chrono::DateTime<chrono::Utc>>,
    pub deadline_at: Option<chrono::DateTime<chrono::Utc>>,
    pub version: Option<String>,
    pub parent_step_attempt_id: Option<String>,
    pub retry_policy: Option<serde_json::Value>,
}

impl WorkflowRunRow {
    pub(super) fn into_model(self) -> Result<WorkflowRun, StoreError> {
        let status = self.status.parse().map_err(|_| {
            StoreError::invalid_state(format!("invalid workflow status: {}", self.status))
        })?;

        Ok(WorkflowRun {
            run_id: self.run_id,
            namespace_id: self.namespace_id,
            external_id: self.external_id,
            task_queue: self.task_queue,
            workflow_type: self.workflow_type,
            status,
            input: self.input,
            output: self.output,
            context: self.context,
            locked_by: self.locked_by,
            attempts: self.attempts,
            error: self.error,
            created_at: self.created_at,
            started_at: self.started_at,
            finished_at: self.finished_at,
            wake_up_at: self.wake_up_at,
            deadline_at: self.deadline_at,
            version: self.version,
            parent_step_attempt_id: self.parent_step_attempt_id,
            retry_policy: self.retry_policy.and_then(|v| {
                serde_json::from_value(v)
                    .map_err(|e| {
                        tracing::warn!(
                            run_id = %self.run_id,
                            error = %e,
                            "Failed to deserialize retry_policy; defaulting to None"
                        );
                    })
                    .ok()
            }),
        })
    }
}

#[derive(sqlx::FromRow)]
pub(super) struct ClaimedRow {
    pub run_id: Uuid,
    pub workflow_type: String,
    pub input: Vec<u8>,
    pub locked_by: Option<String>,
}

pub(super) async fn decrement_counter_tx(
    tx: &mut Transaction<'_, Postgres>,
    namespace_id: &str,
    task_queue: &str,
    workflow_type: &str,
) -> Result<(), StoreError> {
    sqlx::query!(
        r#"
        UPDATE kagzi.queue_counters
        SET active_count = GREATEST(active_count - 1, 0)
        WHERE namespace_id = $1 AND task_queue = $2 AND workflow_type = $3
        "#,
        namespace_id,
        task_queue,
        workflow_type
    )
    .execute(tx.as_mut())
    .await?;

    Ok(())
}

pub(super) async fn decrement_counters_tx(
    tx: &mut Transaction<'_, Postgres>,
    namespace_id: &str,
    task_queue: &str,
    workflow_type: &str,
) -> Result<(), StoreError> {
    decrement_counter_tx(tx, namespace_id, task_queue, "").await?;
    decrement_counter_tx(tx, namespace_id, task_queue, workflow_type).await
}

pub(super) async fn set_failed_tx(
    tx: &mut Transaction<'_, Postgres>,
    run_id: Uuid,
    error: &str,
) -> Result<Option<(String, String, String)>, StoreError> {
    let row = sqlx::query!(
        r#"
        UPDATE kagzi.workflow_runs
        SET status = 'FAILED',
            error = $2,
            finished_at = NOW(),
            locked_by = NULL,
            locked_until = NULL
        WHERE run_id = $1 AND status = 'RUNNING'
        RETURNING namespace_id, task_queue, workflow_type
        "#,
        run_id,
        error
    )
    .fetch_optional(tx.as_mut())
    .await?;

    Ok(row.map(|r| (r.namespace_id, r.task_queue, r.workflow_type)))
}
