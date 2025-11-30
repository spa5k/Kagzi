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

async fn my_workflow(mut ctx: WorkflowContext, input: MyInput) -> anyhow::Result<MyOutput> {
    info!("Workflow started with input: {:?}", input);

    let step1_res: String = ctx
        .step("step1111", || async {
            info!("Running step 1");
            Ok("Hello1111".to_string())
        })
        .await?;

    info!("Step 1 result: {}", step1_res);
    ctx.sleep(Duration::from_secs(2)).await?;
    info!("Woke up from sleep");

    // send heartbeat

    let step2_res: String = ctx
        .step("step2111", || async {
            info!("Running step 2");
            Ok("Worl111d".to_string())
        })
        .await?;

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

    // Start Worker
    let mut worker =
        Worker::new("http://localhost:50051".to_string(), "default".to_string()).await?;
    worker.register_workflow("my_workflow", my_workflow);

    tokio::spawn(async move {
        if let Err(e) = worker.run().await {
            panic!("Worker failed: {:?}", e);
        }
    });

    // Start Workflow
    let mut client = Client::new("http://localhost:50051".to_string()).await?;
    let run_id = client
        .start_workflow(
            format!("test-workflow-{}", uuid::Uuid::new_v4()),
            "default".to_string(),
            "my_workflow".to_string(),
            MyInput {
                name: "Kagzi".to_string(),
            },
        )
        .await?;

    info!("Started workflow: {}", run_id);

    // Keep main alive
    tokio::time::sleep(Duration::from_secs(10)).await;
    Ok(())
}
