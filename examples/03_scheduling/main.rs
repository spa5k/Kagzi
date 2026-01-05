use std::env;

use kagzi::{Context, Kagzi, Worker};
use serde::{Deserialize, Serialize};

#[path = "../common.rs"]
mod common;

#[derive(Debug, Serialize, Deserialize)]
struct CleanupInput {
    table: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct SleepInput {
    step: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = env::args().collect();
    let variant = args.get(1).map(|s| s.as_str()).unwrap_or("cron");

    let server = env::var("KAGZI_SERVER_URL").unwrap_or_else(|_| "http://localhost:50051".into());
    let namespace = env::var("KAGZI_NAMESPACE").unwrap_or_else(|_| "scheduling".into());

    match variant {
        "cron" => cron_demo(&server, &namespace).await?,
        "sleep" => durable_sleep_demo(&server, &namespace).await?,
        "catchup" => catchup_demo(&server, &namespace).await?,
        _ => {
            eprintln!("Usage: cargo run -p kagzi --example 03_scheduling -- [cron|sleep|catchup]");
            std::process::exit(1);
        }
    }

    Ok(())
}

async fn cron_demo(server: &str, namespace: &str) -> anyhow::Result<()> {
    println!("📅 Cron Schedule Example - demonstrates time-based workflow scheduling\n");

    let client = Kagzi::connect(server).await?;
    let input = CleanupInput {
        table: "sessions".into(),
    };
    let schedule = client
        .schedule("cleanup_workflow")
        .namespace(namespace)
        .workflow("cleanup_workflow")
        .cron("*/1 * * * * *") // every second for demo
        .input(&input)?
        .send()
        .await?;

    println!("✅ Created schedule: schedule_id={}", schedule.schedule_id);

    let fetched = client
        .get_workflow_schedule(&schedule.schedule_id, Some(namespace))
        .await?;
    println!("🔍 Fetched schedule: {:?}", fetched);

    client
        .delete_workflow_schedule(&schedule.schedule_id, Some(namespace))
        .await?;
    println!("🗑️  Deleted schedule");
    println!("✅ Example complete\n");

    Ok(())
}

async fn durable_sleep_demo(server: &str, namespace: &str) -> anyhow::Result<()> {
    println!("⏱️  Durable Sleep Example - demonstrates resumable sleep across restarts\n");

    let mut worker = Worker::new(server)
        .namespace(namespace)
        .workflows([("sleep_demo", sleep_workflow)])
        .build()
        .await?;

    println!("👷 Worker started");

    let client = Kagzi::connect(server).await?;

    let input = SleepInput {
        step: "wait-and-resume".into(),
    };
    let run = client
        .start("sleep_demo")
        .namespace(namespace)
        .input(&input)?
        .send()
        .await?;

    println!(
        "🚀 Started sleep demo; stop worker during sleep to see resume: {}",
        run.id
    );
    tokio::spawn(async move { worker.run().await });
    tokio::time::sleep(std::time::Duration::from_secs(25)).await;
    println!("✅ Example complete\n");
    Ok(())
}

async fn catchup_demo(server: &str, namespace: &str) -> anyhow::Result<()> {
    println!("🔄 Catchup Example - demonstrates missed schedule execution replay\n");

    let client = Kagzi::connect(server).await?;
    let input = CleanupInput {
        table: "audit_logs".into(),
    };
    let schedule = client
        .schedule("catchup_workflow")
        .namespace(namespace)
        .workflow("catchup_workflow")
        .cron("*/5 * * * * *") // every 5 seconds
        .catchup(10)
        .input(&input)?
        .send()
        .await?;

    println!(
        "✅ Created catchup schedule; pause server to see replay: schedule_id={}",
        schedule.schedule_id
    );
    println!("💡 Tip: Stop the server, wait 15s, then restart to see catchup in action\n");
    Ok(())
}

async fn sleep_workflow(mut ctx: Context, input: SleepInput) -> anyhow::Result<serde_json::Value> {
    println!("Step A started: step={}", input.step);
    ctx.sleep("wait-15s", "15s").await?;
    println!("Step B resumed after durable sleep");
    Ok(serde_json::json!({ "status": "resumed", "step": input.step }))
}
