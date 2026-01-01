use std::env;

use kagzi::{Context, Kagzi, Worker};
use serde::{Deserialize, Serialize};

#[path = "../common.rs"]
mod common;

#[derive(Debug, Serialize, Deserialize)]
struct HelloInput {
    name: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct HelloOutput {
    message: String,
}

async fn hello_workflow(_ctx: Context, input: HelloInput) -> anyhow::Result<HelloOutput> {
    Ok(HelloOutput {
        message: format!("Hello, {}!", input.name),
    })
}

#[derive(Debug, Serialize, Deserialize)]
struct ChainInput {
    name: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct ChainOutput {
    final_message: String,
}

async fn step_greet(name: String) -> anyhow::Result<String> {
    Ok(format!("hello, {name}"))
}

async fn step_upper(text: String) -> anyhow::Result<String> {
    Ok(text.to_uppercase())
}

async fn step_suffix(text: String) -> anyhow::Result<String> {
    Ok(format!("{text} !!!"))
}

async fn chain_workflow(mut ctx: Context, input: ChainInput) -> anyhow::Result<ChainOutput> {
    let greeted = ctx.step("greet").run(|| step_greet(input.name)).await?;
    let upper = ctx.step("uppercase").run(|| step_upper(greeted)).await?;
    let final_message = ctx.step("suffix").run(|| step_suffix(upper)).await?;
    Ok(ChainOutput { final_message })
}

#[derive(Debug, Serialize, Deserialize)]
struct ContextAwareInput {
    name: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct ContextAwareOutput {
    message: String,
    env: String,
}

async fn context_workflow(
    _ctx: Context,
    input: ContextAwareInput,
) -> anyhow::Result<ContextAwareOutput> {
    // Workflow-level context is not yet exposed via the worker API, so we read
    // an env var to demonstrate per-deployment configuration.
    let env_str = std::env::var("EXAMPLE_ENV").unwrap_or_else(|_| "unknown".into());

    Ok(ContextAwareOutput {
        message: format!("Hello, {}!", input.name),
        env: env_str,
    })
}

#[derive(Debug, Serialize, Deserialize)]
struct SleepInput {
    name: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct SleepOutput {
    message: String,
    steps_completed: u32,
}

async fn step_process(step_num: u32, name: String) -> anyhow::Result<String> {
    let result = format!("Step {} processing {}", step_num, name);
    println!("  → {}", result);
    Ok(result)
}

async fn sleep_workflow(mut ctx: Context, input: SleepInput) -> anyhow::Result<SleepOutput> {
    println!("Starting sleep workflow for: {}", input.name);

    // Step 1: Process and sleep
    let step1 = ctx
        .step("step_1")
        .run(|| step_process(1, input.name.clone()))
        .await?;
    println!("  → Sleeping for 6 seconds...");
    ctx.sleep("sleep_1", "6s").await?;

    // Step 2: Process and sleep
    let step2 = ctx
        .step("step_2")
        .run(|| step_process(2, input.name.clone()))
        .await?;
    println!("  → Sleeping for 7 seconds...");
    ctx.sleep("sleep_2", "7s").await?;

    // Step 3: Process and sleep
    let step3 = ctx
        .step("step_3")
        .run(|| step_process(3, input.name.clone()))
        .await?;
    println!("  → Sleeping for 8 seconds...");
    ctx.sleep("sleep_3", "8s").await?;

    println!("All steps completed!");
    Ok(SleepOutput {
        message: format!("Completed: {} -> {} -> {}", step1, step2, step3),
        steps_completed: 3,
    })
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = env::args().collect();
    let variant = args.get(1).map(|s| s.as_str()).unwrap_or("hello");

    let server = env::var("KAGZI_SERVER_URL").unwrap_or_else(|_| "http://localhost:50051".into());
    let namespace = env::var("KAGZI_NAMESPACE").unwrap_or_else(|_| "basics".into());

    match variant {
        "hello" => run_hello(&server, &namespace).await?,
        "chain" => run_chain(&server, &namespace).await?,
        "context" => run_context(&server, &namespace).await?,
        "sleep" => run_sleep(&server, &namespace).await?,
        _ => {
            eprintln!(
                "Usage: cargo run -p kagzi --example 01_basics -- [hello|chain|context|sleep]"
            );
            std::process::exit(1);
        }
    }

    Ok(())
}

async fn run_hello(server: &str, namespace: &str) -> anyhow::Result<()> {
    let mut worker = Worker::new(server)
        .namespace(namespace)
        .workflows([("hello_workflow", hello_workflow)])
        .build()
        .await?;

    let client = Kagzi::connect(server).await?;

    let run = client
        .start("hello_workflow")
        .namespace(namespace)
        .input(HelloInput {
            name: "Kagzi".to_string(),
        })
        .send()
        .await?;

    println!("Started hello workflow: {}", run.id);
    tokio::spawn(async move {
        if let Err(e) = worker.run().await {
            eprintln!("Worker error: {:?}", e);
        }
    });
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    Ok(())
}

async fn run_chain(server: &str, namespace: &str) -> anyhow::Result<()> {
    let mut worker = Worker::new(server)
        .namespace(namespace)
        .workflows([("chain_workflow", chain_workflow)])
        .build()
        .await?;

    let client = Kagzi::connect(server).await?;

    let run = client
        .start("chain_workflow")
        .namespace(namespace)
        .input(ChainInput {
            name: "pipeline".to_string(),
        })
        .send()
        .await?;

    println!("Started chained workflow: {}", run.id);
    tokio::spawn(async move {
        if let Err(e) = worker.run().await {
            eprintln!("Worker error: {:?}", e);
        }
    });
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    Ok(())
}

async fn run_context(server: &str, namespace: &str) -> anyhow::Result<()> {
    let mut worker = Worker::new(server)
        .namespace(namespace)
        .workflows([("context_workflow", context_workflow)])
        .build()
        .await?;

    let client = Kagzi::connect(server).await?;

    let run = client
        .start("context_workflow")
        .namespace(namespace)
        .input(ContextAwareInput {
            name: "context".into(),
        })
        .send()
        .await?;

    println!("Started context workflow: {}", run.id);
    tokio::spawn(async move {
        if let Err(e) = worker.run().await {
            eprintln!("Worker error: {:?}", e);
        }
    });
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    Ok(())
}

async fn run_sleep(server: &str, namespace: &str) -> anyhow::Result<()> {
    let mut worker = Worker::new(server)
        .namespace(namespace)
        .workflows([("sleep_workflow", sleep_workflow)])
        .build()
        .await?;

    let client = Kagzi::connect(server).await?;

    let run = client
        .start("sleep_workflow")
        .namespace(namespace)
        .input(SleepInput {
            name: "sleeper".to_string(),
        })
        .send()
        .await?;

    println!("Started sleep workflow: {}", run.id);
    println!("This workflow will take ~20 seconds to complete (3 steps with 5-10s sleeps)");
    tokio::spawn(async move {
        if let Err(e) = worker.run().await {
            eprintln!("Worker error: {:?}", e);
        }
    });
    tokio::time::sleep(std::time::Duration::from_secs(25)).await;
    Ok(())
}
