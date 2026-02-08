//! Schedule Backfill Demo
//!
//! This example demonstrates the backfill behavior when a worker is unavailable
//! during scheduled workflow triggers. It:
//! 1. Creates a schedule that fires every 10 seconds
//! 2. Runs the worker normally for 30 seconds
//! 3. STOPS the worker for ~25 seconds (missing 2 scheduled fires)
//! 4. Restarts the worker and watches the coordinator catch up missed runs
//!
//! # max_catchup Parameter
//!
//! The `max_catchup` parameter controls how many missed runs will be executed
//! when a schedule resumes after downtime:
//!
//! - `max_catchup = 0`: Skip all missed runs, jump to current time
//! - `max_catchup = N`: Catch up at most N missed runs, skip the rest
//! - `max_catchup = 50` (default): Catch up to 50 missed runs
//!
//! This prevents overwhelming the system if a schedule has been down for a long time.
//! For example, with a 1-minute schedule that's been down for a day (1440 minutes),
//! you'd get 1440 missed runs. With max_catchup=50, only the 50 most recent runs
//! will execute, preventing resource exhaustion.

use std::env;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use kagzi::{Context, Kagzi, Worker};
use serde::{Deserialize, Serialize};

#[path = "../common.rs"]
mod common;

#[derive(Debug, Serialize, Deserialize)]
struct PingInput {
    seq: u32,
}

static FIRE_COUNT: AtomicU32 = AtomicU32::new(0);

async fn ping_workflow(mut ctx: Context, input: PingInput) -> anyhow::Result<serde_json::Value> {
    let count = FIRE_COUNT.fetch_add(1, Ordering::SeqCst) + 1;
    let now = chrono::Utc::now();

    println!(
        "🔥 [{}] FIRE #{} - Workflow executed (input seq: {})",
        now.format("%H:%M:%S"),
        count,
        input.seq
    );

    let timestamp = ctx
        .step("record-ping")
        .run(|| async { Ok(chrono::Utc::now().to_rfc3339()) })
        .await?;

    Ok(serde_json::json!({
        "fire_number": count,
        "timestamp": timestamp
    }))
}

async fn run_worker(server: &str, namespace: &str) -> anyhow::Result<()> {
    let mut worker = Worker::new(server)
        .namespace(namespace)
        .workflows([("ping_workflow", ping_workflow)])
        .build()
        .await?;
    worker.run().await
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let server = env::var("KAGZI_SERVER_URL").unwrap_or_else(|_| "http://localhost:50051".into());
    let namespace = env::var("KAGZI_NAMESPACE").unwrap_or_else(|_| "backfill-demo".into());

    println!("🚀 Schedule Backfill Demo\n");
    println!("This example demonstrates how Kagzi catches up missed schedule runs.");
    println!("─────────────────────────────────────────────────────────────────────\n");
    println!("Plan:");
    println!("  1. Create a schedule that fires every 10 seconds");
    println!("  2. Run worker for 30s (expect ~3 fires)");
    println!("  3. STOP worker for ~25s (miss ~2-3 scheduled runs)");
    println!("  4. Restart worker and watch backfill in action\n");
    println!("─────────────────────────────────────────────────────────────────────\n");

    // 1. Create schedule (fires every 10 seconds)
    let client = Kagzi::connect(&server).await?;
    let input = PingInput { seq: 1 };
    let schedule = client
        .schedule("ping_workflow")
        .namespace(&namespace)
        .workflow("ping_workflow")
        .cron("*/10 * * * * *") // every 10 seconds
        .input(&input)?
        .catchup(50) // Catch up to 50 missed runs (prevents resource exhaustion)
        .send()
        .await?;

    println!("📅 Schedule created: {}", schedule.schedule_id);
    println!("⏰ Cron: */10 * * * * * (fires every 10 seconds)");
    println!("🔧 max_catchup: 50 (will catch up to 50 most recent missed runs)\n");

    // 2. Phase 1: Worker running normally
    println!("═══════════════════════════════════════════════════════════════════");
    println!("📶 PHASE 1: Worker ONLINE (30 seconds)");
    println!("═══════════════════════════════════════════════════════════════════\n");

    let server_clone = server.clone();
    let namespace_clone = namespace.clone();
    let worker_handle = tokio::spawn(async move {
        if let Err(e) = run_worker(&server_clone, &namespace_clone).await {
            eprintln!("Worker error in Phase 1: {:?}", e);
        }
    });

    // Wait 30 seconds with worker running
    for i in (1..=6).rev() {
        tokio::time::sleep(Duration::from_secs(5)).await;
        println!("  ⏱️  Phase 1: {} seconds remaining...", i * 5);
    }

    let fires_phase1 = FIRE_COUNT.load(Ordering::SeqCst);
    println!("\n📊 Phase 1 complete: {} fires observed\n", fires_phase1);

    // 3. Phase 2: Worker DOWN - simulating downtime
    println!("═══════════════════════════════════════════════════════════════════");
    println!("🔴 PHASE 2: Worker OFFLINE (25 seconds) - MISSING SCHEDULED RUNS");
    println!("═══════════════════════════════════════════════════════════════════\n");

    worker_handle.abort(); // Kill the worker
    println!("⚠️  Worker stopped! Schedules will still fire but won't be processed.\n");

    // Wait 25 seconds with NO worker
    for i in (1..=5).rev() {
        tokio::time::sleep(Duration::from_secs(5)).await;
        println!(
            "  💤 Downtime: {} seconds remaining... (missed fires accumulating)",
            i * 5
        );
    }

    let fires_phase2 = FIRE_COUNT.load(Ordering::SeqCst);
    println!(
        "\n📊 Phase 2 complete: still {} fires (no new ones - worker was down)\n",
        fires_phase2
    );

    // 4. Phase 3: Worker back online - watch backfill!
    println!("═══════════════════════════════════════════════════════════════════");
    println!("🟢 PHASE 3: Worker BACK ONLINE - Watching backfill!");
    println!("═══════════════════════════════════════════════════════════════════\n");

    let server_clone = server.clone();
    let namespace_clone = namespace.clone();
    let worker_handle = tokio::spawn(async move {
        if let Err(e) = run_worker(&server_clone, &namespace_clone).await {
            eprintln!("Worker error in Phase 3: {:?}", e);
        }
    });

    println!("👀 Watch for rapid 🔥 fires as missed runs catch up...\n");

    // Wait 30 seconds to observe backfill
    for i in (1..=6).rev() {
        tokio::time::sleep(Duration::from_secs(5)).await;
        let current_fires = FIRE_COUNT.load(Ordering::SeqCst);
        println!(
            "  ⏱️  Phase 3: {} seconds remaining... (fires so far: {})",
            i * 5,
            current_fires
        );
    }

    let fires_phase3 = FIRE_COUNT.load(Ordering::SeqCst);
    let backfilled = fires_phase3.saturating_sub(fires_phase1);

    println!("\n═══════════════════════════════════════════════════════════════════");
    println!("📊 RESULTS");
    println!("═══════════════════════════════════════════════════════════════════");
    println!("  Phase 1 fires (30s online):  {}", fires_phase1);
    println!("  Phase 2 fires (25s offline): 0 (worker down)");
    println!(
        "  Phase 3 fires (30s recovery): {} (including backfill!)",
        backfilled
    );
    println!("  ─────────────────────────────");
    println!("  Total fires: {}", fires_phase3);
    println!();

    if backfilled > 3 {
        println!("✅ Backfill detected! Missed runs were caught up.");
    } else {
        println!("ℹ️  Run longer or check coordinator logs for backfill activity.");
    }

    // Cleanup
    worker_handle.abort();
    client
        .delete_workflow_schedule(&schedule.schedule_id, Some(&namespace))
        .await?;
    println!("\n🧹 Schedule deleted. Demo complete!");

    Ok(())
}
