use std::env;

use kagzi::{Context, Kagzi, KagziError, Retry, Worker};
use serde::{Deserialize, Serialize};

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
    let args: Vec<String> = env::args().collect();
    let variant = args.get(1).map(|s| s.as_str()).unwrap_or("flaky");

    let server = env::var("KAGZI_SERVER_URL").unwrap_or_else(|_| "http://localhost:50051".into());
    let namespace = env::var("KAGZI_NAMESPACE").unwrap_or_else(|_| "error-handling".into());

    match variant {
        "flaky" => run_flaky(&server, &namespace).await?,
        "fatal" => run_fatal(&server, &namespace).await?,
        "override" => run_override(&server, &namespace).await?,
        _ => {
            eprintln!(
                "Usage: cargo run -p kagzi --example 02_error_handling -- [flaky|fatal|override]"
            );
            std::process::exit(1);
        }
    }

    Ok(())
}

async fn run_flaky(server: &str, namespace: &str) -> anyhow::Result<()> {
    println!("🔄 Flaky Retry Example - demonstrates automatic retry with exponential backoff\n");

    let flaky = common::FlakyStep::succeed_after(3);

    let mut worker = Worker::new(server)
        .namespace(namespace)
        .retry(Retry::exponential(3).initial("300ms").max("5s"))
        .workflows([("flaky_step", move |mut ctx: Context, input: Input| {
            let flaky = flaky.clone();
            async move {
                let msg = ctx.step("flaky").run(|| flaky.run(&input.label)).await?;
                Ok(Output { message: msg })
            }
        })])
        .build()
        .await?;

    println!("👷 Worker started");

    let client = Kagzi::connect(server).await?;
    let run = client
        .start("flaky_step")
        .namespace(namespace)
        .input(Input {
            label: "sometimes fails".into(),
        })
        .send()
        .await?;

    println!("🚀 Started flaky workflow (should retry twice): {}", run.id);
    tokio::spawn(async move { worker.run().await });
    tokio::time::sleep(std::time::Duration::from_secs(12)).await;
    println!("✅ Example complete\n");
    Ok(())
}

async fn run_fatal(server: &str, namespace: &str) -> anyhow::Result<()> {
    println!("💥 Fatal Error Example - demonstrates non-retryable error handling\n");

    let mut worker = Worker::new(server)
        .namespace(namespace)
        .workflows([("fatal_step", |_ctx: Context, input: Input| async move {
            if input.label.contains("bad-key") {
                Err(KagziError::non_retryable("invalid api key").into())
            } else {
                Ok(Output {
                    message: format!("processed {}", input.label),
                })
            }
        })])
        .build()
        .await?;

    println!("👷 Worker started");

    let client = Kagzi::connect(server).await?;
    let run = client
        .start("fatal_step")
        .namespace(namespace)
        .input(Input {
            label: "bad-key".into(),
        })
        .send()
        .await?;

    println!(
        "🚀 Started fatal workflow (should fail immediately): {}",
        run.id
    );
    tokio::spawn(async move { worker.run().await });
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    println!("✅ Example complete\n");
    Ok(())
}

async fn run_override(server: &str, namespace: &str) -> anyhow::Result<()> {
    println!("⚙️  Retry Override Example - demonstrates per-step retry configuration\n");

    let flaky = common::FlakyStep::succeed_after(4);
    let step_retry = Retry::exponential(5).initial("200ms").max("3s");

    let mut worker = Worker::new(server)
        .namespace(namespace)
        .retry(Retry::exponential(2).initial("500ms").max("2s"))
        .workflows([("override_step", move |mut ctx: Context, _input: Input| {
            let flaky = flaky.clone();
            let step_retry = step_retry.clone();
            async move {
                let msg = ctx
                    .step("override")
                    .retry(step_retry)
                    .run(|| flaky.run("override"))
                    .await?;
                Ok(Output { message: msg })
            }
        })])
        .build()
        .await?;

    println!("👷 Worker started");

    let client = Kagzi::connect(server).await?;
    let run = client
        .start("override_step")
        .namespace(namespace)
        .input(Input {
            label: "override".into(),
        })
        .send()
        .await?;

    println!(
        "🚀 Started workflow with per-step retry override: {}",
        run.id
    );
    tokio::spawn(async move { worker.run().await });
    tokio::time::sleep(std::time::Duration::from_secs(15)).await;
    println!("✅ Example complete\n");
    Ok(())
}
