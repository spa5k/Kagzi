mod helpers;
mod queue;
mod state;

use async_trait::async_trait;
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
}
