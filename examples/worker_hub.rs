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
    let server = env::var("KAGZI_SERVER_URL").unwrap_or_else(|_| "http://localhost:50051".into());
    let queue = env::var("KAGZI_TASK_QUEUE").unwrap_or_else(|_| "default".into());

    let mut worker = common::build_worker(&server, &queue).await?;
    worker.register("hello_workflow", hello_workflow);
    worker.register("sleep_workflow", sleep_workflow);
    worker.register("long_poll_workflow", long_poll_workflow);
    // Matches the sleep demo from 03_scheduling to allow resuming those runs.
    worker.register("sleep_demo", sleep_demo);

    println!(
        "Worker hub starting; waiting for incoming runs: server={}, queue={}",
        server, queue
    );
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
    println!("Sleeping workflow started: seconds={}", input.seconds);
    ctx.sleep(Duration::from_secs(input.seconds)).await?;
    println!("Sleeping workflow resumed");
    Ok(serde_json::json!({ "slept_for": input.seconds }))
}

async fn sleep_demo(
    mut ctx: WorkflowContext,
    input: SleepDemoInput,
) -> anyhow::Result<serde_json::Value> {
    println!("Sleep demo started: step={}", input.step);
    ctx.sleep(Duration::from_secs(15)).await?;
    println!("Sleep demo resumed");
    Ok(serde_json::json!({ "status": "resumed", "step": input.step }))
}

async fn long_poll_workflow(
    mut ctx: WorkflowContext,
    _input: serde_json::Value,
) -> anyhow::Result<String> {
    // Simulates a worker that periodically checks for external completion.
    for attempt in 1..=5 {
        println!("Polling external job...: attempt={}", attempt);
        ctx.sleep(Duration::from_secs(2)).await?;
    }
    Ok("external job finished".to_string())
}
