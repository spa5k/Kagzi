use kagzi::{Client, Worker, WorkflowContext};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::info;

#[derive(Serialize, Deserialize, Debug)]
struct MyInput {
    name: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct MyOutput {
    message: String,
}

async fn step1() -> anyhow::Result<String> {
    info!("Running step 1");
    Ok("Hello".to_string())
}

async fn step2() -> anyhow::Result<String> {
    info!("Running step 2");
    Ok("World".to_string())
}

async fn my_workflow(mut ctx: WorkflowContext, input: MyInput) -> anyhow::Result<MyOutput> {
    info!("Workflow started with input: {:?}", input);

    let step1_res = ctx.run("step1", step1()).await?;

    info!("Step 1 result: {}", step1_res);
    ctx.sleep(Duration::from_secs(2)).await?;
    info!("Woke up from sleep");

    let step2_res = ctx.run("step2", step2()).await?;

    Ok(MyOutput {
        message: format!("{} {}, {}", step1_res, step2_res, input.name),
    })
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let mut worker = Worker::builder("http://localhost:50051", "default")
        .build()
        .await?;
    worker.register("my_workflow", my_workflow);

    tokio::spawn(async move {
        if let Err(e) = worker.run().await {
            panic!("Worker failed: {:?}", e);
        }
    });

    let mut client = Client::connect("http://localhost:50051").await?;

    // Simple
    let run_id = client
        .workflow(
            "my_workflow",
            "default",
            MyInput {
                name: "Kagzi".to_string(),
            },
        )
        .await?;

    info!("Started simple workflow: {}", run_id);

    // With options
    let run_id = client
        .workflow(
            "my_workflow",
            "default",
            MyInput {
                name: "Advanced Kagzi".to_string(),
            },
        )
        .idempotent(format!("workflow-{}", uuid::Uuid::new_v4()))
        .version("1.0.0")
        .context(serde_json::json!({
            "user_id": "user-123",
            "source": "api_v1"
        }))
        .deadline(chrono::Utc::now() + chrono::Duration::minutes(30))
        .await?;

    info!("Started advanced workflow: {}", run_id);

    tokio::time::sleep(Duration::from_secs(10)).await;
    Ok(())
}
