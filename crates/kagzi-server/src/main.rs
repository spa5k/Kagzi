use kagzi_proto::kagzi::admin_service_server::AdminServiceServer;
use kagzi_proto::kagzi::worker_service_server::WorkerServiceServer;
use kagzi_proto::kagzi::workflow_schedule_service_server::WorkflowScheduleServiceServer;
use kagzi_proto::kagzi::workflow_service_server::WorkflowServiceServer;
use kagzi_server::{
    AdminServiceImpl, WorkerServiceImpl, WorkflowScheduleServiceImpl, WorkflowServiceImpl,
    run_scheduler, tracing_utils, watchdog,
};
use kagzi_store::PgStore;
use sqlx::postgres::PgPoolOptions;
use std::env;
use tonic::transport::Server;
use tracing::info;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_utils::init_tracing("kagzi-server")?;

    let database_url = env::var("DATABASE_URL").map_err(|_| {
        tracing::error!("DATABASE_URL environment variable not set");
        "DATABASE_URL must be set"
    })?;

    let pool = PgPoolOptions::new()
        .max_connections(50)
        .connect(&database_url)
        .await
        .map_err(|e| {
            tracing::error!("Failed to connect to database: {:?}", e);
            e
        })?;

    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .map_err(|e| {
            tracing::error!("Failed to run migrations: {:?}", e);
            e
        })?;

    // Create the store
    let store = PgStore::new(pool);

    // Start the background watchdog
    let watchdog_store = store.clone();
    tokio::spawn(async move {
        watchdog::run(watchdog_store).await;
    });

    // Start the scheduler loop
    let scheduler_store = store.clone();
    tokio::spawn(async move {
        run_scheduler(scheduler_store).await;
    });

    let addr = "0.0.0.0:50051".parse()?;
    let workflow_service = WorkflowServiceImpl::new(store.clone());
    let workflow_schedule_service = WorkflowScheduleServiceImpl::new(store.clone());
    let admin_service = AdminServiceImpl::new(store.clone());
    let worker_service = WorkerServiceImpl::new(store);

    info!("Kagzi Server listening on {}", addr);

    let reflection_service = tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(kagzi_proto::FILE_DESCRIPTOR_SET)
        .build_v1()?;

    Server::builder()
        .add_service(WorkflowServiceServer::new(workflow_service))
        .add_service(WorkflowScheduleServiceServer::new(
            workflow_schedule_service,
        ))
        .add_service(AdminServiceServer::new(admin_service))
        .add_service(WorkerServiceServer::new(worker_service))
        .add_service(reflection_service)
        .serve(addr)
        .await?;

    Ok(())
}
