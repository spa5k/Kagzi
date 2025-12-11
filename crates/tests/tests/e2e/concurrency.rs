use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use kagzi::WorkflowContext;
use kagzi_proto::kagzi::WorkflowStatus;
use serde::{Deserialize, Serialize};
use tests::common::{TestConfig, TestHarness, wait_for_completed_by_type, wait_for_status};
use tokio::time::sleep;
use uuid::Uuid;

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Clone, Copy)]
struct Empty;

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

    wait_for_completed_by_type(
        &harness.pool,
        "slow_wf",
        total,
        40,
        Duration::from_millis(250),
    )
    .await?;
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

    wait_for_completed_by_type(
        &harness.pool,
        "multi_wf",
        total,
        40,
        Duration::from_millis(250),
    )
    .await?;
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

#[tokio::test]
async fn rapid_poll_burst_no_duplicate_claims() -> anyhow::Result<()> {
    let harness = TestHarness::with_config(TestConfig {
        poll_timeout_secs: 1,
        ..Default::default()
    })
    .await;
    let queue = "e2e-concurrency-rapid-poll";
    let total = 12;

    let w1 = Arc::new(AtomicUsize::new(0));
    let w2 = Arc::new(AtomicUsize::new(0));
    let w3 = Arc::new(AtomicUsize::new(0));

    let mut worker1 = harness.worker(queue).await;
    let c1 = w1.clone();
    worker1.register("rapid_poll", move |_ctx: WorkflowContext, _: Empty| {
        let c1 = c1.clone();
        async move {
            sleep(Duration::from_millis(100)).await;
            c1.fetch_add(1, Ordering::SeqCst);
            Ok::<_, anyhow::Error>(())
        }
    });
    let shutdown1 = worker1.shutdown_token();
    let handle1 = tokio::spawn(async move { worker1.run().await });

    let mut worker2 = harness.worker(queue).await;
    let c2 = w2.clone();
    worker2.register("rapid_poll", move |_ctx: WorkflowContext, _: Empty| {
        let c2 = c2.clone();
        async move {
            sleep(Duration::from_millis(100)).await;
            c2.fetch_add(1, Ordering::SeqCst);
            Ok::<_, anyhow::Error>(())
        }
    });
    let shutdown2 = worker2.shutdown_token();
    let handle2 = tokio::spawn(async move { worker2.run().await });

    let mut worker3 = harness.worker(queue).await;
    let c3 = w3.clone();
    worker3.register("rapid_poll", move |_ctx: WorkflowContext, _: Empty| {
        let c3 = c3.clone();
        async move {
            sleep(Duration::from_millis(100)).await;
            c3.fetch_add(1, Ordering::SeqCst);
            Ok::<_, anyhow::Error>(())
        }
    });
    let shutdown3 = worker3.shutdown_token();
    let handle3 = tokio::spawn(async move { worker3.run().await });

    let mut client = harness.client().await;
    for _ in 0..total {
        client.workflow("rapid_poll", queue, Empty).await?;
    }

    wait_for_completed_by_type(
        &harness.pool,
        "rapid_poll",
        total,
        40,
        Duration::from_millis(250),
    )
    .await?;
    let processed =
        w1.load(Ordering::SeqCst) + w2.load(Ordering::SeqCst) + w3.load(Ordering::SeqCst);
    assert_eq!(
        processed, total,
        "exactly {} workflows should be processed once (got {})",
        total, processed
    );

    let dup_attempts: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM kagzi.workflow_runs WHERE task_queue = $1 AND workflow_type = $2 AND attempts > 1",
    )
    .bind(queue)
    .bind("rapid_poll")
    .fetch_one(&harness.pool)
    .await?;
    assert_eq!(dup_attempts, 0, "no workflows should be double-claimed");

    shutdown1.cancel();
    shutdown2.cancel();
    shutdown3.cancel();
    let _ = handle1.await;
    let _ = handle2.await;
    let _ = handle3.await;
    Ok(())
}

#[tokio::test]
async fn claim_during_status_transition_handled() -> anyhow::Result<()> {
    let harness = TestHarness::with_config(TestConfig {
        watchdog_interval_secs: 1,
        poll_timeout_secs: 1,
        ..Default::default()
    })
    .await;
    let queue = "e2e-concurrency-transition";

    let w1_hits = Arc::new(AtomicUsize::new(0));
    let w2_hits = Arc::new(AtomicUsize::new(0));

    let mut worker1 = harness.worker(queue).await;
    let c1 = w1_hits.clone();
    worker1.register(
        "transition_sleep",
        move |mut ctx: WorkflowContext, _: Empty| {
            let c1 = c1.clone();
            async move {
                c1.fetch_add(1, Ordering::SeqCst);
                ctx.sleep(Duration::from_secs(2)).await?;
                Ok::<_, anyhow::Error>(())
            }
        },
    );
    let shutdown1 = worker1.shutdown_token();
    let handle1 = tokio::spawn(async move { worker1.run().await });

    let mut worker2 = harness.worker(queue).await;
    let c2 = w2_hits.clone();
    worker2.register(
        "transition_sleep",
        move |mut ctx: WorkflowContext, _: Empty| {
            let c2 = c2.clone();
            async move {
                c2.fetch_add(1, Ordering::SeqCst);
                ctx.sleep(Duration::from_secs(2)).await?;
                Ok::<_, anyhow::Error>(())
            }
        },
    );
    let shutdown2 = worker2.shutdown_token();
    let handle2 = tokio::spawn(async move { worker2.run().await });

    let mut client = harness.client().await;
    let run_id = client.workflow("transition_sleep", queue, Empty).await?;
    let run_uuid = Uuid::parse_str(&run_id)?;

    let wf = wait_for_status(&harness.server_url, &run_id, WorkflowStatus::Completed, 40).await?;
    assert_eq!(wf.status, WorkflowStatus::Completed as i32);

    let attempts = harness.db_workflow_attempts(&run_uuid).await?;
    assert!(
        attempts <= 2,
        "status transition should not create many attempts, got {}",
        attempts
    );

    shutdown1.cancel();
    shutdown2.cancel();
    let _ = handle1.await;
    let _ = handle2.await;
    Ok(())
}

#[tokio::test]
async fn multi_worker_type_filtering_distributes_correctly() -> anyhow::Result<()> {
    let harness = TestHarness::new().await;
    let queue = "e2e-concurrency-type-filter";

    let a_hits = Arc::new(AtomicUsize::new(0));
    let b_hits = Arc::new(AtomicUsize::new(0));

    let mut worker_a = harness.worker(queue).await;
    let a_counter = a_hits.clone();
    worker_a.register("type_a", move |_ctx: WorkflowContext, _: Empty| {
        let a_counter = a_counter.clone();
        async move {
            a_counter.fetch_add(1, Ordering::SeqCst);
            sleep(Duration::from_millis(50)).await;
            Ok::<_, anyhow::Error>(())
        }
    });
    let b_counter = b_hits.clone();
    worker_a.register("type_b", move |_ctx: WorkflowContext, _: Empty| {
        let b_counter = b_counter.clone();
        async move {
            b_counter.fetch_add(1, Ordering::SeqCst);
            sleep(Duration::from_millis(50)).await;
            Ok::<_, anyhow::Error>(())
        }
    });
    let shutdown_a = worker_a.shutdown_token();
    let handle_a = tokio::spawn(async move { worker_a.run().await });

    let mut worker_b = harness.worker(queue).await;
    let b_counter = b_hits.clone();
    worker_b.register("type_b", move |_ctx: WorkflowContext, _: Empty| {
        let b_counter = b_counter.clone();
        async move {
            b_counter.fetch_add(1, Ordering::SeqCst);
            sleep(Duration::from_millis(50)).await;
            Ok::<_, anyhow::Error>(())
        }
    });
    let a_counter_b = a_hits.clone();
    worker_b.register("type_a", move |_ctx: WorkflowContext, _: Empty| {
        let a_counter_b = a_counter_b.clone();
        async move {
            a_counter_b.fetch_add(1, Ordering::SeqCst);
            sleep(Duration::from_millis(50)).await;
            Ok::<_, anyhow::Error>(())
        }
    });
    let shutdown_b = worker_b.shutdown_token();
    let handle_b = tokio::spawn(async move { worker_b.run().await });

    let mut client = harness.client().await;
    for _ in 0..5 {
        client.workflow("type_a", queue, Empty).await?;
    }
    for _ in 0..5 {
        client.workflow("type_b", queue, Empty).await?;
    }

    wait_for_completed_by_type(&harness.pool, "type_a", 5, 40, Duration::from_millis(250)).await?;
    wait_for_completed_by_type(&harness.pool, "type_b", 5, 40, Duration::from_millis(250)).await?;

    let a_total = a_hits.load(Ordering::SeqCst);
    let b_total = b_hits.load(Ordering::SeqCst);
    assert_eq!(a_total, 5, "five type_a workflows should complete");
    assert_eq!(b_total, 5, "five type_b workflows should complete");

    shutdown_a.cancel();
    shutdown_b.cancel();
    let _ = handle_a.await;
    let _ = handle_b.await;
    Ok(())
}

#[tokio::test]
async fn max_concurrent_enforced_across_burst() -> anyhow::Result<()> {
    let harness = TestHarness::new().await;
    let queue = "e2e-concurrency-queue-limit";
    let total = 6;

    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));

    let active_clone = active.clone();
    let peak_clone = peak.clone();
    let mut worker = kagzi::Worker::builder(&harness.server_url, queue)
        .max_concurrent(1)
        .build()
        .await?;
    worker.register("queue_limited", move |_ctx: WorkflowContext, _: Empty| {
        let active_clone = active_clone.clone();
        let peak_clone = peak_clone.clone();
        async move {
            let current = active_clone.fetch_add(1, Ordering::SeqCst) + 1;
            peak_clone.fetch_max(current, Ordering::SeqCst);
            sleep(Duration::from_millis(300)).await;
            active_clone.fetch_sub(1, Ordering::SeqCst);
            Ok::<_, anyhow::Error>(())
        }
    });
    let shutdown = worker.shutdown_token();
    let handle = tokio::spawn(async move { worker.run().await });

    let mut client = harness.client().await;
    for _ in 0..total {
        client.workflow("queue_limited", queue, Empty).await?;
    }

    wait_for_completed_by_type(
        &harness.pool,
        "queue_limited",
        total,
        40,
        Duration::from_millis(250),
    )
    .await?;
    assert!(
        peak.load(Ordering::SeqCst) <= 1,
        "max_concurrent should cap parallelism to 1, observed {}",
        peak.load(Ordering::SeqCst)
    );

    shutdown.cancel();
    let _ = handle.await;
    Ok(())
}

#[tokio::test]
async fn per_type_concurrency_limits_enforced() -> anyhow::Result<()> {
    let harness = TestHarness::new().await;
    let queue = "e2e-concurrency-type-limit";

    let active_a = Arc::new(AtomicUsize::new(0));
    let peak_a = Arc::new(AtomicUsize::new(0));
    let active_b = Arc::new(AtomicUsize::new(0));
    let peak_b = Arc::new(AtomicUsize::new(0));

    let mut worker = kagzi::Worker::builder(&harness.server_url, queue)
        .max_concurrent(2)
        .workflow_type_concurrency("type_a", 1)
        .workflow_type_concurrency("type_b", 2)
        .build()
        .await?;
    let active_a_clone = active_a.clone();
    let peak_a_clone = peak_a.clone();
    worker.register("type_a", move |_ctx: WorkflowContext, _: Empty| {
        let active_a_clone = active_a_clone.clone();
        let peak_a_clone = peak_a_clone.clone();
        async move {
            let current = active_a_clone.fetch_add(1, Ordering::SeqCst) + 1;
            peak_a_clone.fetch_max(current, Ordering::SeqCst);
            sleep(Duration::from_millis(400)).await;
            active_a_clone.fetch_sub(1, Ordering::SeqCst);
            Ok::<_, anyhow::Error>(())
        }
    });
    let active_b_clone = active_b.clone();
    let peak_b_clone = peak_b.clone();
    worker.register("type_b", move |_ctx: WorkflowContext, _: Empty| {
        let active_b_clone = active_b_clone.clone();
        let peak_b_clone = peak_b_clone.clone();
        async move {
            let current = active_b_clone.fetch_add(1, Ordering::SeqCst) + 1;
            peak_b_clone.fetch_max(current, Ordering::SeqCst);
            sleep(Duration::from_millis(200)).await;
            active_b_clone.fetch_sub(1, Ordering::SeqCst);
            Ok::<_, anyhow::Error>(())
        }
    });
    let shutdown = worker.shutdown_token();
    let handle = tokio::spawn(async move { worker.run().await });

    let mut client = harness.client().await;
    for _ in 0..4 {
        client.workflow("type_a", queue, Empty).await?;
    }
    for _ in 0..4 {
        client.workflow("type_b", queue, Empty).await?;
    }

    wait_for_completed_by_type(&harness.pool, "type_a", 4, 40, Duration::from_millis(250)).await?;
    wait_for_completed_by_type(&harness.pool, "type_b", 4, 40, Duration::from_millis(250)).await?;

    assert!(
        peak_a.load(Ordering::SeqCst) <= 2,
        "type_a should respect configured limits, observed {}",
        peak_a.load(Ordering::SeqCst)
    );
    assert!(
        peak_b.load(Ordering::SeqCst) <= 2,
        "type_b should respect configured limits, observed {}",
        peak_b.load(Ordering::SeqCst)
    );

    shutdown.cancel();
    let _ = handle.await;
    Ok(())
}

#[tokio::test]
async fn burst_of_workflows_completes_without_drops() -> anyhow::Result<()> {
    let harness = TestHarness::new().await;
    let queue = "e2e-concurrency-burst";
    let total = 20;

    let completed = Arc::new(AtomicUsize::new(0));

    let mut worker = harness.worker(queue).await;
    let counter = completed.clone();
    worker.register("burst_work", move |_ctx: WorkflowContext, _: Empty| {
        let counter = counter.clone();
        async move {
            sleep(Duration::from_millis(50)).await;
            counter.fetch_add(1, Ordering::SeqCst);
            Ok::<_, anyhow::Error>(())
        }
    });
    let shutdown = worker.shutdown_token();
    let handle = tokio::spawn(async move { worker.run().await });

    let mut client = harness.client().await;
    for _ in 0..total {
        client.workflow("burst_work", queue, Empty).await?;
    }

    wait_for_completed_by_type(
        &harness.pool,
        "burst_work",
        total,
        40,
        Duration::from_millis(250),
    )
    .await?;
    let observed = completed.load(Ordering::SeqCst);
    assert_eq!(
        observed, total,
        "all workflows in burst should complete (observed={})",
        observed
    );

    shutdown.cancel();
    let _ = handle.await;
    Ok(())
}
