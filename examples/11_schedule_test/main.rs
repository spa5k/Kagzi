use std::env;
use std::time::Duration;

use kagzi::{Context, Kagzi, Worker};
use serde::{Deserialize, Serialize};

#[path = "../common.rs"]
mod common;

#[derive(Debug, Serialize, Deserialize)]
struct CleanupInput {
    table: String,
}

async fn cleanup_workflow(
    mut ctx: Context,
    input: CleanupInput,
) -> anyhow::Result<serde_json::Value> {
    let now = chrono::Utc::now();
    println!(
        "🔥🔥🔥 [{}] SCHEDULED WORKFLOW FIRED! Cleaning: {}",
        now.format("%H:%M:%S"),
        input.table
    );

    let timestamp = ctx
        .step("get-timestamp")
        .run(|| async { Ok(chrono::Utc::now().to_rfc3339()) })
        .await?;

    println!("✅ Cleanup completed at: {}", timestamp);

    Ok(serde_json::json!({
        "table": input.table,
        "cleaned_at": timestamp
    }))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let server = env::var("KAGZI_SERVER_URL").unwrap_or_else(|_| "http://localhost:50051".into());
    let namespace = env::var("KAGZI_NAMESPACE").unwrap_or_else(|_| "schedule-test".into());

    println!("🚀 Live Schedule Test - demonstrates periodic workflow execution\n");
    println!("This example will:");
    println!("  1. Start a worker");
    println!("  2. Create a schedule that fires every 2 minutes");
    println!("  3. Run for 5 minutes to observe multiple executions");
    println!("  4. Clean up the schedule\n");

    // 1. Start worker
    let mut worker = Worker::new(&server)
        .namespace(&namespace)
        .workflows([("cleanup_workflow", cleanup_workflow)])
        .build()
        .await?;

    tokio::spawn(async move {
        println!("👷 Worker started\n");
        worker.run().await
    });

    // 2. Create schedule (fires every 2 minutes for testing)
    let client = Kagzi::connect(&server).await?;
    let schedule = client
        .schedule("cleanup_workflow")
        .namespace(&namespace)
        .workflow("cleanup_workflow")
        .cron("0 */2 * * * *") // every 2 minutes (second minute hour day month weekday)
        .input(CleanupInput {
            table: "live_test_sessions".into(),
        })
        .send()
        .await?;

    println!("📅 Schedule created: {}", schedule.schedule_id);
    println!("⏰ Cron: 0 */2 * * * * (fires every 2 minutes at second 0)");
    println!("🕐 Watch for '🔥🔥🔥' messages below...\n");
    println!("─────────────────────────────────────────────────");

    // 3. Wait 5 minutes to see 2-3 firings
    for i in (1..=10).rev() {
        tokio::time::sleep(Duration::from_secs(30)).await;
        println!("⏱️  {} seconds remaining...", i * 30);
    }

    println!("─────────────────────────────────────────────────");
    println!("\n✨ Test complete! You should see 2-3 firings above.");

    // 4. Cleanup
    client
        .delete_workflow_schedule(&schedule.schedule_id, Some(&namespace))
        .await?;

    println!("🧹 Schedule deleted");

    Ok(())
}
