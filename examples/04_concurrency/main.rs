use std::env;
use std::time::Duration;

use kagzi::{Context, Kagzi, Worker};
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
    let namespace = env::var("KAGZI_NAMESPACE").unwrap_or_else(|_| "concurrency".into());

    match variant {
        "local" => local_concurrency(server, namespace).await?,
        "multi" => multi_worker_demo(server, namespace).await?,
        _ => {
            eprintln!("Usage: cargo run -p examples --example 04_concurrency -- [local|multi]");
            std::process::exit(1);
        }
    }

    Ok(())
}

async fn local_concurrency(server: String, namespace: String) -> anyhow::Result<()> {
    println!("⚡ Local Concurrency Example - demonstrates max concurrent workflow limit\n");

    let mut worker = Worker::new(&server)
        .namespace(&namespace)
        .max_concurrent(2) // Only 2 workflows run at a time on THIS worker
        .retry(common::default_retry())
        .workflows([("short_task", |_: Context, input: Input| async move {
            println!("▶️  Starting short task id={}", input.id);
            sleep(Duration::from_secs(2)).await;
            println!("✅ Finished short task id={}", input.id);
            Ok(serde_json::json!({"id": input.id}))
        })])
        .build()
        .await?;

    println!("👷 Worker started with max_concurrent=2");

    let client = Kagzi::connect(&server).await?;
    for id in 0..10 {
        let run = client
            .start("short_task")
            .namespace(&namespace)
            .input(Input { id })
            .send()
            .await?;
        println!("📝 Queued short task run={} id={}", run.id, id);
    }

    println!("⏱️  Watch the logs: only 2 tasks run at a time (2s each, 10 tasks = ~10s total)\n");
    worker.run().await
}

async fn multi_worker_demo(server: String, namespace: String) -> anyhow::Result<()> {
    let worker_id = std::process::id();

    let mut worker = Worker::new(&server)
        .namespace(&namespace)
        .max_concurrent(3) // Each worker handles up to 3 workflows
        .retry(common::default_retry())
        .workflows([(
            "parallel_task",
            move |_: Context, input: Input| async move {
                println!(
                    "👷 Worker {} ▶️  processing task id={}",
                    worker_id, input.id
                );
                sleep(Duration::from_secs(3)).await;
                println!("👷 Worker {} ✅ task complete id={}", worker_id, input.id);
                Ok(serde_json::json!({"worker": worker_id, "id": input.id}))
            },
        )])
        .build()
        .await?;

    // Only create tasks from the first worker (by convention, use arg)
    if std::env::args().any(|a| a == "--create") {
        let client = Kagzi::connect(&server).await?;
        println!("📝 Creating 20 tasks...");
        for id in 0..20 {
            client
                .start("parallel_task")
                .namespace(&namespace)
                .input(Input { id })
                .send()
                .await?;
        }
        println!(
            "✅ Tasks created. Start additional workers with: cargo run -p examples --example 04_concurrency -- multi"
        );
    }

    println!(
        "🚀 Worker {} started with max_concurrent=3. Run multiple instances to scale throughput.\n",
        worker_id
    );
    worker.run().await
}
