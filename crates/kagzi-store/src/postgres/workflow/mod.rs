mod helpers;
mod notify;
mod queue;
mod state;

use async_trait::async_trait;
use sqlx::PgPool;
use std::time::Duration;
use uuid::Uuid;

use crate::error::StoreError;
use crate::models::{
    ClaimedWorkflow, CreateWorkflow, ListWorkflowsParams, OrphanedWorkflow, PaginatedResult,
    RetryPolicy, WorkCandidate, WorkflowCursor, WorkflowExistsResult, WorkflowRun,
};
use crate::repository::WorkflowRepository;

#[derive(Clone)]
pub struct PgWorkflowRepository {
    pub(super) pool: PgPool,
}

impl PgWorkflowRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
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
        idempotency_suffix: Option<&str>,
    ) -> Result<Option<Uuid>, StoreError> {
        state::find_active_by_external_id(self, namespace_id, external_id, idempotency_suffix).await
    }

    async fn list(
        &self,
        params: ListWorkflowsParams,
    ) -> Result<PaginatedResult<WorkflowRun, WorkflowCursor>, StoreError> {
        state::list(self, params).await
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

    async fn get_retry_policy(&self, run_id: Uuid) -> Result<Option<RetryPolicy>, StoreError> {
        state::get_retry_policy(self, run_id).await
    }

    async fn claim_workflow_batch(
        &self,
        task_queue: &str,
        namespace_id: &str,
        worker_id: &str,
        supported_types: &[String],
        limit: usize,
        lock_duration_secs: i64,
    ) -> Result<Vec<ClaimedWorkflow>, StoreError> {
        queue::claim_workflow_batch(
            self,
            task_queue,
            namespace_id,
            worker_id,
            supported_types,
            limit,
            lock_duration_secs,
        )
        .await
    }

    async fn list_available_workflows(
        &self,
        task_queue: &str,
        namespace_id: &str,
        supported_types: &[String],
        limit: i32,
    ) -> Result<Vec<WorkCandidate>, StoreError> {
        queue::list_available_workflows(self, task_queue, namespace_id, supported_types, limit)
            .await
    }

    async fn claim_specific_workflow(
        &self,
        run_id: Uuid,
        worker_id: &str,
        lock_duration_secs: i64,
    ) -> Result<Option<ClaimedWorkflow>, StoreError> {
        queue::claim_specific_workflow(self, run_id, worker_id, lock_duration_secs).await
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

    async fn increment_queue_counter(
        &self,
        namespace_id: &str,
        task_queue: &str,
        workflow_type: &str,
        max: i32,
    ) -> Result<bool, StoreError> {
        queue::increment_queue_counter(self, namespace_id, task_queue, workflow_type, max).await
    }

    async fn decrement_queue_counter(
        &self,
        namespace_id: &str,
        task_queue: &str,
        workflow_type: &str,
    ) -> Result<(), StoreError> {
        queue::decrement_queue_counter(self, namespace_id, task_queue, workflow_type).await
    }

    async fn reconcile_queue_counters(&self) -> Result<u64, StoreError> {
        queue::reconcile_queue_counters(self).await
    }

    async fn wait_for_new_work(
        &self,
        task_queue: &str,
        namespace_id: &str,
        timeout: Duration,
    ) -> Result<bool, StoreError> {
        notify::wait_for_new_work(&self.pool, task_queue, namespace_id, timeout).await
    }
}
