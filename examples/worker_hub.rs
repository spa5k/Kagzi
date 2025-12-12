use std::env;
use std::time::Duration;

use kagzi::WorkflowContext;
use serde::{Deserialize, Serialize};

#[path = "common.rs"]
mod common;

#[derive(Debug, Serialize, Deserialize)]
struct HelloInput {
    name: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct SleepInput {
    seconds: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct SleepDemoInput {
    step: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    common::init_tracing()?;
    let server = env::var("KAGZI_SERVER_URL").unwrap_or_else(|_| "http://localhost:50051".into());
    let queue = env::var("KAGZI_TASK_QUEUE").unwrap_or_else(|_| "default".into());

    let mut worker = common::build_worker(&server, &queue).await?;
    worker.register("hello_workflow", hello_workflow);
    worker.register("sleep_workflow", sleep_workflow);
    worker.register("long_poll_workflow", long_poll_workflow);
    // Matches the sleep demo from 03_scheduling to allow resuming those runs.
    worker.register("sleep_demo", sleep_demo);

    tracing::info!(%server, %queue, "Worker hub starting; waiting for incoming runs");
    worker.run().await?;
    Ok(())
}

async fn hello_workflow(_ctx: WorkflowContext, input: HelloInput) -> anyhow::Result<String> {
    Ok(format!("Hello, {}!", input.name))
}

async fn sleep_workflow(
    mut ctx: WorkflowContext,
    input: SleepInput,
) -> anyhow::Result<serde_json::Value> {
    tracing::info!(seconds = input.seconds, "Sleeping workflow started");
    ctx.sleep(Duration::from_secs(input.seconds)).await?;
    tracing::info!("Sleeping workflow resumed");
    Ok(serde_json::json!({ "slept_for": input.seconds }))
}

async fn sleep_demo(
    mut ctx: WorkflowContext,
    input: SleepDemoInput,
) -> anyhow::Result<serde_json::Value> {
    tracing::info!(step = %input.step, "Sleep demo started");
    ctx.sleep(Duration::from_secs(15)).await?;
    tracing::info!("Sleep demo resumed");
    Ok(serde_json::json!({ "status": "resumed", "step": input.step }))
}

async fn long_poll_workflow(
    mut ctx: WorkflowContext,
    _input: serde_json::Value,
) -> anyhow::Result<String> {
    // Simulates a worker that periodically checks for external completion.
    for attempt in 1..=5 {
        tracing::info!(attempt, "Polling external job...");
        ctx.sleep(Duration::from_secs(2)).await?;
    }
    Ok("external job finished".to_string())
}
