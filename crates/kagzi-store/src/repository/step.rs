use async_trait::async_trait;
use uuid::Uuid;

use crate::error::StoreError;
use crate::models::{
    BeginStepParams, BeginStepResult, FailStepParams, FailStepResult, RetryTriggered,
    StepRetryInfo, StepRun,
};

#[async_trait]
pub trait StepRepository: Send + Sync {
    async fn find_by_id(&self, attempt_id: Uuid) -> Result<Option<StepRun>, StoreError>;

    async fn find_latest(&self, run_id: Uuid, step_id: &str)
    -> Result<Option<StepRun>, StoreError>;

    async fn list_by_workflow(
        &self,
        run_id: Uuid,
        step_id: Option<&str>,
        limit: i32,
    ) -> Result<Vec<StepRun>, StoreError>;

    async fn get_retry_info(
        &self,
        run_id: Uuid,
        step_id: &str,
    ) -> Result<Option<StepRetryInfo>, StoreError>;

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
