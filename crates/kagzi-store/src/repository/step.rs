use async_trait::async_trait;
use uuid::Uuid;

use crate::error::StoreError;
use crate::models::{
    BeginStepParams, BeginStepResult, FailStepParams, FailStepResult, ListStepsParams,
    PaginatedResult, StepCursor, StepRun,
};

/// Repository trait for step persistence operations.
///
/// Steps are individual units of work within a workflow. When a step fails:
/// - If retryable, returns `schedule_workflow_retry_ms` to reschedule the workflow
/// - The workflow replays from the beginning, with completed steps returning cached results
/// - Failed step is re-executed on replay
#[async_trait]
pub trait StepRepository: Send + Sync {
    async fn find_by_id(&self, attempt_id: Uuid) -> Result<Option<StepRun>, StoreError>;

    async fn list(
        &self,
        params: ListStepsParams,
    ) -> Result<PaginatedResult<StepRun, StepCursor>, StoreError>;

    async fn begin(&self, params: BeginStepParams) -> Result<BeginStepResult, StoreError>;

    async fn complete(
        &self,
        run_id: Uuid,
        step_id: &str,
        output: Vec<u8>,
    ) -> Result<(), StoreError>;

    /// Fail a step and optionally schedule a workflow retry.
    ///
    /// Returns `FailStepResult` with:
    /// - `scheduled_retry: true` if the workflow should be rescheduled for replay
    /// - `schedule_workflow_retry_ms` containing the backoff delay
    async fn fail(&self, params: FailStepParams) -> Result<FailStepResult, StoreError>;

    /// Complete all pending sleep steps for a workflow.
    /// Called when a workflow wakes up from sleep to mark sleep steps as completed.
    /// Returns the number of steps completed.
    async fn complete_pending_sleeps(&self, run_id: Uuid) -> Result<u64, StoreError>;
}
