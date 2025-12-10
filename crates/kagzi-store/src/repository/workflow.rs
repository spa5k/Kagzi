use std::time::Duration;

use async_trait::async_trait;
use uuid::Uuid;

use crate::error::StoreError;
use crate::models::{
    ClaimedWorkflow, CreateWorkflow, ListWorkflowsParams, OrphanedWorkflow, PaginatedResult,
    RetryPolicy, WorkCandidate, WorkflowCursor, WorkflowExistsResult, WorkflowRun,
};

#[async_trait]
pub trait WorkflowRepository: Send + Sync {
    async fn create(&self, params: CreateWorkflow) -> Result<Uuid, StoreError>;

    async fn find_by_id(
        &self,
        run_id: Uuid,
        namespace_id: &str,
    ) -> Result<Option<WorkflowRun>, StoreError>;

    async fn find_active_by_external_id(
        &self,
        namespace_id: &str,
        external_id: &str,
        idempotency_suffix: Option<&str>,
    ) -> Result<Option<Uuid>, StoreError>;

    async fn list(
        &self,
        params: ListWorkflowsParams,
    ) -> Result<PaginatedResult<WorkflowRun, WorkflowCursor>, StoreError>;

    async fn check_exists(
        &self,
        run_id: Uuid,
        namespace_id: &str,
    ) -> Result<WorkflowExistsResult, StoreError>;

    async fn check_status(
        &self,
        run_id: Uuid,
        namespace_id: &str,
    ) -> Result<WorkflowExistsResult, StoreError>;

    async fn cancel(&self, run_id: Uuid, namespace_id: &str) -> Result<bool, StoreError>;

    async fn complete(&self, run_id: Uuid, output: Vec<u8>) -> Result<(), StoreError>;

    async fn fail(&self, run_id: Uuid, error: &str) -> Result<(), StoreError>;

    async fn schedule_sleep(&self, run_id: Uuid, duration_secs: u64) -> Result<(), StoreError>;

    /// Batch claim multiple workflows atomically.
    async fn claim_workflow_batch(
        &self,
        task_queue: &str,
        namespace_id: &str,
        worker_id: &str,
        supported_types: &[String],
        limit: usize,
        lock_duration_secs: i64,
    ) -> Result<Vec<ClaimedWorkflow>, StoreError>;

    async fn list_available_workflows(
        &self,
        task_queue: &str,
        namespace_id: &str,
        supported_types: &[String],
        limit: i32,
    ) -> Result<Vec<WorkCandidate>, StoreError>;

    async fn claim_specific_workflow(
        &self,
        run_id: Uuid,
        worker_id: &str,
        lock_duration_secs: i64,
    ) -> Result<Option<ClaimedWorkflow>, StoreError>;

    async fn extend_worker_locks(
        &self,
        worker_id: &str,
        duration_secs: i64,
    ) -> Result<u64, StoreError>;

    async fn extend_locks_for_runs(
        &self,
        run_ids: &[Uuid],
        duration_secs: i64,
    ) -> Result<u64, StoreError>;

    async fn create_batch(&self, params: Vec<CreateWorkflow>) -> Result<Vec<Uuid>, StoreError>;

    async fn wake_sleeping(&self, batch_size: i32) -> Result<u64, StoreError>;

    async fn find_orphaned(&self) -> Result<Vec<OrphanedWorkflow>, StoreError>;

    async fn schedule_retry(&self, run_id: Uuid, delay_ms: u64) -> Result<(), StoreError>;

    async fn mark_exhausted(&self, run_id: Uuid, error: &str) -> Result<(), StoreError>;

    async fn get_retry_policy(&self, run_id: Uuid) -> Result<Option<RetryPolicy>, StoreError>;

    async fn increment_queue_counter(
        &self,
        namespace_id: &str,
        task_queue: &str,
        workflow_type: &str,
        max: i32,
    ) -> Result<bool, StoreError>;

    async fn decrement_queue_counter(
        &self,
        namespace_id: &str,
        task_queue: &str,
        workflow_type: &str,
    ) -> Result<(), StoreError>;

    async fn reconcile_queue_counters(&self) -> Result<u64, StoreError>;

    /// Wait for new work notification (database specific implementation).
    async fn wait_for_new_work(
        &self,
        task_queue: &str,
        namespace_id: &str,
        timeout: Duration,
    ) -> Result<bool, StoreError>;
}
