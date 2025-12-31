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
        _ => {
            eprintln!("Usage: cargo run -p kagzi --example 01_basics -- [hello|chain|context]");
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
        .r#await()
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
        .r#await()
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
        .r#await()
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
