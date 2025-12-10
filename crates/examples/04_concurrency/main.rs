use kagzi::{Worker, WorkflowContext};
use serde::{Deserialize, Serialize};
use std::{env, time::Duration};
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
        "local" => local_cap(server, queue).await?,
        "queue" => queue_cap(server, queue).await?,
        "workflow" => workflow_type_cap(server, queue).await?,
        _ => {
            eprintln!(
                "Usage: cargo run -p kagzi --example 04_concurrency -- [local|queue|workflow]"
            );
            std::process::exit(1);
        }
    }

    Ok(())
}

async fn local_cap(server: String, queue: String) -> anyhow::Result<()> {
    let mut worker = Worker::builder(&server, &queue)
        .queue_concurrency_limit(2) // only 2 running at a time
        .default_step_retry(common::default_retry())
        .build()
        .await?;
    worker.register(
        "short_task",
        |_: WorkflowContext, input: Input| async move {
            tracing::info!(id = input.id, "Starting short task");
            sleep(Duration::from_secs(1)).await;
            tracing::info!(id = input.id, "Finished short task");
            Ok(serde_json::json!({"id": input.id}))
        },
    );

    let mut client = common::connect_client(&server).await?;
    for id in 0..10 {
        let run = client.workflow("short_task", &queue, Input { id }).await?;
        tracing::info!(%run, id, "queued short task");
    }

    tracing::info!("Only 2 tasks should run concurrently on this worker");
    worker.run().await
}

async fn queue_cap(server: String, queue: String) -> anyhow::Result<()> {
    // Demonstrates shared queue limit across multiple workers (start this in two processes)
    let mut worker = Worker::builder(&server, &queue)
        .queue_concurrency_limit(5)
        .default_step_retry(common::default_retry())
        .build()
        .await?;
    worker.register(
        "shared_task",
        |_: WorkflowContext, input: Input| async move {
            tracing::info!(id = input.id, "running shared task");
            sleep(Duration::from_secs(2)).await;
            Ok(serde_json::json!({"id": input.id}))
        },
    );

    tracing::info!(
        "Start multiple workers with this binary; total active tasks across them stays <=5"
    );
    worker.run().await
}

async fn workflow_type_cap(server: String, queue: String) -> anyhow::Result<()> {
    let mut worker = Worker::builder(&server, &queue)
        .workflow_type_concurrency("VideoEncode", 1)
        .workflow_type_concurrency("SendEmail", 8)
        .build()
        .await?;

    worker.register(
        "VideoEncode",
        |_: WorkflowContext, input: Input| async move {
            tracing::info!(id = input.id, "encoding video");
            sleep(Duration::from_secs(5)).await;
            tracing::info!(id = input.id, "finished encode");
            Ok(serde_json::json!({"status": "encoded", "id": input.id}))
        },
    );

    worker.register("SendEmail", |_: WorkflowContext, input: Input| async move {
        tracing::info!(id = input.id, "sending email");
        sleep(Duration::from_millis(500)).await;
        Ok(serde_json::json!({"status": "sent", "id": input.id}))
    });

    let mut client = common::connect_client(&server).await?;
    for id in 0..4 {
        client.workflow("VideoEncode", &queue, Input { id }).await?;
    }
    for id in 100..110 {
        client.workflow("SendEmail", &queue, Input { id }).await?;
    }

    tracing::info!("Only one VideoEncode runs at once; emails can fan out");
    worker.run().await
}
