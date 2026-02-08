use async_trait::async_trait;

use crate::StoreError;

#[async_trait]
pub trait HealthRepository: Send + Sync {
    async fn health_check(&self) -> Result<(), StoreError>;
}
