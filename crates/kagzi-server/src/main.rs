use kagzi_proto::kagzi::workflow_service_server::WorkflowServiceServer;
use sqlx::postgres::PgPoolOptions;
use std::env;
use tonic::transport::Server;
use tracing::info;

mod reaper;
mod service;
mod tracing_utils;
use service::MyWorkflowService;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_utils::init_tracing("kagzi-server")?;

    // 2. Setup Database
    let database_url = env::var("DATABASE_URL").map_err(|_| {
        tracing::error!("DATABASE_URL environment variable not set");
        "DATABASE_URL must be set"
    })?;

    info!("Connecting to database...");
    let pool = PgPoolOptions::new()
        .max_connections(50)
        .connect(&database_url)
        .await
        .map_err(|e| {
            tracing::error!("Failed to connect to database: {:?}", e);
            e
        })?;

    info!("Connected to database successfully.");

    // 3. Run Migrations
    info!("Running migrations...");
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .map_err(|e| {
            tracing::error!("Failed to run migrations: {:?}", e);
            e
        })?;
    info!("Migrations applied successfully.");

    // 4. Start Background Reaper
    let reaper_pool = pool.clone();
    tokio::spawn(async move {
        reaper::run_reaper(reaper_pool).await;
    });

    // 5. Setup Server
    let addr = "0.0.0.0:50051".parse().map_err(|e| {
        tracing::error!("Failed to parse server address: {:?}", e);
        e
    })?;
    let service = MyWorkflowService { pool };

    info!("Kagzi Server listening on {}", addr);

    // Setup reflection service
    let reflection_service = tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(kagzi_proto::FILE_DESCRIPTOR_SET)
        .build_v1()
        .map_err(|e| {
            tracing::error!("Failed to build reflection service: {:?}", e);
            e
        })?;

    Server::builder()
        .add_service(WorkflowServiceServer::new(service))
        .add_service(reflection_service)
        .serve(addr)
        .await?;

    Ok(())
}
