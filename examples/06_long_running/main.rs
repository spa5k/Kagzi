use std::env;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use kagzi::WorkflowContext;
use serde::{Deserialize, Serialize};
use tokio::time::sleep;

#[path = "../common.rs"]
mod common;

#[derive(Debug, Serialize, Deserialize)]
struct JobInput {
    job_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct JobStatus {
    state: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = env::args().collect();
    let variant = args.get(1).map(|s| s.as_str()).unwrap_or("poll");

    let server = env::var("KAGZI_SERVER_URL").unwrap_or_else(|_| "http://localhost:50051".into());
    let queue = env::var("KAGZI_TASK_QUEUE").unwrap_or_else(|_| "long-running".into());

    match variant {
        "poll" => run_polling(&server, &queue).await?,
        "timeout" => run_timeout(&server, &queue).await?,
        _ => {
            eprintln!("Usage: cargo run -p kagzi --example 06_long_running -- [poll|timeout]");
            std::process::exit(1);
        }
    }

    Ok(())
}

async fn run_polling(server: &str, queue: &str) -> anyhow::Result<()> {
    FAST_COUNTER.store(0, Ordering::SeqCst);
    let mut worker = common::build_worker(server, queue).await?;
    worker.register("poll_job", polling_workflow);

    let mut client = common::connect_client(server).await?;
    let run = client
        .workflow(
            "poll_job",
            queue,
            JobInput {
                job_id: "job-42".into(),
            },
        )
        .await?;

    println!("Started polling workflow: {}", run);
    tokio::spawn(async move { worker.run().await });
    tokio::time::sleep(Duration::from_secs(20)).await;
    Ok(())
}

async fn run_timeout(server: &str, queue: &str) -> anyhow::Result<()> {
    FAST_COUNTER.store(0, Ordering::SeqCst);
    let mut worker = common::build_worker(server, queue).await?;
    worker.register("poll_with_timeout", timeout_workflow);

    let mut client = common::connect_client(server).await?;
    let run = client
        .workflow(
            "poll_with_timeout",
            queue,
            JobInput {
                job_id: "job-timeout".into(),
            },
        )
        .await?;

    println!(
        "Started timeout workflow (expected to fail after limit): {}",
        run
    );
    tokio::spawn(async move { worker.run().await });
    tokio::time::sleep(Duration::from_secs(20)).await;
    Ok(())
}

async fn polling_workflow(mut ctx: WorkflowContext, input: JobInput) -> anyhow::Result<JobStatus> {
    // Simulate external job creation
    println!("Trigger external job: job_id={}", input.job_id);

    let mut attempts = 0;
    loop {
        attempts += 1;
        let step_name = format!("check-status-{attempts}");
        let status = ctx
            .run(&step_name, check_status(input.job_id.clone(), false))
            .await?;

        if status.state == "complete" {
            println!("Job finished: job_id={}", input.job_id);
            return Ok(status);
        }

        println!("Job pending -> sleep 5s: job_id={}", input.job_id);
        ctx.sleep(Duration::from_secs(5)).await?;
    }
}

async fn timeout_workflow(mut ctx: WorkflowContext, input: JobInput) -> anyhow::Result<JobStatus> {
    let mut attempts = 0;
    loop {
        attempts += 1;
        let step_name = format!("check-status-timeout-{attempts}");
        let status = ctx
            .run(&step_name, check_status(input.job_id.clone(), true))
            .await?;

        if status.state == "complete" {
            return Ok(status);
        }

        if attempts >= 3 {
            anyhow::bail!("job {} did not complete within 3 polls", input.job_id);
        }

        ctx.sleep(Duration::from_secs(3)).await?;
    }
}

static FAST_COUNTER: AtomicUsize = AtomicUsize::new(0);

async fn check_status(job_id: String, slow: bool) -> anyhow::Result<JobStatus> {
    if slow {
        // Simulate a job that never completes quickly
        println!("job still processing: job_id={}", job_id);
        sleep(Duration::from_secs(2)).await;
        Ok(JobStatus {
            state: "pending".into(),
        })
    } else {
        let count = FAST_COUNTER.fetch_add(1, Ordering::SeqCst) + 1;
        if count >= 2 {
            return Ok(JobStatus {
                state: "complete".into(),
            });
        }
        sleep(Duration::from_secs(2)).await;
        Ok(JobStatus {
            state: "pending".into(),
        })
    }
}
