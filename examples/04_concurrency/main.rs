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

async fn local_concurrency(server: String, queue: String) -> anyhow::Result<()> {
    let mut worker = Worker::builder(&server, &queue)
        .max_concurrent(2) // Only 2 workflows run at a time on THIS worker
        .default_step_retry(common::default_retry())
        .build()
        .await?;

    worker.register(
        "short_task",
        |_: WorkflowContext, input: Input| async move {
            println!("Starting short task id={}", input.id);
            sleep(Duration::from_secs(2)).await;
            println!("Finished short task id={}", input.id);
            Ok(serde_json::json!({"id": input.id}))
        },
    );

    let mut client = common::connect_client(&server).await?;
    for id in 0..10 {
        let run = client.workflow("short_task", &queue, Input { id }).await?;
        println!("Queued short task run={} id={}", run, id);
    }

    println!("Watch the logs: only 2 tasks run at a time (2s each, 10 tasks = ~10s total)");
    worker.run().await
}

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
            println!("Worker {} processing task id={}", worker_id, input.id);
            sleep(Duration::from_secs(3)).await;
            println!("Worker {} task complete id={}", worker_id, input.id);
            Ok(serde_json::json!({"worker": worker_id, "id": input.id}))
        },
    );

    // Only create tasks from the first worker (by convention, use arg)
    if std::env::args().any(|a| a == "--create") {
        let mut client = common::connect_client(&server).await?;
        println!("Creating 20 tasks...");
        for id in 0..20 {
            client
                .workflow("parallel_task", &queue, Input { id })
                .await?;
        }
        println!(
            "Tasks created. Start additional workers with: cargo run -p examples --example 04_concurrency -- multi"
        );
    }

    println!(
        "Worker {} started with max_concurrent=3. Run multiple instances to scale throughput.",
        worker_id
    );
    worker.run().await
}
