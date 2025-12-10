use kagzi_proto::kagzi::admin_service_server::AdminServiceServer;
use kagzi_proto::kagzi::worker_service_server::WorkerServiceServer;
use kagzi_proto::kagzi::workflow_schedule_service_server::WorkflowScheduleServiceServer;
use kagzi_proto::kagzi::workflow_service_server::WorkflowServiceServer;
use kagzi_server::config::Settings;
use kagzi_server::{
    AdminServiceImpl, WorkerServiceImpl, WorkflowScheduleServiceImpl, WorkflowServiceImpl,
    run_scheduler, tracing_utils, watchdog,
};
use kagzi_store::PgStore;
use sqlx::postgres::PgPoolOptions;
use tokio_util::sync::CancellationToken;
use tonic::transport::Server;
use tracing::info;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_utils::init_tracing("kagzi-server")?;

    let settings = Settings::new().map_err(|e| {
        tracing::error!("Failed to load configuration: {:?}", e);
        e
    })?;

    let pool = PgPoolOptions::new()
        .max_connections(settings.server.db_max_connections)
        .connect(&settings.database_url)
        .await
        .map_err(|e| {
            tracing::error!("Failed to connect to database: {:?}", e);
            e
        })?;

    run_migrations(&pool).await?;

    // Create the store
    let store_config = kagzi_store::StoreConfig {
        payload_warn_threshold_bytes: settings.payload.warn_threshold_bytes,
        payload_max_size_bytes: settings.payload.max_size_bytes,
    };
    let store = PgStore::new(pool, store_config);

    let shutdown = CancellationToken::new();
    let scheduler_settings = settings.scheduler.clone();
    let watchdog_settings = settings.watchdog.clone();
    let worker_settings = settings.worker.clone();

    // Start the background watchdog
    let watchdog_store = store.clone();
    let watchdog_token = shutdown.child_token();
    watchdog::spawn(watchdog_store, watchdog_settings, watchdog_token);

    // Start the scheduler loop
    let scheduler_store = store.clone();
    let scheduler_token = shutdown.child_token();
    tokio::spawn(async move {
        run_scheduler(scheduler_store, scheduler_settings, scheduler_token).await;
    });

    let addr = format!("{}:{}", settings.server.host, settings.server.port).parse()?;
    let workflow_service = WorkflowServiceImpl::new(store.clone());
    let workflow_schedule_service = WorkflowScheduleServiceImpl::new(store.clone());
    let admin_service = AdminServiceImpl::new(store.clone());
    let worker_service = WorkerServiceImpl::new(store, worker_settings);

    info!("Kagzi Server listening on {}", addr);

    let reflection_service = tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(kagzi_proto::FILE_DESCRIPTOR_SET)
        .build_v1()?;

    let server_token = shutdown.child_token();
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
            info!("Received shutdown signal");
            shutdown.cancel();
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
            tracing::error!("Failed to run migrations: {:?}", e);
            e
        })?;

    Ok(())
}
