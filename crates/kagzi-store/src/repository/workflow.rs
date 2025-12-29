use async_trait::async_trait;
use uuid::Uuid;

use crate::error::StoreError;
use crate::models::{
    ClaimedWorkflow, CreateWorkflow, ListWorkflowsParams, OrphanedWorkflow, PaginatedResult,
    RetryPolicy, WokenWorkflow, WorkflowCursor, WorkflowExistsResult, WorkflowRun,
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
    ) -> Result<Option<Uuid>, StoreError>;

    async fn list(
        &self,
        params: ListWorkflowsParams,
    ) -> Result<PaginatedResult<WorkflowRun, WorkflowCursor>, StoreError>;

    async fn count(
        &self,
        namespace_id: &str,
        filter_status: Option<&str>,
    ) -> Result<i64, StoreError>;

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

    /// Poll for a single workflow using SKIP LOCKED.
    /// Handles PENDING work, waking SLEEPING workflows, and stealing expired locks (crash recovery).
    async fn poll_workflow(
        &self,
        namespace_id: &str,
        task_queue: &str,
        worker_id: &str,
        types: &[String],
        lock_secs: i64,
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

    async fn wake_sleeping(&self, batch_size: i32) -> Result<Vec<WokenWorkflow>, StoreError>;

    async fn find_orphaned(&self) -> Result<Vec<OrphanedWorkflow>, StoreError>;

    async fn find_and_recover_offline_worker_workflows(&self) -> Result<u64, StoreError>;

    async fn schedule_retry(&self, run_id: Uuid, delay_ms: u64) -> Result<(), StoreError>;

    async fn mark_exhausted(&self, run_id: Uuid, error: &str) -> Result<(), StoreError>;

    async fn mark_exhausted_with_increment(
        &self,
        run_id: Uuid,
        error: &str,
    ) -> Result<(), StoreError>;

    async fn get_retry_policy(&self, run_id: Uuid) -> Result<Option<RetryPolicy>, StoreError>;
}
