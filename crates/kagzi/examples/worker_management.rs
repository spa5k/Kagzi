//! Example: Worker Management and Configuration
//!
//! This example demonstrates V2 worker management features:
//! - Custom worker configuration with WorkerBuilder
//! - Graceful shutdown handling
//! - Heartbeat mechanism
//! - Concurrent workflow limiting
//! - Health monitoring
//!
//! Run with: cargo run --example worker_management

use kagzi::{Kagzi, WorkflowContext, WorkerConfig};
use serde::{Deserialize, Serialize};
use std::env;
use std::time::Duration;
use tokio::signal;
use tracing::{info, warn};

#[derive(Debug, Serialize, Deserialize)]
struct TaskInput {
    task_id: String,
    duration_secs: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct TaskOutput {
    task_id: String,
    worker_id: String,
    completed: bool,
}

/// Simple task workflow
async fn simple_task(ctx: WorkflowContext, input: TaskInput) -> anyhow::Result<TaskOutput> {
    info!("Starting task: {}", input.task_id);

    // Simulate work
    ctx.step("process-task", async {
        info!("Processing task {}...", input.task_id);
        tokio::time::sleep(Duration::from_secs(input.duration_secs)).await;
        Ok::<_, anyhow::Error>(())
    })
    .await?;

    Ok(TaskOutput {
        task_id: input.task_id,
        worker_id: ctx.workflow_run_id.to_string(),
        completed: true,
    })
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("info,kagzi=debug,sqlx=warn")
        .init();

    let database_url = env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://kagzi:kagzi_dev_password@localhost:5432/kagzi".to_string());

    let kagzi = Kagzi::connect(&database_url).await?;

    kagzi.register_workflow("simple-task", simple_task).await;

    // Start several workflows to demonstrate concurrent processing
    info!("Starting 5 workflows...");
    for i in 1..=5 {
        let input = TaskInput {
            task_id: format!("task-{}", i),
            duration_secs: 3,
        };
        let handle = kagzi.start_workflow("simple-task", input).await?;
        info!("Started workflow {}: {}", i, handle.run_id());
    }

    // Create a worker with custom configuration using WorkerBuilder
    info!("\nConfiguring worker with WorkerBuilder...");
    let worker = kagzi
        .create_worker_builder()
        .worker_id(format!("custom-worker-{}", uuid::Uuid::new_v4()))
        .max_concurrent_workflows(2) // Process only 2 workflows at a time
        .heartbeat_interval_secs(15) // Heartbeat every 15 seconds
        .poll_interval_ms(500) // Poll every 500ms
        .graceful_shutdown_timeout_secs(60) // Wait up to 60s for graceful shutdown
        .build();

    info!("Worker configuration:");
    info!("  - Max concurrent workflows: 2");
    info!("  - Heartbeat interval: 15 seconds");
    info!("  - Poll interval: 500ms");
    info!("  - Graceful shutdown timeout: 60 seconds");
    info!("\nStarting worker (press Ctrl+C to trigger graceful shutdown)...\n");

    // Spawn shutdown signal handler
    let shutdown_handle = tokio::spawn(async move {
        match signal::ctrl_c().await {
            Ok(()) => {
                warn!("\n📢 Shutdown signal received!");
                warn!("Worker will complete current workflows before stopping...");
            }
            Err(err) => {
                eprintln!("Error listening for shutdown signal: {}", err);
            }
        }
    });

    // Start the worker
    // The worker will run until all workflows are complete or Ctrl+C is pressed
    let worker_result = tokio::select! {
        result = worker.start() => {
            info!("Worker stopped normally");
            result
        }
        _ = shutdown_handle => {
            info!("Shutdown signal detected");
            Ok(())
        }
    };

    worker_result?;

    info!("\n✅ Worker management example completed!");
    info!("All workflows processed successfully");

    Ok(())
}
