//! Demonstrates worker failover with a long-running (sleeping) workflow.
//!
//! Steps to try:
//! 1) Start worker A: `cargo run -p kagzi --example sleep_failover -- worker`
//! 2) In another shell, start the workflow: `cargo run -p kagzi --example sleep_failover -- trigger`
//! 3) Before 15s elapse, stop worker A (Ctrl+C) to simulate a crash.
//! 4) Wait ~30s for the lock/heartbeat to expire, then start worker B with the same command as step 1.
//! 5) Observe that worker B picks up and completes the workflow (logs will show host/pid).

use std::time::Duration;

use kagzi::{Client, Worker, WorkflowContext};
use serde_json::json;
use tokio::time::sleep;
use tracing::info;

const DEFAULT_SERVER: &str = "http://localhost:50051";
const DEFAULT_QUEUE: &str = "sleep-failover";
const WORKFLOW_TYPE: &str = "Sleep15Seconds";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();

    let server = std::env::var("SERVER_URL").unwrap_or_else(|_| DEFAULT_SERVER.to_string());
    let task_queue = std::env::var("TASK_QUEUE").unwrap_or_else(|_| DEFAULT_QUEUE.to_string());

    match args.get(1).map(|s| s.as_str()) {
        Some("worker") => run_worker(&server, &task_queue).await?,
        Some("trigger") => trigger_workflow(&server, &task_queue).await?,
        _ => print_usage(&args),
    }

    Ok(())
}

fn print_usage(args: &[String]) {
    let bin = args.first().map(String::as_str).unwrap_or("sleep_failover");
    eprintln!("Usage:");
    eprintln!("  {bin} worker   # start a worker for the sleep workflow");
    eprintln!("  {bin} trigger  # start one sleep workflow run");
    eprintln!();
    eprintln!("Environment:");
    eprintln!("  SERVER_URL  (default: {DEFAULT_SERVER})");
    eprintln!("  TASK_QUEUE  (default: {DEFAULT_QUEUE})");
}

async fn run_worker(server: &str, task_queue: &str) -> anyhow::Result<()> {
    kagzi::tracing_utils::init_tracing()?;

    let mut worker = Worker::builder(server, task_queue).build().await?;
    worker.register(WORKFLOW_TYPE, sleep_workflow);

    info!(
        server = server,
        task_queue = task_queue,
        "Worker starting; stop it within 15s to simulate a crash"
    );

    worker.run().await
}

async fn trigger_workflow(server: &str, task_queue: &str) -> anyhow::Result<()> {
    kagzi::tracing_utils::init_tracing()?;

    let mut client = Client::connect(server).await?;
    let started_at = chrono::Utc::now();

    let run_id = client
        .workflow(
            WORKFLOW_TYPE,
            task_queue,
            json!({ "started_at": started_at }),
        )
        .await?;

    info!(
        run_id = %run_id,
        task_queue = task_queue,
        "Started sleep workflow; stop worker within 15s, then restart after ~30s"
    );

    Ok(())
}

async fn sleep_workflow(
    _ctx: WorkflowContext,
    input: serde_json::Value,
) -> anyhow::Result<serde_json::Value> {
    let host = hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "unknown-host".to_string());
    let pid = std::process::id();

    info!(host = %host, pid = pid, input = ?input, "Sleep workflow started");
    sleep(Duration::from_secs(15)).await;
    info!(host = %host, pid = pid, "Sleep workflow finished");

    // Returning host/pid helps confirm which worker completed it.
    Ok(json!({
        "handled_by": host,
        "pid": pid,
        "slept_seconds": 15
    }))
}
