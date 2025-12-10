#[path = "../common/mod.rs"]
mod common;

use std::time::Duration;

use common::TestHarness;
use kagzi::WorkflowContext;
use serde::{Deserialize, Serialize};
use tokio::time::sleep;
use uuid::Uuid;

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Clone, Copy)]
struct Empty;

#[tokio::test]
async fn scheduler_fires_workflow_on_cron() -> anyhow::Result<()> {
    let harness = TestHarness::new().await;
    let queue = "e2e-scheduling-cron";

    let mut worker = harness.worker(queue).await;
    worker.register(
        "cron_wf",
        |_ctx: WorkflowContext, _input: Empty| async move { Ok::<_, anyhow::Error>(()) },
    );
    let shutdown = worker.shutdown_token();
    let handle = tokio::spawn(async move { worker.run().await });

    let mut client = harness.client().await;
    let _schedule = client
        .workflow_schedule("cron_wf", queue, "*/1 * * * * *", Empty)
        .await?;

    sleep(Duration::from_secs(3)).await;

    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM kagzi.workflow_runs WHERE workflow_type = $1 AND status = 'COMPLETED'",
    )
    .bind("cron_wf")
    .fetch_one(&harness.pool)
    .await?;
    assert!(
        count >= 1,
        "expected at least one workflow fired by scheduler, got {}",
        count
    );
    let completed = harness.db_count_workflows_by_status("COMPLETED").await?;
    assert!(
        completed >= 1,
        "db helper should also see completed workflows, got {}",
        completed
    );

    shutdown.cancel();
    let _ = handle.await;
    Ok(())
}

#[tokio::test]
async fn scheduler_catchup_fires_missed_runs() -> anyhow::Result<()> {
    let harness = TestHarness::new().await;
    let queue = "e2e-scheduling-catchup";

    let mut worker = harness.worker(queue).await;
    worker.register(
        "catchup_wf",
        |_ctx: WorkflowContext, _input: Empty| async move { Ok::<_, anyhow::Error>(()) },
    );
    let shutdown = worker.shutdown_token();
    let handle = tokio::spawn(async move { worker.run().await });

    let mut client = harness.client().await;
    let schedule = client
        .workflow_schedule("catchup_wf", queue, "*/1 * * * * *", Empty)
        .await?;
    let schedule_id = Uuid::parse_str(&schedule.schedule_id)?;

    // Force next_fire_at into the past to trigger catchup.
    sqlx::query(
        "UPDATE kagzi.schedules SET next_fire_at = NOW() - INTERVAL '4 seconds', last_fired_at = NULL WHERE schedule_id = $1",
    )
    .bind(schedule_id)
    .execute(&harness.pool)
    .await?;

    sleep(Duration::from_secs(4)).await;

    let fired: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM kagzi.workflow_runs WHERE external_id = $1")
            .bind(schedule.schedule_id.clone())
            .fetch_one(&harness.pool)
            .await?;

    assert!(
        fired >= 2,
        "expected catchup to fire multiple runs, got {}",
        fired
    );
    let completed = harness.db_count_workflows_by_status("COMPLETED").await?;
    assert!(
        completed + 1 >= fired,
        "completed should be close to fired (allow 1 in-flight); completed={}, fired={}",
        completed,
        fired
    );

    shutdown.cancel();
    let _ = handle.await;
    Ok(())
}
