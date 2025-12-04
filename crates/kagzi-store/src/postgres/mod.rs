mod step;
mod workflow;

use sqlx::PgPool;

pub use step::PgStepRepository;
pub use workflow::PgWorkflowRepository;

#[derive(Clone)]
pub struct PgStore {
    pool: PgPool,
}

impl PgStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn workflows(&self) -> PgWorkflowRepository {
        PgWorkflowRepository::new(self.pool.clone())
    }

    pub fn steps(&self) -> PgStepRepository {
        PgStepRepository::new(self.pool.clone())
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}
