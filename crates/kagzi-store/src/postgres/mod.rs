mod pagination;
mod query;
mod schedule;
mod step;
mod worker;
mod workflow;

use sqlx::PgPool;

pub use pagination::PaginatedResult;
pub use query::{FilterBuilder, columns};
pub use schedule::PgScheduleRepository;
pub use step::PgStepRepository;
pub use worker::PgWorkerRepository;
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

    pub fn workers(&self) -> PgWorkerRepository {
        PgWorkerRepository::new(self.pool.clone())
    }

    pub fn schedules(&self) -> PgScheduleRepository {
        PgScheduleRepository::new(self.pool.clone())
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}
