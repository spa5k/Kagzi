mod helpers;
mod queue;
mod state;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use super::StoreConfig;
use crate::error::StoreError;
use crate::models::{
    ClaimedWorkflow, CreateWorkflow, ListWorkflowsParams, PaginatedResult, RetryPolicy,
    WorkflowCursor, WorkflowExistsResult, WorkflowRun,
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
        sqlx::query!(
            r#"
            UPDATE kagzi.workflow_runs
            SET status = 'FAILED',
                error = $2,
                finished_at = NOW(),
                locked_by = NULL,
                available_at = NULL,
                attempts = attempts + 1
            WHERE run_id = $1 AND status = 'RUNNING'
            "#,
            run_id,
            error
        )
        .execute(&self.pool)
        .await?;

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
        visibility_timeout_secs: i64,
    ) -> Result<Option<ClaimedWorkflow>, StoreError> {
        queue::poll_workflow(
            self,
            namespace_id,
            task_queue,
            worker_id,
            types,
            visibility_timeout_secs,
        )
        .await
    }

    async fn create_batch(&self, params: Vec<CreateWorkflow>) -> Result<Vec<Uuid>, StoreError> {
        state::create_batch(self, params).await
    }

    async fn find_due_schedules(
        &self,
        namespace_id: &str,
        now: DateTime<Utc>,
        limit: i64,
    ) -> Result<Vec<WorkflowRun>, StoreError> {
        state::find_due_schedules(self, namespace_id, now, limit).await
    }

    async fn create_schedule_instance(
        &self,
        template_run_id: Uuid,
        fire_at: DateTime<Utc>,
    ) -> Result<Option<Uuid>, StoreError> {
        state::create_schedule_instance(self, template_run_id, fire_at).await
    }

    async fn update_next_fire(
        &self,
        run_id: Uuid,
        next_fire_at: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        state::update_next_fire(self, run_id, next_fire_at).await
    }

    async fn update(&self, run_id: Uuid, workflow: WorkflowRun) -> Result<(), StoreError> {
        state::update(self, run_id, workflow).await
    }

    async fn delete(&self, run_id: Uuid) -> Result<(), StoreError> {
        state::delete(self, run_id).await
    }
}
