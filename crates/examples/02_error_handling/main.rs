use kagzi::{KagziError, RetryPolicy, Worker, WorkflowContext};
use serde::{Deserialize, Serialize};
use std::env;
use std::time::Duration;

#[path = "../common.rs"]
mod common;

#[derive(Debug, Serialize, Deserialize)]
struct Input {
    label: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct Output {
    message: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    common::init_tracing()?;
    let args: Vec<String> = env::args().collect();
    let variant = args.get(1).map(|s| s.as_str()).unwrap_or("flaky");

    let server = env::var("KAGZI_SERVER_URL").unwrap_or_else(|_| "http://localhost:50051".into());
    let queue = env::var("KAGZI_TASK_QUEUE").unwrap_or_else(|_| "error-handling".into());

    match variant {
        "flaky" => run_flaky(&server, &queue).await?,
        "fatal" => run_fatal(&server, &queue).await?,
        "override" => run_override(&server, &queue).await?,
        _ => {
            eprintln!(
                "Usage: cargo run -p kagzi --example 02_error_handling -- [flaky|fatal|override]"
            );
            std::process::exit(1);
        }
    }

    Ok(())
}

async fn run_flaky(server: &str, queue: &str) -> anyhow::Result<()> {
    let mut worker = common::build_worker(server, queue).await?;
    let flaky = common::FlakyStep::succeed_after(3);
    worker.register(
        "flaky_step",
        move |mut ctx: WorkflowContext, input: Input| {
            let flaky = flaky.clone();
            async move {
                let msg = ctx
                    .run_with_input("flaky", &input, flaky.run(&input.label))
                    .await?;
                Ok(Output { message: msg })
            }
        },
    );

    let mut client = common::connect_client(server).await?;
    let run_id = client
        .workflow(
            "flaky_step",
            queue,
            Input {
                label: "sometimes fails".into(),
            },
        )
        .await?;
    tracing::info!(%run_id, "Started flaky workflow (should retry twice)");
    tokio::spawn(async move { worker.run().await });
    tokio::time::sleep(Duration::from_secs(12)).await;
    Ok(())
}

async fn run_fatal(server: &str, queue: &str) -> anyhow::Result<()> {
    let mut worker = common::build_worker(server, queue).await?;
    worker.register(
        "fatal_step",
        |_ctx: WorkflowContext, input: Input| async move {
            if input.label.contains("bad-key") {
                Err(KagziError::non_retryable("invalid api key").into())
            } else {
                Ok(Output {
                    message: format!("processed {}", input.label),
                })
            }
        },
    );

    let mut client = common::connect_client(server).await?;
    let run_id = client
        .workflow(
            "fatal_step",
            queue,
            Input {
                label: "bad-key".into(),
            },
        )
        .await?;
    tracing::info!(%run_id, "Started fatal workflow (should fail immediately)");
    tokio::spawn(async move { worker.run().await });
    tokio::time::sleep(Duration::from_secs(5)).await;
    Ok(())
}

async fn run_override(server: &str, queue: &str) -> anyhow::Result<()> {
    let step_retry = RetryPolicy {
        maximum_attempts: Some(5),
        initial_interval: Some(Duration::from_millis(200)),
        backoff_coefficient: Some(2.0),
        maximum_interval: Some(Duration::from_secs(3)),
        non_retryable_errors: vec![],
    };

    let mut worker = Worker::builder(server, queue)
        .default_step_retry(RetryPolicy {
            maximum_attempts: Some(2),
            initial_interval: Some(Duration::from_millis(500)),
            backoff_coefficient: Some(1.5),
            maximum_interval: Some(Duration::from_secs(2)),
            non_retryable_errors: vec![],
        })
        .build()
        .await?;

    let flaky = common::FlakyStep::succeed_after(4);
    worker.register(
        "override_step",
        move |mut ctx: WorkflowContext, _input: Input| {
            let flaky = flaky.clone();
            let step_retry = step_retry.clone();
            async move {
                let msg = ctx
                    .run_with_input_with_retry(
                        "override",
                        &"data",
                        Some(step_retry),
                        flaky.run("override"),
                    )
                    .await?;
                Ok(Output { message: msg })
            }
        },
    );

    let mut client = common::connect_client(server).await?;
    let run_id = client
        .workflow(
            "override_step",
            queue,
            Input {
                label: "override".into(),
            },
        )
        .await?;
    tracing::info!(%run_id, "Started workflow with per-step retry override");
    tokio::spawn(async move { worker.run().await });
    tokio::time::sleep(Duration::from_secs(15)).await;
    Ok(())
}
