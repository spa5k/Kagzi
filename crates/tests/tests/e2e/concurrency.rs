#[path = "../common/mod.rs"]
mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use common::TestHarness;
use kagzi::WorkflowContext;
use serde::{Deserialize, Serialize};
use tokio::time::sleep;

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Clone, Copy)]
struct Empty;

async fn wait_completed(
    pool: &sqlx::PgPool,
    workflow_type: &str,
    expected: usize,
) -> anyhow::Result<()> {
    let mut attempts = 0;
    loop {
        let done: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM kagzi.workflow_runs WHERE workflow_type = $1 AND status = 'COMPLETED'",
        )
        .bind(workflow_type)
        .fetch_one(pool)
        .await?;
        if done as usize >= expected {
            return Ok(());
        }
        if attempts > 40 {
            anyhow::bail!(
                "timed out waiting for {} completed runs of {} (got {})",
                expected,
                workflow_type,
                done
            );
        }
        attempts += 1;
        sleep(Duration::from_millis(250)).await;
    }
}

#[tokio::test]
async fn worker_respects_max_concurrent() -> anyhow::Result<()> {
    let harness = TestHarness::new().await;
    let queue = "e2e-concurrency-cap";
    let total = 8;

    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));

    let active_clone = active.clone();
    let peak_clone = peak.clone();

    let mut worker = kagzi::Worker::builder(&harness.server_url, queue)
        .max_concurrent(2)
        .build()
        .await?;
    worker.register("slow_wf", move |_ctx: WorkflowContext, _: Empty| {
        let active = active_clone.clone();
        let peak = peak_clone.clone();
        async move {
            let current = active.fetch_add(1, Ordering::SeqCst) + 1;
            peak.fetch_max(current, Ordering::SeqCst);
            sleep(Duration::from_millis(500)).await;
            active.fetch_sub(1, Ordering::SeqCst);
            Ok::<_, anyhow::Error>(())
        }
    });
    let shutdown = worker.shutdown_token();
    let handle = tokio::spawn(async move { worker.run().await });

    let mut client = harness.client().await;
    for _ in 0..total {
        client.workflow("slow_wf", queue, Empty).await?;
    }

    wait_completed(&harness.pool, "slow_wf", total).await?;
    let completed = harness.db_count_workflows_by_status("COMPLETED").await?;
    assert!(
        completed as usize >= total,
        "db helper should reflect completed runs; got {} expected >= {}",
        completed,
        total
    );
    let observed = peak.load(Ordering::SeqCst);
    assert!(
        observed <= 2,
        "max_concurrent should cap parallelism to 2, observed {}",
        observed
    );

    shutdown.cancel();
    let _ = handle.await;
    Ok(())
}

#[tokio::test]
async fn multiple_workers_share_queue() -> anyhow::Result<()> {
    let harness = TestHarness::new().await;
    let queue = "e2e-concurrency-multi";
    let total = 6;

    let w1_count = Arc::new(AtomicUsize::new(0));
    let w2_count = Arc::new(AtomicUsize::new(0));

    let mut worker1 = harness.worker(queue).await;
    let c1 = w1_count.clone();
    worker1.register("multi_wf", move |_ctx: WorkflowContext, _: Empty| {
        let c1 = c1.clone();
        async move {
            c1.fetch_add(1, Ordering::SeqCst);
            sleep(Duration::from_millis(200)).await;
            Ok::<_, anyhow::Error>(())
        }
    });
    let shutdown1 = worker1.shutdown_token();
    let handle1 = tokio::spawn(async move { worker1.run().await });

    let mut worker2 = harness.worker(queue).await;
    let c2 = w2_count.clone();
    worker2.register("multi_wf", move |_ctx: WorkflowContext, _: Empty| {
        let c2 = c2.clone();
        async move {
            c2.fetch_add(1, Ordering::SeqCst);
            sleep(Duration::from_millis(200)).await;
            Ok::<_, anyhow::Error>(())
        }
    });
    let shutdown2 = worker2.shutdown_token();
    let handle2 = tokio::spawn(async move { worker2.run().await });

    let mut client = harness.client().await;
    for _ in 0..total {
        client.workflow("multi_wf", queue, Empty).await?;
    }

    wait_completed(&harness.pool, "multi_wf", total).await?;
    let completed = harness.db_count_workflows_by_status("COMPLETED").await?;
    assert!(
        completed as usize >= total,
        "db helper should reflect completed runs; got {} expected >= {}",
        completed,
        total
    );

    let c1_total = w1_count.load(Ordering::SeqCst);
    let c2_total = w2_count.load(Ordering::SeqCst);
    assert_eq!(
        c1_total + c2_total,
        total,
        "all workflows should be processed (c1={}, c2={})",
        c1_total,
        c2_total
    );

    shutdown1.cancel();
    shutdown2.cancel();
    let _ = handle1.await;
    let _ = handle2.await;
    Ok(())
}
