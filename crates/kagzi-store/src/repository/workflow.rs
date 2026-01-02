use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::error::StoreError;
use crate::models::{
    ClaimedWorkflow, CreateWorkflow, ListWorkflowsParams, PaginatedResult, RetryPolicy,
    WorkflowCursor, WorkflowExistsResult, WorkflowRun,
};

/// Repository trait for workflow persistence operations.
///
/// This trait defines the contract for all workflow-related database operations.
/// The simplified model uses a single `available_at` timestamp for scheduling:
/// - New workflows: `available_at = NOW()` (immediately available)
/// - Running workflows: `available_at = NOW() + visibility_timeout` (claimed)
/// - Sleeping workflows: `available_at = NOW() + sleep_duration`
/// - Retrying workflows: `available_at = NOW() + backoff_delay`
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

    async fn schedule_retry(&self, run_id: Uuid, delay_ms: u64) -> Result<(), StoreError>;

    async fn mark_exhausted(&self, run_id: Uuid, error: &str) -> Result<(), StoreError>;

    async fn mark_exhausted_with_increment(
        &self,
        run_id: Uuid,
        error: &str,
    ) -> Result<(), StoreError>;

    async fn get_retry_policy(&self, run_id: Uuid) -> Result<Option<RetryPolicy>, StoreError>;

    /// Poll for a single workflow using FOR UPDATE SKIP LOCKED.
    ///
    /// Claims any workflow where `available_at <= NOW()`:
    /// - PENDING workflows ready for first execution
    /// - SLEEPING workflows whose sleep has elapsed
    /// - RUNNING workflows with expired visibility timeout (orphan recovery)
    ///
    /// Sets `available_at = NOW() + visibility_timeout_secs` to claim the workflow.
    async fn poll_workflow(
        &self,
        namespace_id: &str,
        task_queue: &str,
        worker_id: &str,
        types: &[String],
        visibility_timeout_secs: i64,
    ) -> Result<Option<ClaimedWorkflow>, StoreError>;

    async fn create_batch(&self, params: Vec<CreateWorkflow>) -> Result<Vec<Uuid>, StoreError>;

    async fn find_due_schedules(
        &self,
        namespace_id: &str,
        now: DateTime<Utc>,
        limit: i64,
    ) -> Result<Vec<WorkflowRun>, StoreError>;

    async fn create_schedule_instance(
        &self,
        template_run_id: Uuid,
        fire_at: DateTime<Utc>,
    ) -> Result<Option<Uuid>, StoreError>;

    async fn update_next_fire(
        &self,
        run_id: Uuid,
        next_fire_at: DateTime<Utc>,
        last_fired_at: Option<DateTime<Utc>>,
    ) -> Result<(), StoreError>;

    async fn update(&self, run_id: Uuid, workflow: WorkflowRun) -> Result<(), StoreError>;

    async fn delete(&self, run_id: Uuid) -> Result<(), StoreError>;

    /// Get the namespace for a workflow by run_id only.
    /// Used when the namespace is not known upfront but run_id is.
    async fn get_namespace(&self, run_id: Uuid) -> Result<Option<String>, StoreError>;

    /// Extend visibility timeout for workflows locked by a specific worker.
    /// Called during heartbeat to keep workflow locked while worker is healthy.
    /// Returns the number of workflows extended.
    async fn extend_visibility(
        &self,
        worker_id: &str,
        extension_secs: i64,
    ) -> Result<u64, StoreError>;
}
