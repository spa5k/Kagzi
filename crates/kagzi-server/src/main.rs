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
use tracing::info;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let settings = Settings::new().map_err(|e| {
        tracing::error!(error = ?e, "Failed to load configuration");
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
            tracing::error!(error = ?e, "Failed to connect to database");
            e
        })?;

    run_migrations(&db_pool).await?;

    // Print welcome banner
    print_welcome_banner(&settings);

    // Create the store
    let store_config = kagzi_store::StoreConfig {
        payload_warn_threshold_bytes: settings.payload.warn_threshold_bytes,
        payload_max_size_bytes: settings.payload.max_size_bytes,
    };
    let store = PgStore::new(db_pool, store_config);

    // Print startup stats (workers, schedules, pending jobs)
    print_startup_stats(&store).await;

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
    let queue_listener_handle = tokio::spawn(async move {
        if let Err(e) = queue_listener.start(queue_listener_token).await {
            tracing::error!(error = ?e, "Queue listener failed");
            tracing::warn!(
                "Server is running in degraded mode - queue notifications will not work"
            );
        }
    });

    // Start the coordinator (replaces scheduler + watchdog)
    let coordinator_store = store.clone();
    let coordinator_queue = queue.clone();
    let coordinator_token = shutdown_token.child_token();
    let coordinator_handle = tokio::spawn(async move {
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
    let status_reporter_handle = tokio::spawn(async move {
        status_reporter(status_store, status_token).await;
    });

    let addr = format!("{}:{}", settings.server.host, settings.server.port).parse()?;
    let workflow_service = WorkflowServiceImpl::new(store.clone(), queue.clone());
    let workflow_schedule_service =
        WorkflowScheduleServiceImpl::new(store.clone(), settings.coordinator.default_max_catchup);
    let admin_service = AdminServiceImpl::new(store.clone());
    let worker_service = WorkerServiceImpl::new(store, worker_settings, queue_settings, queue);

    // Start the server
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

    // Wait for shutdown signal (Ctrl+C or SIGTERM)
    let ctrl_c = tokio::signal::ctrl_c();

    #[cfg(unix)]
    let terminate = async {
        let sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate());
        match sigterm {
            Ok(mut handler) => {
                handler.recv().await;
            }
            Err(e) => {
                tracing::error!(error = ?e, "Failed to install SIGTERM handler, proceeding without graceful SIGTERM support");
            }
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        res = server => res?,
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    // Graceful shutdown sequence
    info!("Received shutdown signal, initiating graceful shutdown");
    shutdown_token.cancel();

    // Wait for background tasks with timeout
    let shutdown_timeout = Duration::from_secs(10);
    info!(
        "Waiting for background tasks ({}s timeout)...",
        shutdown_timeout.as_secs()
    );

    let shutdown_result = tokio::time::timeout(shutdown_timeout, async {
        let _ = tokio::join!(
            coordinator_handle,
            queue_listener_handle,
            status_reporter_handle,
        );
    })
    .await;

    match shutdown_result {
        Ok(_) => info!("All tasks stopped gracefully"),
        Err(_) => info!("Shutdown timeout reached, forcing exit"),
    }

    info!("Kagzi server stopped");

    Ok(())
}

async fn run_migrations(
    pool: &sqlx::Pool<sqlx::Postgres>,
) -> Result<(), Box<dyn std::error::Error>> {
    sqlx::migrate!("../../migrations")
        .run(pool)
        .await
        .map_err(|e| {
            tracing::error!(error = ?e, "Failed to run migrations");
            e
        })?;

    Ok(())
}

fn print_welcome_banner(settings: &Settings) {
    let version = env!("CARGO_PKG_VERSION");
    println!(
        r#"
  ██╗  ██╗ █████╗  ██████╗ ███████╗██╗
  ██║ ██╔╝██╔══██╗██╔════╝ ╚══███╔╝██║
  █████╔╝ ███████║██║  ███╗  ███╔╝ ██║
  ██╔═██╗ ██╔══██║██║   ██║ ███╔╝  ██║
  ██║  ██╗██║  ██║╚██████╔╝███████╗██║
  ╚═╝  ╚═╝╚═╝  ╚═╝ ╚═════╝ ╚══════╝╚═╝
"#
    );

    tracing::info!(
        target: "kagzi::startup",
        version = %version,
        host = %settings.server.host,
        port = %settings.server.port,
        db_connections = %settings.server.db_max_connections,
        coordinator_tick_secs = %settings.coordinator.interval_secs,
        visibility_timeout_secs = %settings.worker.visibility_timeout_secs,
        heartbeat_interval_secs = %settings.worker.heartbeat_interval_secs,
        worker_stale_secs = %settings.coordinator.worker_stale_threshold_secs,
        "Kagzi server started"
    );
}

async fn print_startup_stats(store: &PgStore) {
    // Query current state
    let workers = store.workers().count("*", None, None).await.unwrap_or(0);
    let pending = store
        .workflows()
        .count("*", Some("PENDING"))
        .await
        .unwrap_or(0);
    let running = store
        .workflows()
        .count("*", Some("RUNNING"))
        .await
        .unwrap_or(0);
    let schedules = store
        .workflows()
        .count("*", Some("SCHEDULED"))
        .await
        .unwrap_or(0);

    info!("Current State");
    info!("   - Workers online:    {}", workers);
    info!("   - Active schedules:  {}", schedules);
    info!("   - Pending workflows: {}", pending);
    info!("   - Running workflows: {}", running);

    if workers == 0 {
        info!("No workers connected. Start a worker to process jobs.");
    } else {
        info!("Ready to process workflows!");
    }
}

async fn status_reporter(store: PgStore, shutdown: CancellationToken) {
    let mut interval = tokio::time::interval(Duration::from_secs(30));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut no_worker_reminder_count = 0u32;

    loop {
        tokio::select! {
            _ = shutdown.cancelled() => {
                break;
            }
            _ = interval.tick() => {
                // Count workers across all namespaces
                let total_workers = store
                    .workers()
                    .count("*", None, None)
                    .await
                    .unwrap_or(0);

                if total_workers == 0 {
                    no_worker_reminder_count += 1;
                    // Remind every 2 minutes (4 x 30s ticks)
                    if no_worker_reminder_count % 4 == 1 {
                        info!("Awaiting workers... (no workers connected)");
                    }
                    continue;
                }

                no_worker_reminder_count = 0;

                // Get namespaces for detailed stats
                let namespaces = match store.workers().list_distinct_namespaces().await {
                    Ok(ns) => ns,
                    Err(_) => continue,
                };

                let mut total_pending = 0i64;
                let mut total_running = 0i64;
                let mut total_sleeping = 0i64;
                let mut total_completed = 0i64;
                let mut total_schedules = 0i64;

                for namespace in &namespaces {
                    if let Ok(count) = store.workflows().count(namespace, Some("PENDING")).await {
                        total_pending += count;
                    }
                    if let Ok(count) = store.workflows().count(namespace, Some("RUNNING")).await {
                        total_running += count;
                    }
                    if let Ok(count) = store.workflows().count(namespace, Some("SLEEPING")).await {
                        total_sleeping += count;
                    }
                    if let Ok(count) = store.workflows().count(namespace, Some("COMPLETED")).await {
                        total_completed += count;
                    }
                    if let Ok(count) = store.workflows().count(namespace, Some("SCHEDULED")).await {
                        total_schedules += count;
                    }
                }

                let now = chrono::Utc::now().format("%H:%M:%S");
                info!(
                    "Status @ {} | Workers: {} | Schedules: {} | Workflows: [{} pending, {} running, {} sleeping] | Completed: {}",
                    now, total_workers, total_schedules, total_pending, total_running, total_sleeping, total_completed
                );
            }
        }
    }
}
