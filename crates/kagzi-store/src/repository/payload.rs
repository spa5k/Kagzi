use async_trait::async_trait;
use uuid::Uuid;

use crate::error::StoreError;
use crate::models::WorkflowPayload;

#[async_trait]
pub trait PayloadRepository: Send + Sync {
    /// Get the payload for a workflow
    async fn get(&self, run_id: Uuid) -> Result<Option<WorkflowPayload>, StoreError>;

    /// Create or update the payload for a workflow
    async fn upsert(&self, payload: WorkflowPayload) -> Result<(), StoreError>;

    /// Update just the output of a workflow payload
    async fn set_output(&self, run_id: Uuid, output: serde_json::Value) -> Result<(), StoreError>;
}
