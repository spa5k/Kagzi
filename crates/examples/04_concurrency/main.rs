//! Concurrency Control Example
//!
//! Demonstrates how to limit concurrent workflow execution on a worker.
//!
//! **Architecture Note**: Concurrency is controlled per-worker via local semaphores.
//! Each worker independently limits how many workflows it processes simultaneously.
//! This is similar to Temporal's worker-side concurrency model.
//!
//! Variants:
//! - `local`: Limits total concurrent workflows on this worker (recommended)
//! - `multi`: Run multiple workers to see independent concurrency limits

use std::env;
use std::time::Duration;

use kagzi::{Worker, WorkflowContext};
use serde::{Deserialize, Serialize};
use tokio::time::sleep;

#[path = "../common.rs"]
mod common;

#[derive(Debug, Serialize, Deserialize)]
struct Input {
    id: u32,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    common::init_tracing()?;
    let args: Vec<String> = env::args().collect();
    let variant = args.get(1).map(|s| s.as_str()).unwrap_or("local");

    let server = env::var("KAGZI_SERVER_URL").unwrap_or_else(|_| "http://localhost:50051".into());
    let queue = env::var("KAGZI_TASK_QUEUE").unwrap_or_else(|_| "concurrency".into());

    match variant {
        "local" => local_concurrency(server, queue).await?,
        "multi" => multi_worker_demo(server, queue).await?,
        _ => {
            eprintln!("Usage: cargo run -p examples --example 04_concurrency -- [local|multi]");
            std::process::exit(1);
        }
    }

    Ok(())
}

/// Demonstrates per-worker concurrency limit using `max_concurrent`.
///
/// This is the primary way to control concurrency in Kagzi. Each worker
/// maintains a local semaphore that limits how many workflows it processes
/// at once.
async fn local_concurrency(server: String, queue: String) -> anyhow::Result<()> {
    let mut worker = Worker::builder(&server, &queue)
        .max_concurrent(2) // Only 2 workflows run at a time on THIS worker
        .default_step_retry(common::default_retry())
        .build()
        .await?;

    worker.register(
        "short_task",
        |_: WorkflowContext, input: Input| async move {
            tracing::info!(id = input.id, "Starting short task");
            sleep(Duration::from_secs(2)).await;
            tracing::info!(id = input.id, "Finished short task");
            Ok(serde_json::json!({"id": input.id}))
        },
    );

    let mut client = common::connect_client(&server).await?;
    for id in 0..10 {
        let run = client.workflow("short_task", &queue, Input { id }).await?;
        tracing::info!(%run, id, "queued short task");
    }

    tracing::info!("Watch the logs: only 2 tasks run at a time (2s each, 10 tasks = ~10s total)");
    worker.run().await
}

/// Demonstrates running multiple workers with independent concurrency limits.
///
/// Run this in multiple terminals to see that each worker maintains its own
/// concurrency limit. Total throughput scales with the number of workers.
async fn multi_worker_demo(server: String, queue: String) -> anyhow::Result<()> {
    let worker_id = std::process::id();

    let mut worker = Worker::builder(&server, &queue)
        .max_concurrent(3) // Each worker handles up to 3 workflows
        .default_step_retry(common::default_retry())
        .build()
        .await?;

    worker.register(
        "parallel_task",
        move |_: WorkflowContext, input: Input| async move {
            tracing::info!(worker = worker_id, id = input.id, "Processing task");
            sleep(Duration::from_secs(3)).await;
            tracing::info!(worker = worker_id, id = input.id, "Task complete");
            Ok(serde_json::json!({"worker": worker_id, "id": input.id}))
        },
    );

    // Only create tasks from the first worker (by convention, use arg)
    if std::env::args().any(|a| a == "--create") {
        let mut client = common::connect_client(&server).await?;
        tracing::info!("Creating 20 tasks...");
        for id in 0..20 {
            client
                .workflow("parallel_task", &queue, Input { id })
                .await?;
        }
        tracing::info!(
            "Tasks created. Start additional workers with: cargo run -p examples --example 04_concurrency -- multi"
        );
    }

    tracing::info!(
        worker = worker_id,
        "Worker started with max_concurrent=3. Run multiple instances to scale throughput."
    );
    worker.run().await
}
