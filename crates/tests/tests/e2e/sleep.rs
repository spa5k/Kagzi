use std::time::Duration;

use kagzi::WorkflowContext;
use kagzi_proto::kagzi::WorkflowStatus;
use serde::{Deserialize, Serialize};
use tests::common::{TestConfig, TestHarness, wait_for_status};
use tokio::time::sleep;
use uuid::Uuid;

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Clone, Copy)]
struct Empty;

#[tokio::test]
async fn sleeping_workflow_claimable_after_wake_up() -> anyhow::Result<()> {
    let harness = TestHarness::with_config(TestConfig {
        watchdog_interval_secs: 1,
        poll_timeout_secs: 1,
        ..Default::default()
    })
    .await;
    let queue = "e2e-sleep-claimable";

    let mut worker1 = harness.worker(queue).await;
    worker1.register(
        "sleepy",
        |mut ctx: WorkflowContext, _input: Empty| async move {
            ctx.sleep(Duration::from_secs(2)).await?;
            Ok::<_, anyhow::Error>(())
        },
    );
    let shutdown1 = worker1.shutdown_token();
    let handle1 = tokio::spawn(async move { worker1.run().await });

    let mut client = harness.client().await;
    let run_id = client.workflow("sleepy", queue, Empty).await?;
    let run_uuid = Uuid::parse_str(&run_id)?;

    harness
        .wait_for_db_status(&run_uuid, "SLEEPING", 20, Duration::from_millis(150))
        .await?;

    shutdown1.cancel();
    let _ = handle1.await;

    sleep(Duration::from_secs(3)).await;

    harness
        .wait_for_db_status(&run_uuid, "PENDING", 20, Duration::from_millis(200))
        .await?;

    let mut worker2 = harness.worker(queue).await;
    worker2.register(
        "sleepy",
        |mut ctx: WorkflowContext, _input: Empty| async move {
            ctx.sleep(Duration::from_secs(2)).await?;
            Ok::<_, anyhow::Error>(())
        },
    );
    let shutdown2 = worker2.shutdown_token();
    let handle2 = tokio::spawn(async move { worker2.run().await });

    let wf = wait_for_status(&harness.server_url, &run_id, WorkflowStatus::Completed, 30).await?;
    assert_eq!(wf.status, WorkflowStatus::Completed as i32);

    shutdown2.cancel();
    let _ = handle2.await;
    Ok(())
}

#[tokio::test]
async fn watchdog_wakes_sleeping_workflows_in_batches() -> anyhow::Result<()> {
    let harness = TestHarness::with_config(TestConfig {
        watchdog_interval_secs: 1,
        poll_timeout_secs: 1,
        ..Default::default()
    })
    .await;
    let queue = "e2e-sleep-watchdog";

    let mut worker = harness.worker(queue).await;
    worker.register(
        "batch_sleep",
        |mut ctx: WorkflowContext, _input: Empty| async move {
            ctx.sleep(Duration::from_secs(2)).await?;
            Ok::<_, anyhow::Error>(())
        },
    );
    let shutdown = worker.shutdown_token();
    let handle = tokio::spawn(async move { worker.run().await });

    let mut client = harness.client().await;
    let mut runs = Vec::new();
    for _ in 0..3 {
        let id = client.workflow("batch_sleep", queue, Empty).await?;
        runs.push(Uuid::parse_str(&id)?);
    }

    for run in &runs {
        harness
            .wait_for_db_status(run, "SLEEPING", 20, Duration::from_millis(150))
            .await?;
    }

    shutdown.cancel();
    let _ = handle.await;

    sleep(Duration::from_secs(4)).await;

    let pending: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM kagzi.workflow_runs WHERE task_queue = $1 AND workflow_type = $2 AND status = 'PENDING'",
    )
    .bind(queue)
    .bind("batch_sleep")
    .fetch_one(&harness.pool)
    .await?;
    assert_eq!(
        pending, 3,
        "scheduler should wake all sleeping workflows after sleep expires"
    );

    let sleeping: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM kagzi.workflow_runs WHERE task_queue = $1 AND workflow_type = $2 AND status = 'SLEEPING'",
    )
    .bind(queue)
    .bind("batch_sleep")
    .fetch_one(&harness.pool)
    .await?;
    assert_eq!(sleeping, 0, "no sleeping workflows should remain");

    Ok(())
}

#[tokio::test]
async fn worker_can_directly_claim_expired_sleeping_workflow() -> anyhow::Result<()> {
    let harness = TestHarness::with_config(TestConfig {
        watchdog_interval_secs: 60, // ensure direct poll handles wake-up
        poll_timeout_secs: 1,
        ..Default::default()
    })
    .await;
    let queue = "e2e-sleep-direct-claim";

    let mut worker = harness.worker(queue).await;
    worker.register(
        "direct_sleep",
        |mut ctx: WorkflowContext, _input: Empty| async move {
            ctx.sleep(Duration::from_secs(2)).await?;
            Ok::<_, anyhow::Error>(())
        },
    );
    let shutdown = worker.shutdown_token();
    let handle = tokio::spawn(async move { worker.run().await });

    let mut client = harness.client().await;
    let run_id = client.workflow("direct_sleep", queue, Empty).await?;

    let wf = wait_for_status(&harness.server_url, &run_id, WorkflowStatus::Completed, 40).await?;
    assert_eq!(wf.status, WorkflowStatus::Completed as i32);

    shutdown.cancel();
    let _ = handle.await;
    Ok(())
}

#[tokio::test]
async fn sleep_step_replay_skips_execution() -> anyhow::Result<()> {
    let harness = TestHarness::new().await;
    let queue = "e2e-sleep-replay";

    let mut worker1 = harness.worker(queue).await;
    worker1.register(
        "replay_sleep",
        |mut ctx: WorkflowContext, _input: Empty| async move {
            ctx.sleep(Duration::from_secs(3)).await?;
            Ok::<_, anyhow::Error>(())
        },
    );
    let shutdown1 = worker1.shutdown_token();
    let handle1 = tokio::spawn(async move { worker1.run().await });

    let mut client = harness.client().await;
    let run_id = client.workflow("replay_sleep", queue, Empty).await?;
    let run_uuid = Uuid::parse_str(&run_id)?;

    harness
        .wait_for_db_status(&run_uuid, "SLEEPING", 20, Duration::from_millis(150))
        .await?;

    // Note: With lazy sleep completion (Simplify_plan.md section 2.3), the sleep step
    // stays in RUNNING status until the workflow resumes. The step will be completed
    // lazily by `complete_pending_sleep_steps()` when a new worker claims the workflow.
    // We only verify the step exists, not that it's completed yet.
    let step_status = harness.db_step_status(&run_uuid, "__sleep_0").await?;
    assert!(step_status.is_some(), "sleep step should exist");

    shutdown1.cancel();
    let _ = handle1.await;

    sleep(Duration::from_secs(4)).await;

    let mut worker2 = harness.worker(queue).await;
    worker2.register(
        "replay_sleep",
        |mut ctx: WorkflowContext, _input: Empty| async move {
            ctx.sleep(Duration::from_secs(3)).await?;
            Ok::<_, anyhow::Error>(())
        },
    );
    let shutdown2 = worker2.shutdown_token();
    let handle2 = tokio::spawn(async move { worker2.run().await });

    let wf = wait_for_status(&harness.server_url, &run_id, WorkflowStatus::Completed, 40).await?;
    assert_eq!(wf.status, WorkflowStatus::Completed as i32);

    let attempts = harness
        .db_step_attempt_count(&run_uuid, "__sleep_0")
        .await?;
    assert_eq!(attempts, 1, "sleep step should not re-execute on replay");

    shutdown2.cancel();
    let _ = handle2.await;
    Ok(())
}
