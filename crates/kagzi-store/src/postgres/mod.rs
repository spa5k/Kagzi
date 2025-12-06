mod payload;
mod schedule;
mod step;
mod worker;
mod workflow;

use sqlx::PgPool;

pub use payload::PgPayloadRepository;
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

    pub fn payloads(&self) -> PgPayloadRepository {
        PgPayloadRepository::new(self.pool.clone())
    }

    /// Returns a new `PgWorkerRepository` backed by a clone of this store's connection pool.
    ///
    /// # Returns
    ///
    /// A `PgWorkerRepository` connected to the same pool as this `PgStore`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use sqlx::PgPool;
    /// // assume `pool` is created elsewhere
    /// let pool: PgPool = /* created pool */ unimplemented!();
    /// let store = crate::postgres::PgStore::new(pool);
    /// let repo = store.workers();
    /// // `repo` is a `PgWorkerRepository` ready to perform worker-related operations
    /// ```
    pub fn workers(&self) -> PgWorkerRepository {
        PgWorkerRepository::new(self.pool.clone())
    }

    /// Create a schedule repository backed by this store's connection pool.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let store: PgStore = /* previously constructed store */ unimplemented!();
    /// let schedules_repo = store.schedules();
    /// // use `schedules_repo` to interact with schedules in the database
    /// ```
    pub fn schedules(&self) -> PgScheduleRepository {
        PgScheduleRepository::new(self.pool.clone())
    }

    /// Accesses the underlying PostgreSQL connection pool.
    ///
    /// # Returns
    ///
    /// A reference to the internal `PgPool` used for database operations.
    ///
    /// # Examples
    ///
    /// ```
    /// let store = PgStore::new(pool);
    /// let p: &PgPool = store.pool();
    /// ```
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}