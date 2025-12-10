use kagzi::{Worker, WorkflowContext};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
struct Input {
    message: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct Output {
    response: String,
    processed_at: String,
}

async fn process_message(message: String) -> anyhow::Result<String> {
    Ok(format!("Hello, {}! Your message was processed.", message))
}

async fn add_timestamp(response: String) -> anyhow::Result<Output> {
    Ok(Output {
        response,
        processed_at: chrono::Utc::now().to_rfc3339(),
    })
}

async fn welcome_workflow(mut ctx: WorkflowContext, input: Input) -> anyhow::Result<Output> {
    let processed = ctx
        .run("process_message", process_message(input.message))
        .await?;

    let output = ctx.run("add_timestamp", add_timestamp(processed)).await?;

    Ok(output)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    kagzi::tracing_utils::init_tracing()?;

    let mut worker = Worker::builder("http://localhost:50051", "welcome-queue")
        .build()
        .await?;
    worker.register("WelcomeEmail", welcome_workflow);
    worker.run().await
}
