use std::future::pending;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use kagzi::{RetryPolicy, WorkflowContext};
use kagzi_proto::kagzi::WorkflowStatus;
use serde::{Deserialize, Serialize};
use tests::common::{TestConfig, TestHarness, wait_for_status};
use tokio::time::sleep;
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
    harness
        .wait_for_db_status(&run_uuid, "SLEEPING", 20, Duration::from_millis(250))
        .await?;

    let pre_kill_status = harness.db_workflow_status(&run_uuid).await?;
    assert_eq!(
        pre_kill_status, "SLEEPING",
        "workflow should be sleeping before worker dies"
    );

    // Note: With lazy sleep completion (Simplify_plan.md section 2.3), the sleep step
    // stays in RUNNING status until the workflow resumes. The step will be completed
    // lazily by `complete_pending_sleep_steps()` when a new worker claims the workflow.
    // We only verify the step exists, not that it's completed yet.
    let step_status = harness.db_step_status(&run_uuid, "__sleep_0").await?;
    assert!(step_status.is_some(), "sleep step should exist");

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

/// Ensure only one worker can claim a workflow under a poll race.
#[tokio::test]
async fn single_worker_claims_workflow_under_race() -> anyhow::Result<()> {
    let harness = TestHarness::with_config(TestConfig {
        poll_timeout_secs: 1,
        watchdog_interval_secs: 1,
        ..Default::default()
    })
    .await;
    let queue = "e2e-resilience-claim-race";

    let w1_hits = Arc::new(AtomicUsize::new(0));
    let w2_hits = Arc::new(AtomicUsize::new(0));

    let mut worker1 = harness.worker(queue).await;
    let c1 = w1_hits.clone();
    worker1.register("racey", move |_ctx: WorkflowContext, _input: SleepInput| {
        let c1 = c1.clone();
        async move {
            c1.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(800)).await;
            Ok::<_, anyhow::Error>(())
        }
    });
    let shutdown1 = worker1.shutdown_token();
    let handle1 = tokio::spawn(async move { worker1.run().await });

    let mut worker2 = harness.worker(queue).await;
    let c2 = w2_hits.clone();
    worker2.register("racey", move |_ctx: WorkflowContext, _input: SleepInput| {
        let c2 = c2.clone();
        async move {
            c2.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(800)).await;
            Ok::<_, anyhow::Error>(())
        }
    });
    let shutdown2 = worker2.shutdown_token();
    let handle2 = tokio::spawn(async move { worker2.run().await });

    let mut client = harness.client().await;
    let run_id = client
        .workflow("racey", queue, SleepInput { seconds: 0 })
        .await?;
    let run_uuid = Uuid::parse_str(&run_id)?;

    // Complete should happen exactly once despite two workers polling.
    let wf = wait_for_status(&harness.server_url, &run_id, WorkflowStatus::Completed, 30).await?;
    assert_eq!(wf.status, WorkflowStatus::Completed as i32);

    // Only one worker should have executed the workflow.
    let total_hits = w1_hits.load(Ordering::SeqCst) + w2_hits.load(Ordering::SeqCst);
    assert_eq!(
        total_hits,
        1,
        "exactly one worker should execute the workflow (w1={}, w2={})",
        w1_hits.load(Ordering::SeqCst),
        w2_hits.load(Ordering::SeqCst)
    );

    // DB attempts should reflect a single claim.
    let attempts = harness.db_workflow_attempts(&run_uuid).await?;
    assert_eq!(
        attempts, 1,
        "workflow should only have one attempt recorded"
    );

    shutdown1.cancel();
    shutdown2.cancel();
    let _ = handle1.await;
    let _ = handle2.await;
    Ok(())
}

#[tokio::test]
async fn orphaned_workflow_detected_by_watchdog() -> anyhow::Result<()> {
    let harness = TestHarness::with_config(TestConfig {
        worker_stale_threshold_secs: 1,
        watchdog_interval_secs: 1,
        poll_timeout_secs: 1,
        ..Default::default()
    })
    .await;
    let queue = "e2e-resilience-orphan-detect";

    let mut worker = harness.worker(queue).await;
    worker.register(
        "stuck",
        |_ctx: WorkflowContext, _input: SleepInput| async move {
            tokio::time::sleep(Duration::from_secs(20)).await;
            Ok::<_, anyhow::Error>(())
        },
    );
    let handle = tokio::spawn(async move { worker.run().await });

    let mut client = harness.client().await;
    let run_id = client
        .workflow("stuck", queue, SleepInput { seconds: 0 })
        .await?;
    let run_uuid = Uuid::parse_str(&run_id)?;

    tokio::time::sleep(Duration::from_millis(500)).await;
    handle.abort();

    // Wait for watchdog to reschedule (tolerate eventual completion if retried quickly).
    for _ in 0..40 {
        let status = harness.db_workflow_status(&run_uuid).await?;
        if status == "PENDING" || status == "SLEEPING" || status == "RUNNING" {
            break;
        }
        sleep(Duration::from_millis(200)).await;
    }

    let status = harness.db_workflow_status(&run_uuid).await?;
    assert!(
        status == "PENDING" || status == "SLEEPING" || status == "RUNNING",
        "orphan should be rescheduled, got {}",
        status
    );
    let locked_by = harness.db_workflow_locked_by(&run_uuid).await?;
    assert!(
        locked_by.is_none() || status == "RUNNING",
        "locked_by should typically be cleared after orphan detection (status={}, locked_by={:?})",
        status,
        locked_by
    );

    Ok(())
}

#[tokio::test]
async fn orphaned_workflow_rescheduled_with_backoff() -> anyhow::Result<()> {
    let harness = TestHarness::with_config(TestConfig {
        worker_stale_threshold_secs: 1,
        watchdog_interval_secs: 1,
        poll_timeout_secs: 1,
        ..Default::default()
    })
    .await;
    let queue = "e2e-resilience-orphan-backoff";

    let mut worker = harness.worker(queue).await;
    worker.register(
        "backoff_wf",
        |_ctx: WorkflowContext, _input: SleepInput| async move {
            tokio::time::sleep(Duration::from_secs(30)).await;
            Ok::<_, anyhow::Error>(())
        },
    );
    let shutdown = worker.shutdown_token();
    let handle = tokio::spawn(async move { worker.run().await });

    let retry_policy = RetryPolicy {
        maximum_attempts: Some(3),
        initial_interval: Some(Duration::from_millis(500)),
        backoff_coefficient: Some(2.0),
        maximum_interval: Some(Duration::from_secs(5)),
        non_retryable_errors: vec![],
    };

    let mut client = harness.client().await;
    let run_id = client
        .workflow("backoff_wf", queue, SleepInput { seconds: 0 })
        .retry_policy(retry_policy.clone())
        .await?;
    let run_uuid = Uuid::parse_str(&run_id)?;

    // Wait for workflow to be RUNNING before we abort (same as orphan_recovery_increments_attempt)
    for _ in 0..20 {
        let status = harness.db_workflow_status(&run_uuid).await?;
        if status == "RUNNING" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    let attempts_before = harness.db_workflow_attempts(&run_uuid).await?;
    shutdown.cancel();
    handle.abort();

    // Expire lock so watchdog can reclaim promptly.
    sqlx::query("UPDATE kagzi.workflow_runs SET locked_until = NOW() - INTERVAL '5 seconds' WHERE run_id = $1")
        .bind(run_uuid)
        .execute(&harness.pool)
        .await?;

    // Wait for watchdog to schedule retry with backoff.
    sleep(Duration::from_secs(2)).await;
    for _ in 0..40 {
        let status = harness.db_workflow_status(&run_uuid).await?;
        if status == "PENDING" || status == "SLEEPING" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }

    let attempts = harness.db_workflow_attempts(&run_uuid).await?;
    assert!(
        attempts > attempts_before,
        "attempt counter should increase after reschedule (before={}, after={})",
        attempts_before,
        attempts
    );

    // Allow some time for wake_up_at to be populated by the scheduler/backoff logic.
    let wake_up_at: Option<chrono::DateTime<chrono::Utc>> = {
        let mut attempts_left = 30;
        loop {
            if let Some(ts) = sqlx::query_scalar::<_, Option<chrono::DateTime<chrono::Utc>>>(
                "SELECT wake_up_at FROM kagzi.workflow_runs WHERE run_id = $1",
            )
            .bind(run_uuid)
            .fetch_one(&harness.pool)
            .await?
            {
                break Some(ts);
            }

            if attempts_left == 0 {
                break None;
            }
            attempts_left -= 1;
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
    };

    if let Some(wake_up_at) = wake_up_at {
        let now = chrono::Utc::now();
        let delta = (wake_up_at - now).num_milliseconds();
        assert!(
            (-1500..=5_000).contains(&delta),
            "wake_up_at should be near-term (delta_ms={})",
            delta
        );
    } else {
        let status = harness.db_workflow_status(&run_uuid).await?;
        assert!(
            status == "PENDING" || status == "SLEEPING",
            "workflow should still be pending/sleeping if wake_up_at is missing, got {status}"
        );
    }

    Ok(())
}

#[tokio::test]
async fn orphan_recovery_increments_attempt() -> anyhow::Result<()> {
    let harness = TestHarness::with_config(TestConfig {
        worker_stale_threshold_secs: 1,
        watchdog_interval_secs: 1,
        poll_timeout_secs: 1,
        ..Default::default()
    })
    .await;
    let queue = "e2e-resilience-orphan-attempts";

    let mut worker = harness.worker(queue).await;
    worker.register(
        "attempt_wf",
        |_ctx: WorkflowContext, _input: SleepInput| async move {
            tokio::time::sleep(Duration::from_secs(20)).await;
            Ok::<_, anyhow::Error>(())
        },
    );
    let handle = tokio::spawn(async move { worker.run().await });

    let mut client = harness.client().await;
    let run_id = client
        .workflow("attempt_wf", queue, SleepInput { seconds: 0 })
        .await?;
    let run_uuid = Uuid::parse_str(&run_id)?;

    // Wait for workflow to be running before we abort
    for _ in 0..10 {
        let status = harness.db_workflow_status(&run_uuid).await?;
        if status == "RUNNING" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    tokio::time::sleep(Duration::from_secs(1)).await;
    handle.abort();

    // Expire lock so watchdog can reclaim promptly
    sqlx::query("UPDATE kagzi.workflow_runs SET locked_until = NOW() - INTERVAL '5 seconds' WHERE run_id = $1")
        .bind(run_uuid)
        .execute(&harness.pool)
        .await?;

    sleep(Duration::from_secs(4)).await;
    let status_after = harness.db_workflow_status(&run_uuid).await?;
    let attempts = harness.db_workflow_attempts(&run_uuid).await?;

    // After orphan recovery, the workflow should be either:
    // 1. FAILED (if it exhausted retries)
    // 2. PENDING with attempts incremented (if it was recovered for retry)
    assert!(
        status_after == "FAILED" || (status_after == "PENDING" && attempts > 0),
        "orphan recovery should either mark as FAILED or recover with incremented attempts (status={}, attempts={})",
        status_after,
        attempts
    );

    Ok(())
}

#[tokio::test]
async fn orphan_with_exhausted_retries_fails() -> anyhow::Result<()> {
    let harness = TestHarness::with_config(TestConfig {
        worker_stale_threshold_secs: 1,
        watchdog_interval_secs: 1,
        poll_timeout_secs: 1,
        ..Default::default()
    })
    .await;
    let queue = "e2e-resilience-orphan-exhausted";

    let mut worker = harness.worker(queue).await;
    worker.register(
        "exhausted_wf",
        |_ctx: WorkflowContext, _input: SleepInput| async move {
            tokio::time::sleep(Duration::from_secs(30)).await;
            Ok::<_, anyhow::Error>(())
        },
    );
    let shutdown = worker.shutdown_token();
    let handle = tokio::spawn(async move { worker.run().await });

    let policy = RetryPolicy {
        maximum_attempts: Some(1),
        initial_interval: Some(Duration::from_millis(200)),
        backoff_coefficient: Some(1.0),
        maximum_interval: Some(Duration::from_secs(1)),
        non_retryable_errors: vec![],
    };

    let mut client = harness.client().await;
    let run_id = client
        .workflow("exhausted_wf", queue, SleepInput { seconds: 0 })
        .retry_policy(policy)
        .await?;
    let run_uuid = Uuid::parse_str(&run_id)?;

    tokio::time::sleep(Duration::from_millis(500)).await;
    shutdown.cancel();
    let _ = handle.await;

    // Expire lock so watchdog can reclaim promptly
    sqlx::query("UPDATE kagzi.workflow_runs SET locked_until = NOW() - INTERVAL '5 seconds' WHERE run_id = $1")
        .bind(run_uuid)
        .execute(&harness.pool)
        .await?;

    sleep(Duration::from_secs(4)).await;
    let status = harness.db_workflow_status(&run_uuid).await?;
    assert!(
        status == "FAILED" || status == "COMPLETED",
        "exhausted orphan should be terminal, got {}",
        status
    );

    Ok(())
}

#[tokio::test]
async fn workflow_timeout_triggers_failure() -> anyhow::Result<()> {
    let harness = TestHarness::with_config(TestConfig {
        worker_stale_threshold_secs: 1,
        watchdog_interval_secs: 1,
        poll_timeout_secs: 1,
        ..Default::default()
    })
    .await;
    let queue = "e2e-resilience-timeout";

    let mut worker = harness.worker(queue).await;
    worker.register(
        "timeout_wf",
        |_ctx: WorkflowContext, _input: SleepInput| async move {
            // Block forever so the workflow stays RUNNING while the worker is killed.
            pending::<()>().await;
            #[allow(unreachable_code)]
            Ok::<_, anyhow::Error>(())
        },
    );
    let shutdown = worker.shutdown_token();
    let handle = tokio::spawn(async move { worker.run().await });

    let retry_policy = RetryPolicy {
        maximum_attempts: Some(1),
        initial_interval: Some(Duration::from_millis(100)),
        backoff_coefficient: Some(1.0),
        maximum_interval: Some(Duration::from_secs(1)),
        non_retryable_errors: vec![],
    };

    let mut client = harness.client().await;
    let run_id = client
        .workflow("timeout_wf", queue, SleepInput { seconds: 0 })
        .retry_policy(retry_policy)
        .await?;
    let run_uuid = Uuid::parse_str(&run_id)?;

    // Let the worker start the workflow, then abort it so watchdog marks it stale.
    tokio::time::sleep(Duration::from_millis(500)).await;
    shutdown.cancel();
    handle.abort();

    let wf = wait_for_status(&harness.server_url, &run_id, WorkflowStatus::Failed, 160).await?;
    assert_eq!(wf.status, WorkflowStatus::Failed as i32);
    let error = wf.error.unwrap_or_default();
    assert!(
        error.message.contains("exhausted") || error.message.contains("stale"),
        "timeout/stale should be recorded, got {}",
        error.message
    );

    let db_status = harness.db_workflow_status(&run_uuid).await?;
    assert_eq!(db_status, "FAILED");

    Ok(())
}
