//! Common test utilities and PostgreSQL testcontainer setup

use kagzi_server::{
    AdminServiceImpl, WorkerServiceImpl, WorkflowScheduleServiceImpl, WorkflowServiceImpl,
};
use kagzi_store::PgStore;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use testcontainers::{ContainerAsync, runners::AsyncRunner};
use testcontainers_modules::postgres::Postgres;
/// Test harness that manages PostgreSQL container lifecycle
#[allow(dead_code)]
pub struct TestHarness {
    pub pool: PgPool,
    pub workflow_service: WorkflowServiceImpl,
    pub worker_service: WorkerServiceImpl,
    pub workflow_schedule_service: WorkflowScheduleServiceImpl,
    pub admin_service: AdminServiceImpl,
    _container: ContainerAsync<Postgres>,
}
impl TestHarness {
    /// Create a new test harness with a fresh PostgreSQL container
    pub async fn new() -> Self {
        // Start PostgreSQL container
        let container = Postgres::default()
            .with_db_name("kagzi_test")
            .with_user("postgres")
            .with_password("postgres")
            .start()
            .await
            .expect("Failed to start PostgreSQL container");

        let host_port = container
            .get_host_port_ipv4(5432)
            .await
            .expect("Failed to get PostgreSQL port");

        let database_url = format!(
            "postgres://postgres:postgres@127.0.0.1:{}/kagzi_test",
            host_port
        );

        // Create connection pool
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(&database_url)
            .await
            .expect("Failed to connect to test database");

        sqlx::migrate!("../../migrations")
            .run(&pool)
            .await
            .expect("Failed to run migrations");

        // Create the store
        let store = PgStore::new(pool.clone());
        let workflow_service = WorkflowServiceImpl::new(store.clone());
        let worker_service = WorkerServiceImpl::new(store.clone());
        let workflow_schedule_service = WorkflowScheduleServiceImpl::new(store.clone());
        let admin_service = AdminServiceImpl::new(store.clone());

        TestHarness {
            pool,
            workflow_service,
            worker_service,
            workflow_schedule_service,
            admin_service,
            _container: container,
        }
    }
    /// Clean up test data between tests (useful for test isolation if running multiple scenarios)
    #[allow(dead_code)]
    pub async fn cleanup(&self) {
        sqlx::query("TRUNCATE kagzi.workflow_runs, kagzi.step_runs CASCADE")
            .execute(&self.pool)
            .await
            .expect("Failed to cleanup test data");
    }
}

/// Helper to create a gRPC request with metadata
pub fn make_request<T>(inner: T) -> tonic::Request<T> {
    let mut request = tonic::Request::new(inner);
    request
        .metadata_mut()
        .insert("x-correlation-id", "test-correlation-id".parse().unwrap());
    request
        .metadata_mut()
        .insert("x-trace-id", "test-trace-id".parse().unwrap());
    request
}

/// Helper to create JSON bytes from a value
pub fn json_bytes<T: serde::Serialize>(value: &T) -> Vec<u8> {
    serde_json::to_vec(value).expect("Failed to serialize to JSON")
}
