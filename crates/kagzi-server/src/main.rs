use kagzi_proto::kagzi::workflow_service_server::WorkflowServiceServer;
use sqlx::postgres::PgPoolOptions;
use std::env;
use tonic::transport::Server;
use tracing::info;

mod service;
use service::MyWorkflowService;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Setup tracing
    tracing_subscriber::fmt::init();

    // 2. Setup Database
    // Load .env file if it exists (useful for local dev)
    // We don't use dotenv crate here explicitly as we rely on the environment or the user loading it
    // But for convenience in this setup we can assume env vars are set or use dotenvy if added.
    // Since dotenv isn't in Cargo.toml, we assume the user runs with `just` which loads .env
    // or we can just expect DATABASE_URL.
    let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");

    info!("Connecting to database...");
    let pool = PgPoolOptions::new()
        .max_connections(50)
        .connect(&database_url)
        .await?;

    info!("Connected to database successfully.");

    // 3. Run Migrations
    info!("Running migrations...");
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");
    info!("Migrations applied successfully.");

    // 4. Setup Server
    let addr = "0.0.0.0:50051".parse()?;
    let service = MyWorkflowService { pool };

    info!("Kagzi Server listening on {}", addr);

    // Setup reflection service
    let reflection_service = tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(kagzi_proto::FILE_DESCRIPTOR_SET)
        .build_v1()?;

    Server::builder()
        .add_service(WorkflowServiceServer::new(service))
        .add_service(reflection_service)
        .serve(addr)
        .await?;

    Ok(())
}
