//! Common test utilities for end-to-end tests using a real Kagzi server and PostgreSQL
//! running inside testcontainers.

use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, anyhow};
use chrono::{DateTime, Utc};
use kagzi::{Client, Worker};
use kagzi_proto::kagzi::workflow_service_client::WorkflowServiceClient;
use kagzi_proto::kagzi::{GetWorkflowRequest, WorkflowStatus};
use kagzi_queue::QueueNotifier;
use kagzi_server::config::{SchedulerSettings, WatchdogSettings, WorkerSettings};
use kagzi_server::{
    AdminServiceImpl, WorkerServiceImpl, WorkflowScheduleServiceImpl, WorkflowServiceImpl,
    run_scheduler, watchdog,
};
use kagzi_store::{PgStore, StoreConfig};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::postgres::Postgres;
use tokio::net::TcpListener;
use tokio_stream::wrappers::TcpListenerStream;
use tokio_util::sync::CancellationToken;
use tonic::Request;
use tonic::transport::Server;
use tracing;
use uuid::Uuid;

/// Configuration knobs for the test server to keep integration tests fast.
#[derive(Clone, Debug)]
pub struct TestConfig {
    pub scheduler_interval_secs: u64,
    pub scheduler_batch_size: i32,
    pub scheduler_max_per_tick: i32,
    pub watchdog_interval_secs: u64,
    pub worker_stale_threshold_secs: i64,
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
            poll_timeout_secs: 2,
        }
    }
}

/// End-to-end harness that runs Postgres + Kagzi server (gRPC) with scheduler and watchdog.
pub struct TestHarness {
    pub pool: PgPool,
    pub server_url: String,
    pub server_addr: SocketAddr,
    _container: ContainerAsync<Postgres>,
    server_handle: Option<tokio::task::JoinHandle<Result<(), tonic::transport::Error>>>,
    shutdown: CancellationToken,
}

impl TestHarness {
    pub async fn new() -> Self {
        Self::with_config(TestConfig::default()).await
    }

    pub async fn with_config(config: TestConfig) -> Self {
        let container = Postgres::default()
            .with_tag("18-alpine")
            .with_env_var("POSTGRES_DB", "kagzi_test")
            .with_env_var("POSTGRES_USER", "postgres")
            .with_env_var("POSTGRES_PASSWORD", "postgres")
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

        let migrations_dir = std::env::var("MIGRATIONS_DIR")
            .unwrap_or_else(|_| format!("{}/../../migrations", env!("CARGO_MANIFEST_DIR")));
        let migrator = sqlx::migrate::Migrator::new(Path::new(&migrations_dir))
            .await
            .expect("Failed to load migrations");

        migrator.run(&pool).await.expect("Failed to run migrations");

        let store = PgStore::new(pool.clone(), StoreConfig::default());

        let scheduler_settings = SchedulerSettings {
            interval_secs: config.scheduler_interval_secs,
            batch_size: config.scheduler_batch_size,
            max_workflows_per_tick: config.scheduler_max_per_tick,
        };
        let watchdog_settings = WatchdogSettings {
            interval_secs: config.watchdog_interval_secs,
            worker_stale_threshold_secs: config.worker_stale_threshold_secs,
        };
        let worker_settings = WorkerSettings {
            poll_timeout_secs: config.poll_timeout_secs,
            heartbeat_interval_secs: 1, // Fast heartbeat for tests
        };

        let shutdown = CancellationToken::new();

        let queue = kagzi_queue::PostgresNotifier::new(pool.clone(), 300, 300);
        let queue_listener = queue.clone();
        let queue_listener_token = shutdown.child_token();
        tokio::spawn(async move {
            if let Err(e) = queue_listener.start(queue_listener_token).await {
                tracing::error!("Queue listener failed to start: {:?}", e);
            }
        });

        watchdog::spawn(
            store.clone(),
            watchdog_settings.clone(),
            shutdown.child_token(),
        );

        let scheduler_store = store.clone();
        let scheduler_queue = queue.clone();
        let scheduler_token = shutdown.child_token();
        tokio::spawn(async move {
            run_scheduler(
                scheduler_store,
                scheduler_queue,
                scheduler_settings,
                scheduler_token,
            )
            .await;
        });

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("Failed to bind server socket");
        let addr = listener.local_addr().expect("Failed to read bound address");
        let incoming = TcpListenerStream::new(listener);

        // Use default queue settings for tests
        let queue_settings = kagzi_server::config::QueueSettings {
            cleanup_interval_secs: 300,
            poll_jitter_ms: 100,
            max_reconnect_secs: 300,
        };

        let workflow_service = WorkflowServiceImpl::new(store.clone(), queue.clone());
        let workflow_schedule_service = WorkflowScheduleServiceImpl::new(store.clone());
        let admin_service = AdminServiceImpl::new(store.clone());
        let worker_service = WorkerServiceImpl::new(
            store.clone(),
            worker_settings,
            queue_settings,
            queue.clone(),
        );

        let server_shutdown = shutdown.child_token();
        let server_handle = tokio::spawn(async move {
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
        });

        TestHarness {
            pool,
            server_url: format!("http://{}", addr),
            server_addr: addr,
            _container: container,
            server_handle: Some(server_handle),
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

    /// Expose the bound server address for diagnostics.
    pub fn server_addr(&self) -> SocketAddr {
        self.server_addr
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

    /// Lookup step status for a given run_id and step_id.
    pub async fn db_step_status(
        &self,
        run_id: &Uuid,
        step_id: &str,
    ) -> anyhow::Result<Option<String>> {
        sqlx::query_scalar::<_, Option<String>>(
            "SELECT status FROM kagzi.step_runs WHERE run_id = $1 AND step_id = $2 AND is_latest = true",
        )
        .bind(run_id)
        .bind(step_id)
        .fetch_optional(&self.pool)
        .await
        .map(|opt| opt.flatten())
        .context("fetch step status")
    }

    /// Count step attempts for a given run_id and step_id.
    pub async fn db_step_attempt_count(&self, run_id: &Uuid, step_id: &str) -> anyhow::Result<i64> {
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM kagzi.step_runs WHERE run_id = $1 AND step_id = $2",
        )
        .bind(run_id)
        .bind(step_id)
        .fetch_one(&self.pool)
        .await
        .context("fetch step attempt count")
    }

    /// Get workflow attempts count.
    pub async fn db_workflow_attempts(&self, run_id: &Uuid) -> anyhow::Result<i32> {
        sqlx::query_scalar::<_, i32>("SELECT attempts FROM kagzi.workflow_runs WHERE run_id = $1")
            .bind(run_id)
            .fetch_one(&self.pool)
            .await
            .context("fetch workflow attempts")
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

    /// Wait for a workflow to reach a specific DB status with retries.
    pub async fn wait_for_db_status(
        &self,
        run_id: &Uuid,
        expected: &str,
        attempts: usize,
        delay: Duration,
    ) -> anyhow::Result<()> {
        for _ in 0..attempts {
            let status = self.db_workflow_status(run_id).await?;
            if status == expected {
                return Ok(());
            }
            tokio::time::sleep(delay).await;
        }
        anyhow::bail!(
            "workflow {} did not reach status {}, last={}",
            run_id,
            expected,
            self.db_workflow_status(run_id).await?
        );
    }

    /// Wait for a step to reach a specific DB status with retries.
    pub async fn wait_for_step_status(
        &self,
        run_id: &Uuid,
        step_id: &str,
        expected: &str,
        attempts: usize,
        delay: Duration,
    ) -> anyhow::Result<()> {
        for _ in 0..attempts {
            if self.db_step_status(run_id, step_id).await? == Some(expected.to_string()) {
                return Ok(());
            }
            tokio::time::sleep(delay).await;
        }
        anyhow::bail!(
            "step {} for {} did not reach status {}, last={:?}",
            step_id,
            run_id,
            expected,
            self.db_step_status(run_id, step_id).await?
        );
    }

    /// Shutdown the harness and surface server failures.
    pub async fn shutdown(mut self) -> anyhow::Result<()> {
        self.shutdown.cancel();
        if let Some(handle) = self.server_handle.take() {
            handle
                .await
                .map_err(|err| anyhow!("server task panicked: {err}"))?
                .map_err(|err| anyhow!("server failed: {err}"))?;
        }
        Ok(())
    }
}

/// Fetch workflow via gRPC by run_id (string).
pub async fn fetch_workflow(
    server_url: &str,
    run_id: &str,
) -> anyhow::Result<kagzi_proto::kagzi::Workflow> {
    let mut client = WorkflowServiceClient::connect(server_url.to_string()).await?;
    let resp = client
        .get_workflow(Request::new(GetWorkflowRequest {
            run_id: run_id.to_string(),
            namespace_id: "default".to_string(),
        }))
        .await?;
    resp.into_inner()
        .workflow
        .ok_or_else(|| anyhow::anyhow!("workflow not found"))
}

/// Serialize a payload to bytes for gRPC requests.
pub fn payload_bytes<T: serde::Serialize>(val: &T) -> Vec<u8> {
    serde_json::to_vec(val).expect("serialize payload")
}

/// Wait for a workflow to reach a status, polling with a fixed attempt limit.
pub async fn wait_for_status(
    server_url: &str,
    run_id: &str,
    expected: WorkflowStatus,
    attempts: usize,
) -> anyhow::Result<kagzi_proto::kagzi::Workflow> {
    if attempts == 0 {
        anyhow::bail!("attempts must be greater than 0");
    }
    for attempt in 0..attempts {
        let wf = fetch_workflow(server_url, run_id).await?;
        if wf.status == expected as i32 {
            return Ok(wf);
        }
        if attempt == attempts - 1 {
            anyhow::bail!(
                "workflow {} did not reach status {:?}, last status={:?}",
                run_id,
                expected,
                wf.status
            );
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    unreachable!("wait_for_status should have returned or bailed")
}

/// Wait for a workflow type to reach a completed count.
pub async fn wait_for_completed_by_type(
    pool: &PgPool,
    workflow_type: &str,
    expected: usize,
    attempts: usize,
    delay: Duration,
) -> anyhow::Result<()> {
    for attempt in 0..attempts {
        let done: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM kagzi.workflow_runs WHERE workflow_type = $1 AND status = 'COMPLETED'",
        )
        .bind(workflow_type)
        .fetch_one(pool)
        .await?;

        if done as usize >= expected {
            return Ok(());
        }

        if attempt == attempts - 1 {
            anyhow::bail!(
                "timed out waiting for {} completed runs of {} (got {})",
                expected,
                workflow_type,
                done
            );
        }

        tokio::time::sleep(delay).await;
    }

    unreachable!()
}

impl Drop for TestHarness {
    fn drop(&mut self) {
        self.shutdown.cancel();
        if let Some(handle) = self.server_handle.take() {
            handle.abort();
        }
    }
}
