//! Common test utilities for end-to-end tests using a real Kagzi server and PostgreSQL
//! running inside testcontainers.

use std::net::SocketAddr;

use anyhow::Context;
use chrono::{DateTime, Utc};
use kagzi::{Client, Worker};
use kagzi_server::config::{SchedulerSettings, WatchdogSettings, WorkerSettings};
use kagzi_server::{
    AdminServiceImpl, WorkerServiceImpl, WorkflowScheduleServiceImpl, WorkflowServiceImpl,
    run_scheduler, watchdog,
};
use kagzi_store::{PgStore, StoreConfig};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use testcontainers::ContainerAsync;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;
use tokio::net::TcpListener;
use tokio_stream::wrappers::TcpListenerStream;
use tokio_util::sync::CancellationToken;
use tonic::transport::Server;
use uuid::Uuid;

/// Configuration knobs for the test server to keep integration tests fast.
#[derive(Clone, Debug)]
pub struct TestConfig {
    pub scheduler_interval_secs: u64,
    pub scheduler_batch_size: i32,
    pub scheduler_max_per_tick: i32,
    pub watchdog_interval_secs: u64,
    pub worker_stale_threshold_secs: i64,
    pub counter_reconcile_interval_secs: u64,
    pub wake_sleeping_batch_size: i32,
    pub poll_timeout_secs: u64,
}

impl Default for TestConfig {
    fn default() -> Self {
        Self {
            scheduler_interval_secs: 1,
            scheduler_batch_size: 100,
            scheduler_max_per_tick: 100,
            watchdog_interval_secs: 1,
            worker_stale_threshold_secs: 2,
            counter_reconcile_interval_secs: 5,
            wake_sleeping_batch_size: 50,
            poll_timeout_secs: 2,
        }
    }
}

/// End-to-end harness that runs Postgres + Kagzi server (gRPC) with scheduler and watchdog.
pub struct TestHarness {
    pub pool: PgPool,
    pub server_url: String,
    _container: ContainerAsync<Postgres>,
    shutdown: CancellationToken,
}

impl TestHarness {
    pub async fn new() -> Self {
        Self::with_config(TestConfig::default()).await
    }

    pub async fn with_config(config: TestConfig) -> Self {
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

        let pool = PgPoolOptions::new()
            .max_connections(20)
            .connect(&database_url)
            .await
            .expect("Failed to connect to test database");

        sqlx::migrate!("../../migrations")
            .run(&pool)
            .await
            .expect("Failed to run migrations");

        let store = PgStore::new(pool.clone(), StoreConfig::default());

        let scheduler_settings = SchedulerSettings {
            interval_secs: config.scheduler_interval_secs,
            batch_size: config.scheduler_batch_size,
            max_workflows_per_tick: config.scheduler_max_per_tick,
        };
        let watchdog_settings = WatchdogSettings {
            interval_secs: config.watchdog_interval_secs,
            worker_stale_threshold_secs: config.worker_stale_threshold_secs,
            counter_reconcile_interval_secs: config.counter_reconcile_interval_secs,
            wake_sleeping_batch_size: config.wake_sleeping_batch_size,
        };
        let worker_settings = WorkerSettings {
            poll_timeout_secs: config.poll_timeout_secs,
        };

        let shutdown = CancellationToken::new();

        // Spawn watchdog tasks.
        watchdog::spawn(
            store.clone(),
            watchdog_settings.clone(),
            shutdown.child_token(),
        );

        // Spawn scheduler loop.
        let scheduler_store = store.clone();
        let scheduler_token = shutdown.child_token();
        tokio::spawn(async move {
            run_scheduler(scheduler_store, scheduler_settings, scheduler_token).await;
        });

        // Start gRPC server on a random port.
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("Failed to bind server socket");
        let addr = listener.local_addr().expect("Failed to read bound address");
        let incoming = TcpListenerStream::new(listener);

        let workflow_service = WorkflowServiceImpl::new(store.clone());
        let workflow_schedule_service = WorkflowScheduleServiceImpl::new(store.clone());
        let admin_service = AdminServiceImpl::new(store.clone());
        let worker_service = WorkerServiceImpl::new(store.clone(), worker_settings);

        let server_shutdown = shutdown.child_token();
        tokio::spawn(async move {
            Server::builder()
                .add_service(kagzi_proto::kagzi::workflow_service_server::WorkflowServiceServer::new(
                    workflow_service,
                ))
                .add_service(
                    kagzi_proto::kagzi::workflow_schedule_service_server::WorkflowScheduleServiceServer::new(
                        workflow_schedule_service,
                    ),
                )
                .add_service(kagzi_proto::kagzi::admin_service_server::AdminServiceServer::new(
                    admin_service,
                ))
                .add_service(kagzi_proto::kagzi::worker_service_server::WorkerServiceServer::new(
                    worker_service,
                ))
                .serve_with_incoming_shutdown(incoming, server_shutdown.cancelled())
                .await
                .expect("Server failed");
        });

        TestHarness {
            pool,
            server_url: format!("http://{}", addr),
            _container: container,
            shutdown,
        }
    }

    /// Connected Kagzi client against the harness server.
    pub async fn client(&self) -> Client {
        Client::connect(&self.server_url)
            .await
            .expect("Failed to connect Kagzi client")
    }

    /// Build a worker configured for the harness server.
    pub async fn worker(&self, queue: &str) -> Worker {
        Worker::builder(&self.server_url, queue)
            .build()
            .await
            .expect("Failed to build worker")
    }

    /// Lookup workflow status by run_id (raw DB string value).
    pub async fn db_workflow_status(&self, run_id: &Uuid) -> anyhow::Result<String> {
        sqlx::query_scalar::<_, String>("SELECT status FROM kagzi.workflow_runs WHERE run_id = $1")
            .bind(run_id)
            .fetch_one(&self.pool)
            .await
            .context("fetch workflow status")
    }

    /// Lookup workflow locked_by value.
    pub async fn db_workflow_locked_by(&self, run_id: &Uuid) -> anyhow::Result<Option<String>> {
        sqlx::query_scalar::<_, Option<String>>(
            "SELECT locked_by FROM kagzi.workflow_runs WHERE run_id = $1",
        )
        .bind(run_id)
        .fetch_one(&self.pool)
        .await
        .context("fetch workflow locked_by")
    }

    /// Lookup worker status by worker_id.
    pub async fn db_worker_status(&self, worker_id: &Uuid) -> anyhow::Result<String> {
        sqlx::query_scalar::<_, String>("SELECT status FROM kagzi.workers WHERE worker_id = $1")
            .bind(worker_id)
            .fetch_one(&self.pool)
            .await
            .context("fetch worker status")
    }

    /// Fetch next_fire_at for a schedule.
    pub async fn db_schedule_next_fire(&self, schedule_id: &Uuid) -> anyhow::Result<DateTime<Utc>> {
        sqlx::query_scalar::<_, DateTime<Utc>>(
            "SELECT next_fire_at FROM kagzi.schedules WHERE schedule_id = $1",
        )
        .bind(schedule_id)
        .fetch_one(&self.pool)
        .await
        .context("fetch schedule next_fire_at")
    }

    /// Count workflows in a given status.
    pub async fn db_count_workflows_by_status(&self, status: &str) -> anyhow::Result<i64> {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM kagzi.workflow_runs WHERE status = $1")
            .bind(status)
            .fetch_one(&self.pool)
            .await
            .context("count workflows by status")
    }
}

impl Drop for TestHarness {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}
