use async_trait::async_trait;
use uuid::Uuid;

use crate::error::StoreError;
use crate::models::{
    ClaimedWorkflow, CreateWorkflow, ListWorkflowsParams, OrphanedWorkflow, RetryPolicy,
    WorkCandidate, WorkflowCursor, WorkflowExistsResult, WorkflowRun,
};
use crate::postgres::PaginatedResult;

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

    async fn complete(&self, run_id: Uuid, output: serde_json::Value) -> Result<(), StoreError>;

    async fn fail(&self, run_id: Uuid, error: &str) -> Result<(), StoreError>;

    async fn schedule_sleep(&self, run_id: Uuid, duration_secs: u64) -> Result<(), StoreError>;

    async fn claim_next_filtered(
        &self,
        task_queue: &str,
        namespace_id: &str,
        worker_id: &str,
        supported_types: &[String],
    ) -> Result<Option<ClaimedWorkflow>, StoreError>;

    async fn scan_available(
        &self,
        task_queue: &str,
        namespace_id: &str,
        supported_types: &[String],
        limit: i32,
    ) -> Result<Vec<WorkCandidate>, StoreError>;

    async fn claim_by_id(
        &self,
        run_id: Uuid,
        worker_id: &str,
    ) -> Result<Option<ClaimedWorkflow>, StoreError>;

    async fn extend_locks_for_worker(
        &self,
        worker_id: &str,
        duration_secs: i64,
    ) -> Result<u64, StoreError>;

    async fn extend_locks_batch(
        &self,
        run_ids: &[Uuid],
        duration_secs: i64,
    ) -> Result<u64, StoreError>;

    async fn create_batch(&self, params: Vec<CreateWorkflow>) -> Result<Vec<Uuid>, StoreError>;

    async fn wake_sleeping(&self) -> Result<u64, StoreError>;

    async fn find_orphaned(&self) -> Result<Vec<OrphanedWorkflow>, StoreError>;

    async fn schedule_retry(&self, run_id: Uuid, delay_ms: u64) -> Result<(), StoreError>;

    async fn mark_exhausted(&self, run_id: Uuid, error: &str) -> Result<(), StoreError>;

    async fn get_retry_policy(&self, run_id: Uuid) -> Result<Option<RetryPolicy>, StoreError>;

    async fn increment_counter(
        &self,
        namespace_id: &str,
        task_queue: &str,
        workflow_type: &str,
        max: i32,
    ) -> Result<bool, StoreError>;

    async fn decrement_counter(
        &self,
        namespace_id: &str,
        task_queue: &str,
        workflow_type: &str,
    ) -> Result<(), StoreError>;
}
