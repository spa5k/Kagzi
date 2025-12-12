use std::env;
use std::time::Duration;

use kagzi::WorkflowContext;
use serde::{Deserialize, Serialize};

#[path = "../common.rs"]
mod common;

#[derive(Debug, Serialize, Deserialize)]
struct CleanupInput {
    table: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct SleepInput {
    step: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    common::init_tracing()?;
    let args: Vec<String> = env::args().collect();
    let variant = args.get(1).map(|s| s.as_str()).unwrap_or("cron");

    let server = env::var("KAGZI_SERVER_URL").unwrap_or_else(|_| "http://localhost:50051".into());
    let queue = env::var("KAGZI_TASK_QUEUE").unwrap_or_else(|_| "scheduling".into());

    match variant {
        "cron" => cron_demo(&server, &queue).await?,
        "sleep" => durable_sleep_demo(&server, &queue).await?,
        "catchup" => catchup_demo(&server, &queue).await?,
        _ => {
            eprintln!("Usage: cargo run -p kagzi --example 03_scheduling -- [cron|sleep|catchup]");
            std::process::exit(1);
        }
    }

    Ok(())
}

async fn cron_demo(server: &str, queue: &str) -> anyhow::Result<()> {
    let mut client = common::connect_client(server).await?;
    let schedule = client
        .workflow_schedule(
            "cleanup_workflow",
            queue,
            "*/1 * * * * *", // every second for demo
            CleanupInput {
                table: "sessions".into(),
            },
        )
        .version("v1")
        .max_catchup(3)
        .await?;

    tracing::info!(schedule_id = %schedule.schedule_id, "Created schedule");

    let fetched = client
        .get_workflow_schedule(&schedule.schedule_id, Some("default"))
        .await?;
    tracing::info!(?fetched, "Fetched schedule");

    client
        .delete_workflow_schedule(&schedule.schedule_id, Some("default"))
        .await?;
    tracing::info!("Deleted schedule");

    Ok(())
}

async fn durable_sleep_demo(server: &str, queue: &str) -> anyhow::Result<()> {
    let mut worker = common::build_worker(server, queue).await?;
    worker.register("sleep_demo", sleep_workflow);
    let mut client = common::connect_client(server).await?;

    let run_id = client
        .workflow(
            "sleep_demo",
            queue,
            SleepInput {
                step: "wait-and-resume".into(),
            },
        )
        .await?;

    tracing::info!(%run_id, "Started sleep demo; stop worker during sleep to see resume");
    tokio::spawn(async move { worker.run().await });
    tokio::time::sleep(Duration::from_secs(25)).await;
    Ok(())
}

async fn catchup_demo(server: &str, queue: &str) -> anyhow::Result<()> {
    let mut client = common::connect_client(server).await?;
    let schedule = client
        .workflow_schedule(
            "catchup_workflow",
            queue,
            "*/5 * * * * *", // every 5 seconds
            CleanupInput {
                table: "audit_logs".into(),
            },
        )
        .max_catchup(10)
        .enabled(true)
        .await?;

    tracing::info!(schedule_id = %schedule.schedule_id, "Created catchup schedule; pause server to see replay");
    // Instruct user to stop scheduler and restart; no automated pause here.
    Ok(())
}

async fn sleep_workflow(
    mut ctx: WorkflowContext,
    input: SleepInput,
) -> anyhow::Result<serde_json::Value> {
    tracing::info!(step = %input.step, "Step A started");
    ctx.sleep(Duration::from_secs(15)).await?;
    tracing::info!("Step B resumed after durable sleep");
    Ok(serde_json::json!({ "status": "resumed", "step": input.step }))
}
