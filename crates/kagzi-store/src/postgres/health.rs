use sqlx::PgPool;

use crate::StoreError;
use crate::repository::HealthRepository;

pub struct PgHealthRepository {
    pool: PgPool,
}

impl PgHealthRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl HealthRepository for PgHealthRepository {
    async fn health_check(&self) -> Result<(), StoreError> {
        sqlx::query_scalar!("SELECT 1")
            .fetch_one(&self.pool)
            .await
            .map_err(StoreError::from)?;
        Ok(())
    }
}
