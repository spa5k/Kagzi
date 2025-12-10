mod health;
mod step;
mod worker;
mod workflow;
mod workflow_schedule;

use sqlx::PgPool;

pub use health::PgHealthRepository;
pub use step::PgStepRepository;
pub use worker::PgWorkerRepository;
pub use workflow::PgWorkflowRepository;
pub use workflow_schedule::PgScheduleRepository;

#[derive(Clone, Debug)]
pub struct StoreConfig {
    pub payload_warn_threshold_bytes: usize,
    pub payload_max_size_bytes: usize,
}

impl Default for StoreConfig {
    fn default() -> Self {
        Self {
            payload_warn_threshold_bytes: 1024 * 1024,
            payload_max_size_bytes: 2 * 1024 * 1024,
        }
    }
}

#[derive(Clone)]
pub struct PgStore {
    pool: PgPool,
    config: StoreConfig,
}

impl PgStore {
    pub fn new(pool: PgPool, config: StoreConfig) -> Self {
        Self { pool, config }
    }

    pub fn workflows(&self) -> PgWorkflowRepository {
        PgWorkflowRepository::new(self.pool.clone(), self.config.clone())
    }

    pub fn steps(&self) -> PgStepRepository {
        PgStepRepository::new(self.pool.clone(), self.config.clone())
    }

    pub fn workers(&self) -> PgWorkerRepository {
        PgWorkerRepository::new(self.pool.clone())
    }

    pub fn schedules(&self) -> PgScheduleRepository {
        PgScheduleRepository::new(self.pool.clone(), self.config.clone())
    }

    pub fn health(&self) -> PgHealthRepository {
        PgHealthRepository::new(self.pool.clone())
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}
