//! Simple example showing basic workflow execution
//!
//! Run with: cargo run --example simple

use kagzi::{Kagzi, WorkflowContext};
use serde::{Deserialize, Serialize};
use std::env;

#[derive(Serialize, Deserialize)]
struct SimpleInput {
    message: String,
}

async fn simple_workflow(ctx: WorkflowContext, input: SimpleInput) -> anyhow::Result<String> {
    println!("Starting workflow with message: {}", input.message);

    let result: String = ctx
        .step("process", async {
            println!("Processing...");
            Ok::<_, anyhow::Error>(format!("Processed: {}", input.message))
        })
        .await?;

    println!("Workflow completed!");
    Ok(result)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("info,kagzi=debug,sqlx=warn")
        .init();

    let database_url = env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://kagzi:kagzi_dev_password@localhost:5432/kagzi".to_string());

    let kagzi = Kagzi::connect(&database_url).await?;

    kagzi.register_workflow("simple", simple_workflow).await;

    println!("Starting workflow...");
    let handle = kagzi
        .start_workflow(
            "simple",
            SimpleInput {
                message: "Hello Kagzi!".to_string(),
            },
        )
        .await?;

    println!("Workflow started: {}", handle.run_id());

    // Start worker in same process for this simple example
    let worker = kagzi.create_worker();

    // Spawn worker in background
    tokio::spawn(async move {
        worker.start().await.expect("Worker failed");
    });

    // Wait for result
    println!("Waiting for result...");
    let result = handle.result().await?;
    println!("Result: {}", result);

    Ok(())
}
