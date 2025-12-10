use std::time::Duration;

use kagzi::WorkflowContext;
use kagzi_proto::kagzi::WorkflowStatus;
use serde::{Deserialize, Serialize};
use tests::common::{TestConfig, TestHarness, wait_for_status};
use uuid::Uuid;

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
struct SleepInput {
    seconds: u64,
}

/// Test that a sleeping workflow can be resumed by a different worker after the original
/// worker dies. This verifies that:
/// 1. Sleep steps are properly persisted and can be replayed
/// 2. A new worker can claim and complete the workflow after the sleep expires
#[tokio::test]
async fn workflow_survives_worker_restart_during_sleep() -> anyhow::Result<()> {
    let harness = TestHarness::new().await;
    let queue = "e2e-resilience-sleep-v2";

    let mut worker1 = harness.worker(queue).await;
    worker1.register(
        "simple_sleep",
        |mut ctx: WorkflowContext, _input: SleepInput| async move {
            ctx.sleep(Duration::from_secs(2)).await?;
            Ok::<_, anyhow::Error>(())
        },
    );
    let shutdown1 = worker1.shutdown_token();
    let handle1 = tokio::spawn(async move { worker1.run().await });

    let mut client = harness.client().await;
    let run_id = client
        .workflow("simple_sleep", queue, SleepInput { seconds: 0 })
        .await?;
    let run_uuid = Uuid::parse_str(&run_id)?;

    // Wait for workflow to enter SLEEPING state
    for _ in 0..20 {
        let status = harness.db_workflow_status(&run_uuid).await?;
        if status == "SLEEPING" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    let pre_kill_status = harness.db_workflow_status(&run_uuid).await?;
    assert_eq!(
        pre_kill_status, "SLEEPING",
        "workflow should be sleeping before worker dies"
    );

    // Wait for the sleep step to be marked COMPLETED. There's a window between workflow
    // entering SLEEPING and the step being marked COMPLETED.
    for _ in 0..20 {
        if harness.db_step_status(&run_uuid, "__sleep_0").await? == Some("COMPLETED".to_string()) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let step_status = harness.db_step_status(&run_uuid, "__sleep_0").await?;
    assert_eq!(
        step_status,
        Some("COMPLETED".to_string()),
        "sleep step should be COMPLETED before killing worker"
    );

    // Gracefully shutdown worker1
    shutdown1.cancel();
    let _ = handle1.await;

    // Wait for sleep to expire (2s) + buffer
    tokio::time::sleep(Duration::from_secs(4)).await;

    // Start worker2 which should pick up and complete the resumed workflow
    let mut worker2 = harness.worker(queue).await;
    worker2.register(
        "simple_sleep",
        |mut ctx: WorkflowContext, _input: SleepInput| async move {
            // Must match worker1's sleep duration for deterministic replay
            ctx.sleep(Duration::from_secs(2)).await?;
            Ok::<_, anyhow::Error>(())
        },
    );
    let shutdown2 = worker2.shutdown_token();
    let handle2 = tokio::spawn(async move { worker2.run().await });

    // Workflow should complete after worker2 picks it up and replays the sleep step
    let wf = wait_for_status(&harness.server_url, &run_id, WorkflowStatus::Completed, 30).await?;
    assert_eq!(wf.status, WorkflowStatus::Completed as i32);

    shutdown2.cancel();
    let _ = handle2.await;
    Ok(())
}

#[tokio::test]
async fn orphaned_workflow_recovered_after_worker_death() -> anyhow::Result<()> {
    let harness = TestHarness::with_config(TestConfig {
        worker_stale_threshold_secs: 2,
        watchdog_interval_secs: 1,
        ..Default::default()
    })
    .await;
    let queue = "e2e-resilience-orphan";

    let mut worker1 = harness.worker(queue).await;
    worker1.register(
        "long_run",
        |_ctx: WorkflowContext, _input: SleepInput| async move {
            tokio::time::sleep(Duration::from_secs(10)).await;
            Ok::<_, anyhow::Error>(())
        },
    );
    let shutdown1 = worker1.shutdown_token();
    let handle1 = tokio::spawn(async move { worker1.run().await });

    let mut client = harness.client().await;
    let run_id = client
        .workflow("long_run", queue, SleepInput { seconds: 0 })
        .await?;
    let run_uuid = Uuid::parse_str(&run_id)?;

    tokio::time::sleep(Duration::from_millis(500)).await;
    shutdown1.cancel();
    let _ = handle1.await;

    // Wait for watchdog to mark stale + schedule retry.
    tokio::time::sleep(Duration::from_secs(6)).await;

    let mut worker2 = harness.worker(queue).await;
    worker2.register(
        "long_run",
        |_ctx: WorkflowContext, _input: SleepInput| async move { Ok::<_, anyhow::Error>(()) },
    );
    let shutdown2 = worker2.shutdown_token();
    let handle2 = tokio::spawn(async move { worker2.run().await });

    let wf = wait_for_status(&harness.server_url, &run_id, WorkflowStatus::Completed, 30).await?;
    assert_eq!(wf.status, WorkflowStatus::Completed as i32);

    // Locked_by should be cleared after recovery.
    let locked_by = harness.db_workflow_locked_by(&run_uuid).await?;
    assert!(
        locked_by.is_none(),
        "lock should be cleared after recovery, got {:?}",
        locked_by
    );

    shutdown2.cancel();
    let _ = handle2.await;
    Ok(())
}
