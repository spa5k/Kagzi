mod helpers;
mod queue;
mod state;

use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

use super::StoreConfig;
use crate::error::StoreError;
use crate::models::{
    ClaimedWorkflow, CreateWorkflow, ListWorkflowsParams, OrphanedWorkflow, PaginatedResult,
    RetryPolicy, WorkflowCursor, WorkflowExistsResult, WorkflowRun,
};
use crate::repository::WorkflowRepository;

#[derive(Clone)]
pub struct PgWorkflowRepository {
    pub(super) pool: PgPool,
    pub(super) config: StoreConfig,
}

impl PgWorkflowRepository {
    pub fn new(pool: PgPool, config: StoreConfig) -> Self {
        Self { pool, config }
    }
}

#[async_trait]
impl WorkflowRepository for PgWorkflowRepository {
    async fn create(&self, params: CreateWorkflow) -> Result<Uuid, StoreError> {
        state::create(self, params).await
    }

    async fn find_by_id(
        &self,
        run_id: Uuid,
        namespace_id: &str,
    ) -> Result<Option<WorkflowRun>, StoreError> {
        state::find_by_id(self, run_id, namespace_id).await
    }

    async fn find_active_by_external_id(
        &self,
        namespace_id: &str,
        external_id: &str,
    ) -> Result<Option<Uuid>, StoreError> {
        state::find_active_by_external_id(self, namespace_id, external_id).await
    }

    async fn list(
        &self,
        params: ListWorkflowsParams,
    ) -> Result<PaginatedResult<WorkflowRun, WorkflowCursor>, StoreError> {
        state::list(self, params).await
    }

    async fn count(
        &self,
        namespace_id: &str,
        filter_status: Option<&str>,
    ) -> Result<i64, StoreError> {
        state::count(self, namespace_id, filter_status).await
    }

    async fn check_exists(
        &self,
        run_id: Uuid,
        namespace_id: &str,
    ) -> Result<WorkflowExistsResult, StoreError> {
        state::check_exists(self, run_id, namespace_id).await
    }

    async fn check_status(
        &self,
        run_id: Uuid,
        namespace_id: &str,
    ) -> Result<WorkflowExistsResult, StoreError> {
        state::check_status(self, run_id, namespace_id).await
    }

    async fn cancel(&self, run_id: Uuid, namespace_id: &str) -> Result<bool, StoreError> {
        state::cancel(self, run_id, namespace_id).await
    }

    async fn complete(&self, run_id: Uuid, output: Vec<u8>) -> Result<(), StoreError> {
        state::complete(self, run_id, output).await
    }

    async fn fail(&self, run_id: Uuid, error: &str) -> Result<(), StoreError> {
        state::fail(self, run_id, error).await
    }

    async fn schedule_sleep(&self, run_id: Uuid, duration_secs: u64) -> Result<(), StoreError> {
        state::schedule_sleep(self, run_id, duration_secs).await
    }

    async fn schedule_retry(&self, run_id: Uuid, delay_ms: u64) -> Result<(), StoreError> {
        state::schedule_retry(self, run_id, delay_ms).await
    }

    async fn mark_exhausted(&self, run_id: Uuid, error: &str) -> Result<(), StoreError> {
        state::mark_exhausted(self, run_id, error).await
    }

    async fn mark_exhausted_with_increment(
        &self,
        run_id: Uuid,
        error: &str,
    ) -> Result<(), StoreError> {
        let mut tx = self.pool.begin().await?;

        sqlx::query!(
            r#"
            UPDATE kagzi.workflow_runs
            SET status = 'FAILED',
                error = $2,
                finished_at = NOW(),
                locked_by = NULL,
                locked_until = NULL,
                attempts = attempts + 1
            WHERE run_id = $1 AND status = 'RUNNING'
            "#,
            run_id,
            error
        )
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok(())
    }

    async fn get_retry_policy(&self, run_id: Uuid) -> Result<Option<RetryPolicy>, StoreError> {
        state::get_retry_policy(self, run_id).await
    }

    async fn poll_workflow(
        &self,
        namespace_id: &str,
        task_queue: &str,
        worker_id: &str,
        types: &[String],
        lock_secs: i64,
    ) -> Result<Option<ClaimedWorkflow>, StoreError> {
        queue::poll_workflow(self, namespace_id, task_queue, worker_id, types, lock_secs).await
    }

    async fn extend_worker_locks(
        &self,
        worker_id: &str,
        duration_secs: i64,
    ) -> Result<u64, StoreError> {
        queue::extend_worker_locks(self, worker_id, duration_secs).await
    }

    async fn extend_locks_for_runs(
        &self,
        run_ids: &[Uuid],
        duration_secs: i64,
    ) -> Result<u64, StoreError> {
        queue::extend_locks_for_runs(self, run_ids, duration_secs).await
    }

    async fn create_batch(&self, params: Vec<CreateWorkflow>) -> Result<Vec<Uuid>, StoreError> {
        state::create_batch(self, params).await
    }

    async fn wake_sleeping(&self, batch_size: i32) -> Result<u64, StoreError> {
        queue::wake_sleeping(self, batch_size as i64).await
    }

    async fn find_orphaned(&self) -> Result<Vec<OrphanedWorkflow>, StoreError> {
        queue::find_orphaned(self).await
    }

    async fn find_and_recover_offline_worker_workflows(&self) -> Result<u64, StoreError> {
        let mut tx = self.pool.begin().await?;

        // Find workflows locked by offline workers
        let rows = sqlx::query!(
            r#"
            SELECT w.run_id, w.attempts, w.retry_policy
            FROM kagzi.workflow_runs w
            JOIN kagzi.workers wk ON w.locked_by = wk.worker_id::text
            WHERE w.status = 'RUNNING'
              AND wk.status = 'OFFLINE'
            FOR UPDATE SKIP LOCKED
            "#
        )
        .fetch_all(&mut *tx)
        .await?;

        let mut recovered = 0;
        for row in rows {
            let policy = row
                .retry_policy
                .and_then(|v| {
                    serde_json::from_value::<RetryPolicy>(v)
                        .map_err(|e| {
                            tracing::warn!(
                                run_id = %row.run_id,
                                error = %e,
                                "Failed to deserialize retry policy"
                            );
                            e
                        })
                        .ok()
                })
                .unwrap_or_default();

            if policy.should_retry(row.attempts) {
                let delay_ms = policy.calculate_delay_ms(row.attempts) as u64;
                let next_attempt = row.attempts + 1;

                // Check if the next attempt would exhaust retries
                if policy.should_retry(next_attempt) {
                    // Schedule retry with attempts increment
                    sqlx::query!(
                        r#"
                    UPDATE kagzi.workflow_runs
                    SET status = 'PENDING',
                        locked_by = NULL,
                        locked_until = NULL,
                        wake_up_at = NOW() + ($2 * INTERVAL '1 millisecond'),
                        attempts = attempts + 1
                    WHERE run_id = $1
                    "#,
                        row.run_id,
                        delay_ms as f64
                    )
                    .execute(&mut *tx)
                    .await?;

                    recovered += 1;

                    tracing::warn!(
                        run_id = %row.run_id,
                        attempts = next_attempt,
                        delay_ms = delay_ms,
                        "Recovered workflow from offline worker - scheduling retry"
                    );
                } else {
                    // Next attempt would exhaust retries, mark as failed immediately
                    sqlx::query!(
                        r#"
                    UPDATE kagzi.workflow_runs
                    SET status = 'FAILED',
                        locked_by = NULL,
                        locked_until = NULL,
                        wake_up_at = NULL,
                        attempts = attempts + 1,
                        error = 'Workflow crashed and exhausted all retry attempts (worker offline)'
                    WHERE run_id = $1
                    "#,
                        row.run_id
                    )
                    .execute(&mut *tx)
                    .await?;

                    recovered += 1;

                    tracing::error!(
                        run_id = %row.run_id,
                        attempts = next_attempt,
                        "Workflow from offline worker exhausted retries - marked as failed"
                    );
                }
            } else {
                // Mark as exhausted
                sqlx::query!(
                    r#"
                    UPDATE kagzi.workflow_runs
                    SET status = 'FAILED',
                        error = 'Workflow crashed and exhausted all retry attempts (worker offline)'
                    WHERE run_id = $1
                    "#,
                    row.run_id
                )
                .execute(&mut *tx)
                .await?;

                tracing::error!(
                    run_id = %row.run_id,
                    attempts = row.attempts,
                    "Workflow from offline worker exhausted retries - marked as failed"
                );
            }
        }

        tx.commit().await?;
        Ok(recovered)
    }
}
