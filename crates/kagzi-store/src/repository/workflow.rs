use async_trait::async_trait;
use uuid::Uuid;

use crate::error::StoreError;
use crate::models::{
    ClaimedWorkflow, CreateWorkflow, ListWorkflowsParams, OrphanedWorkflow, PaginatedWorkflows,
    WorkflowExistsResult, WorkflowRun,
};

#[async_trait]
pub trait WorkflowRepository: Send + Sync {
    async fn create(&self, params: CreateWorkflow) -> Result<Uuid, StoreError>;

    async fn find_by_id(
        &self,
        run_id: Uuid,
        namespace_id: &str,
    ) -> Result<Option<WorkflowRun>, StoreError>;

    async fn find_by_idempotency_key(
        &self,
        namespace_id: &str,
        key: &str,
    ) -> Result<Option<Uuid>, StoreError>;

    async fn list(&self, params: ListWorkflowsParams) -> Result<PaginatedWorkflows, StoreError>;

    async fn check_exists(
        &self,
        run_id: Uuid,
        namespace_id: &str,
    ) -> Result<WorkflowExistsResult, StoreError>;

    async fn cancel(&self, run_id: Uuid, namespace_id: &str) -> Result<bool, StoreError>;

    async fn complete(&self, run_id: Uuid, output: serde_json::Value) -> Result<(), StoreError>;

    async fn fail(&self, run_id: Uuid, error: &str) -> Result<(), StoreError>;

    async fn schedule_sleep(&self, run_id: Uuid, duration_secs: u64) -> Result<(), StoreError>;

    async fn claim_next(
        &self,
        task_queue: &str,
        namespace_id: &str,
        worker_id: &str,
    ) -> Result<Option<ClaimedWorkflow>, StoreError>;

    /// Claim next workflow filtered by supported types
    async fn claim_next_filtered(
        &self,
        task_queue: &str,
        namespace_id: &str,
        worker_id: &str,
        supported_types: &[String],
    ) -> Result<Option<ClaimedWorkflow>, StoreError>;

    /// Claim a batch of workflows for distribution to workers
    /// This is used by the work distributor to fetch multiple items in one query
    async fn claim_batch(
        &self,
        task_queue: &str,
        namespace_id: &str,
        limit: i32,
    ) -> Result<Vec<ClaimedWorkflow>, StoreError>;

    /// Transfer the lock from one worker to another (used by work distributor)
    async fn transfer_lock(
        &self,
        run_id: Uuid,
        from_worker_id: &str,
        to_worker_id: &str,
    ) -> Result<bool, StoreError>;

    async fn extend_lock(&self, run_id: Uuid, worker_id: &str) -> Result<bool, StoreError>;

    /// Bulk extend locks for all workflows owned by a worker
    async fn extend_locks_for_worker(
        &self,
        worker_id: &str,
        duration_secs: i64,
    ) -> Result<u64, StoreError>;

    async fn wake_sleeping(&self) -> Result<u64, StoreError>;

    async fn find_orphaned(&self) -> Result<Vec<OrphanedWorkflow>, StoreError>;

    async fn schedule_retry(&self, run_id: Uuid, delay_ms: u64) -> Result<(), StoreError>;

    async fn mark_exhausted(&self, run_id: Uuid, error: &str) -> Result<(), StoreError>;
}
