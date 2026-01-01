use std::time::Duration;

use kagzi_proto::kagzi::admin_service_server::AdminServiceServer;
use kagzi_proto::kagzi::worker_service_server::WorkerServiceServer;
use kagzi_proto::kagzi::workflow_schedule_service_server::WorkflowScheduleServiceServer;
use kagzi_proto::kagzi::workflow_service_server::WorkflowServiceServer;
use kagzi_queue::QueueNotifier;
use kagzi_server::config::Settings;
use kagzi_server::{
    AdminServiceImpl, WorkerServiceImpl, WorkflowScheduleServiceImpl, WorkflowServiceImpl,
    coordinator,
};
use kagzi_store::{PgStore, WorkerRepository, WorkflowRepository};
use sqlx::postgres::PgPoolOptions;
use tokio_util::sync::CancellationToken;
use tonic::transport::Server;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let settings = Settings::new().map_err(|e| {
        eprintln!("Failed to load configuration: {:?}", e);
        e
    })?;

    // Initialize telemetry (tracing + OpenTelemetry)
    // Keep the guard alive for the duration of the program
    let _telemetry_guard = kagzi_server::telemetry::init_telemetry(&settings.telemetry)?;

    let db_pool = PgPoolOptions::new()
        .max_connections(settings.server.db_max_connections)
        .connect(&settings.database_url)
        .await
        .map_err(|e| {
            eprintln!("Failed to connect to database: {:?}", e);
            e
        })?;

    run_migrations(&db_pool).await?;

    // Create the store
    let store_config = kagzi_store::StoreConfig {
        payload_warn_threshold_bytes: settings.payload.warn_threshold_bytes,
        payload_max_size_bytes: settings.payload.max_size_bytes,
    };
    let store = PgStore::new(db_pool, store_config);

    let shutdown_token = CancellationToken::new();
    let coordinator_settings = settings.coordinator.clone();
    let worker_settings = settings.worker.clone();
    let queue_settings = settings.queue.clone();

    // Create the queue notifier and start background listener
    let queue = kagzi_queue::PostgresNotifier::new(
        store.pool().clone(),
        queue_settings.cleanup_interval_secs,
        queue_settings.max_reconnect_secs,
    );
    let queue_listener = queue.clone();
    let queue_listener_token = shutdown_token.child_token();
    let _queue_listener_handle = tokio::spawn(async move {
        if let Err(e) = queue_listener.start(queue_listener_token).await {
            eprintln!("Queue listener failed: {:?}", e);
            eprintln!("Server is running in degraded mode - queue notifications will not work");
        }
    });

    // Start the coordinator (replaces scheduler + watchdog)
    let coordinator_store = store.clone();
    let coordinator_queue = queue.clone();
    let coordinator_token = shutdown_token.child_token();
    tokio::spawn(async move {
        coordinator::run(
            coordinator_store,
            coordinator_queue,
            coordinator_settings,
            coordinator_token,
        )
        .await;
    });

    // Start the status reporter
    let status_store = store.clone();
    let status_token = shutdown_token.child_token();
    tokio::spawn(async move {
        status_reporter(status_store, status_token).await;
    });

    let addr = format!("{}:{}", settings.server.host, settings.server.port).parse()?;
    let workflow_service = WorkflowServiceImpl::new(store.clone(), queue.clone());
    let workflow_schedule_service = WorkflowScheduleServiceImpl::new(store.clone());
    let admin_service = AdminServiceImpl::new(store.clone());
    let worker_service = WorkerServiceImpl::new(store, worker_settings, queue_settings, queue);

    tracing::info!("Kagzi Server listening on {}", addr);

    let reflection_service = tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(kagzi_proto::FILE_DESCRIPTOR_SET)
        .build_v1()?;

    let server_token = shutdown_token.child_token();
    let server = Server::builder()
        .add_service(WorkflowServiceServer::new(workflow_service))
        .add_service(WorkflowScheduleServiceServer::new(
            workflow_schedule_service,
        ))
        .add_service(AdminServiceServer::new(admin_service))
        .add_service(WorkerServiceServer::new(worker_service))
        .add_service(reflection_service)
        .serve_with_shutdown(addr, server_token.cancelled());

    tokio::select! {
        res = server => res?,
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("Received shutdown signal");
            shutdown_token.cancel();
        }
    }

    Ok(())
}

async fn run_migrations(
    pool: &sqlx::Pool<sqlx::Postgres>,
) -> Result<(), Box<dyn std::error::Error>> {
    sqlx::migrate!("../../migrations")
        .run(pool)
        .await
        .map_err(|e| {
            eprintln!("Failed to run migrations: {:?}", e);
            e
        })?;

    Ok(())
}

async fn status_reporter(store: PgStore, shutdown: CancellationToken) {
    let mut interval = tokio::time::interval(Duration::from_secs(10));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = shutdown.cancelled() => {
                break;
            }
            _ = interval.tick() => {
                let namespaces = match store.workers().list_distinct_namespaces().await {
                    Ok(namespaces) => {
                        if namespaces.is_empty() {
                            tracing::debug!("No active workers found, skipping status report");
                            continue;
                        }
                        namespaces
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "Failed to fetch namespaces from workers, skipping status report");
                        continue;
                    }
                };

                let mut total_workers = 0;
                let mut total_pending = 0;
                let mut total_running = 0;

                for namespace in &namespaces {
                    match store.workers().count(namespace, None, None).await {
                        Ok(count) => total_workers += count,
                        Err(e) => tracing::debug!(namespace, error = %e, "Failed to count workers"),
                    }

                    match store.workflows().count(namespace, Some("PENDING")).await {
                        Ok(count) => total_pending += count,
                        Err(e) => tracing::debug!(namespace, error = %e, "Failed to count pending workflows"),
                    }
                    match store.workflows().count(namespace, Some("RUNNING")).await {
                        Ok(count) => total_running += count,
                        Err(e) => tracing::debug!(namespace, error = %e, "Failed to count running workflows"),
                    }
                }

                if total_workers > 0 || total_pending > 0 || total_running > 0 {
                    tracing::info!(
                        workers = total_workers,
                        workflows.pending = total_pending,
                        workflows.running = total_running,
                        "Server status"
                    );
                }
            }
        }
    }
}
