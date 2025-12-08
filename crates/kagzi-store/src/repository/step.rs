use async_trait::async_trait;
use uuid::Uuid;

use crate::error::StoreError;
use crate::models::{
    BeginStepParams, BeginStepResult, FailStepParams, FailStepResult, ListStepsParams,
    RetryTriggered, StepCursor, StepRun,
};
use crate::postgres::PaginatedResult;

#[async_trait]
pub trait StepRepository: Send + Sync {
    /// Find step by attempt ID. Step UUIDs are globally unique.
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
        output: serde_json::Value,
    ) -> Result<(), StoreError>;

    async fn fail(&self, params: FailStepParams) -> Result<FailStepResult, StoreError>;

    async fn process_pending_retries(&self) -> Result<Vec<RetryTriggered>, StoreError>;
}
