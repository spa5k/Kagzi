use kagzi::WorkflowContext;
use serde::{Deserialize, Serialize};
use std::env;

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

async fn hello_workflow(_ctx: WorkflowContext, input: HelloInput) -> anyhow::Result<HelloOutput> {
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

async fn chain_workflow(
    mut ctx: WorkflowContext,
    input: ChainInput,
) -> anyhow::Result<ChainOutput> {
    let greeted = ctx.run("greet", step_greet(input.name)).await?;
    let upper = ctx.run("uppercase", step_upper(greeted)).await?;
    let final_message = ctx.run("suffix", step_suffix(upper)).await?;
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
    _ctx: WorkflowContext,
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
    common::init_tracing()?;
    let args: Vec<String> = env::args().collect();
    let variant = args.get(1).map(|s| s.as_str()).unwrap_or("hello");

    let server = env::var("KAGZI_SERVER_URL").unwrap_or_else(|_| "http://localhost:50051".into());
    let queue = env::var("KAGZI_TASK_QUEUE").unwrap_or_else(|_| "basics".into());

    match variant {
        "hello" => run_hello(&server, &queue).await?,
        "chain" => run_chain(&server, &queue).await?,
        "context" => run_context(&server, &queue).await?,
        _ => {
            eprintln!("Usage: cargo run -p kagzi --example 01_basics -- [hello|chain|context]");
            std::process::exit(1);
        }
    }

    Ok(())
}

async fn run_hello(server: &str, queue: &str) -> anyhow::Result<()> {
    let mut worker = common::build_worker(server, queue).await?;
    worker.register("hello_workflow", hello_workflow);
    let mut client = common::connect_client(server).await?;

    let run_id = client
        .workflow(
            "hello_workflow",
            queue,
            HelloInput {
                name: "Kagzi".to_string(),
            },
        )
        .await?;

    tracing::info!(%run_id, "Started hello workflow");
    tokio::spawn(async move { worker.run().await });
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    Ok(())
}

async fn run_chain(server: &str, queue: &str) -> anyhow::Result<()> {
    let mut worker = common::build_worker(server, queue).await?;
    worker.register("chain_workflow", chain_workflow);
    let mut client = common::connect_client(server).await?;

    let run_id = client
        .workflow(
            "chain_workflow",
            queue,
            ChainInput {
                name: "pipeline".to_string(),
            },
        )
        .await?;

    tracing::info!(%run_id, "Started chained workflow");
    tokio::spawn(async move { worker.run().await });
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    Ok(())
}

async fn run_context(server: &str, queue: &str) -> anyhow::Result<()> {
    let mut worker = common::build_worker(server, queue).await?;
    worker.register("context_workflow", context_workflow);
    let mut client = common::connect_client(server).await?;

    let run_id = client
        .workflow(
            "context_workflow",
            queue,
            ContextAwareInput {
                name: "context".into(),
            },
        )
        .context(serde_json::json!({ "env": "production" }))
        .await?;

    tracing::info!(%run_id, "Started context workflow");
    tokio::spawn(async move { worker.run().await });
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    Ok(())
}
