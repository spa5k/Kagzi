use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use kagzi::Context;
use kagzi_proto::kagzi::worker_service_client::WorkerServiceClient;
use kagzi_proto::kagzi::{DeregisterRequest, PollTaskRequest, RegisterRequest, WorkflowStatus};
use serde::{Deserialize, Serialize};
use tests::common::{TestConfig, TestHarness, wait_for_status};
use tokio::time::sleep;
use tonic::{Code, Request};
use uuid::Uuid;

const WORKER_RETRY_COUNT: usize = 20;
const WORKER_RETRY_DELAY: Duration = Duration::from_millis(100);

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Clone, Copy)]
struct Empty;

async fn register_worker(
    server_url: &str,
    queue: &str,
    hostname: &str,
    pid: i32,
    workflow_types: Vec<String>,
) -> anyhow::Result<String> {
    let mut client = WorkerServiceClient::connect(server_url.to_string()).await?;
    let resp = client
        .register(Request::new(RegisterRequest {
            namespace_id: "default".to_string(),
            task_queue: queue.to_string(),
            hostname: hostname.to_string(),
            pid,
            version: "test".to_string(),
            workflow_types,
            max_concurrent: 2,
            labels: Default::default(),
            queue_concurrency_limit: None,
            workflow_type_concurrency: vec![],
        }))
        .await?
        .into_inner();
    Ok(resp.worker_id)
}

async fn wait_for_worker_row(pool: &sqlx::PgPool, queue: &str) -> anyhow::Result<Uuid> {
    for _ in 0..WORKER_RETRY_COUNT {
        if let Some(id) = sqlx::query_scalar::<_, Option<Uuid>>(
            "SELECT worker_id FROM kagzi.workers WHERE task_queue = $1 ORDER BY registered_at DESC LIMIT 1",
        )
        .bind(queue)
        .fetch_optional(pool)
        .await?
        .flatten()
        {
            return Ok(id);
        }
        sleep(WORKER_RETRY_DELAY).await;
    }
    anyhow::bail!("worker for queue {} not visible in DB", queue)
}

async fn wait_for_worker_count(
    pool: &sqlx::PgPool,
    queue: &str,
    expected: i64,
) -> anyhow::Result<()> {
    for _ in 0..WORKER_RETRY_COUNT {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM kagzi.workers WHERE task_queue = $1 AND status != 'OFFLINE'",
        )
        .bind(queue)
        .fetch_one(pool)
        .await?;
        if count >= expected {
            return Ok(());
        }
        sleep(WORKER_RETRY_DELAY).await;
    }
    anyhow::bail!("expected {} workers online for queue {}", expected, queue)
}

#[tokio::test]
async fn worker_registration_returns_unique_id() -> anyhow::Result<()> {
    let harness = TestHarness::new().await;
    let queue = "e2e-worker-registration-unique";

    let first = register_worker(
        &harness.server_url,
        queue,
        "host-unique-1",
        1001,
        vec!["unique_wf".to_string()],
    )
    .await?;
    let second = register_worker(
        &harness.server_url,
        queue,
        "host-unique-2",
        1002,
        vec!["unique_wf".to_string()],
    )
    .await?;

    let first_id = Uuid::parse_str(&first)?;
    let second_id = Uuid::parse_str(&second)?;
    assert_ne!(first_id, second_id, "worker IDs must be unique");

    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM kagzi.workers WHERE task_queue = $1 AND status = 'ONLINE'",
    )
    .bind(queue)
    .fetch_one(&harness.pool)
    .await?;
    assert_eq!(count, 2, "two distinct workers should be registered");
    Ok(())
}

#[tokio::test]
async fn worker_reregistration_same_queue_updates_existing() -> anyhow::Result<()> {
    let harness = TestHarness::new().await;
    let queue = "e2e-worker-reregistration";

    let first = register_worker(
        &harness.server_url,
        queue,
        "host-rereg",
        4242,
        vec!["rerun".to_string()],
    )
    .await?;
    let second = register_worker(
        &harness.server_url,
        queue,
        "host-rereg",
        4242,
        vec!["rerun".to_string()],
    )
    .await?;

    assert_eq!(
        first, second,
        "re-registering the same worker should return the same worker_id"
    );

    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM kagzi.workers WHERE task_queue = $1 AND hostname = $2 AND pid = $3",
    )
    .bind(queue)
    .bind("host-rereg")
    .bind(4242)
    .fetch_one(&harness.pool)
    .await?;
    assert_eq!(count, 1, "re-registration should not create a second row");
    Ok(())
}

#[tokio::test]
async fn worker_deregistration_clears_from_pool() -> anyhow::Result<()> {
    let harness = TestHarness::new().await;
    let queue = "e2e-worker-dereg";

    let worker_id = register_worker(
        &harness.server_url,
        queue,
        "host-dereg",
        5151,
        vec!["noop".to_string()],
    )
    .await?;
    let worker_uuid = Uuid::parse_str(&worker_id)?;

    let mut client = WorkerServiceClient::connect(harness.server_url.clone()).await?;
    client
        .deregister(Request::new(DeregisterRequest {
            worker_id: worker_id.clone(),
            drain: false,
        }))
        .await?;

    let status = harness.db_worker_status(&worker_uuid).await?;
    assert_eq!(status, "OFFLINE");

    let active: i32 =
        sqlx::query_scalar("SELECT active_count FROM kagzi.workers WHERE worker_id = $1")
            .bind(worker_uuid)
            .fetch_one(&harness.pool)
            .await?;
    assert_eq!(active, 0, "deregistration should clear active_count");

    let poll_err = client
        .poll_task(Request::new(PollTaskRequest {
            task_queue: queue.to_string(),
            worker_id: worker_id.clone(),
            namespace_id: "default".to_string(),
            workflow_types: vec!["noop".to_string()],
        }))
        .await
        .expect_err("polling after deregistration should fail");
    assert_eq!(poll_err.code(), Code::FailedPrecondition);
    Ok(())
}

#[tokio::test]
async fn worker_drain_mode_completes_active_before_stopping() -> anyhow::Result<()> {
    let harness = TestHarness::with_config(TestConfig {
        watchdog_interval_secs: 1,
        poll_timeout_secs: 1,
        ..Default::default()
    })
    .await;
    let queue = "e2e-worker-drain";

    let mut worker = kagzi::Worker::builder(&harness.server_url, queue)
        .max_concurrent(1)
        .build()
        .await?;
    worker.workflows([("slow", |_ctx: Context, _input: Empty| async move {
        sleep(Duration::from_millis(800)).await;
        Ok::<_, anyhow::Error>(())
    })]);
    let shutdown = worker.shutdown_token();
    let handle = tokio::spawn(async move { worker.run().await });

    let mut client = harness.client().await;
    let run1 = client
        .start("slow")
        .namespace(queue)
        .input(Empty)
        .r#await()
        .await?
        .await?;
    let run2 = client
        .start("slow")
        .namespace(queue)
        .input(Empty)
        .r#await()
        .await?
        .await?;
    let run1_uuid = Uuid::parse_str(&run1)?;
    let run2_uuid = Uuid::parse_str(&run2)?;

    // Wait until the worker has claimed the first workflow.
    harness
        .wait_for_db_status(&run1_uuid, "RUNNING", 30, Duration::from_millis(100))
        .await?;

    // Fetch the worker_id from the DB to issue a drain request.
    let worker_id = wait_for_worker_row(&harness.pool, queue).await?;

    let mut worker_client = WorkerServiceClient::connect(harness.server_url.clone()).await?;
    worker_client
        .deregister(Request::new(DeregisterRequest {
            worker_id: worker_id.to_string(),
            drain: true,
        }))
        .await?;

    // The in-flight workflow should finish.
    wait_for_status(
        &harness.server_url,
        &run1,
        ProtoWorkflowStatus::Completed,
        40,
    )
    .await?;

    // The second workflow should remain pending because the worker drained.
    harness
        .wait_for_db_status(&run2_uuid, "PENDING", 20, Duration::from_millis(150))
        .await?;

    shutdown.cancel();
    let _ = handle.await;
    Ok(())
}

#[tokio::test]
async fn worker_offline_after_missed_heartbeats() -> anyhow::Result<()> {
    let harness = TestHarness::with_config(TestConfig {
        worker_stale_threshold_secs: 1,
        watchdog_interval_secs: 1,
        ..Default::default()
    })
    .await;
    let queue = "e2e-worker-heartbeat-missed";

    let mut worker = harness.worker(queue).await;
    worker.workflows([("noop", |_ctx: Context, _input: Empty| async move {
        Ok::<_, anyhow::Error>(())
    })]);
    let shutdown = worker.shutdown_token();
    let handle = tokio::spawn(async move { worker.run().await });

    // Wait for registration to land.
    let worker_id = wait_for_worker_row(&harness.pool, queue).await?;

    shutdown.cancel();
    let _ = handle.await;

    // Allow watchdog to mark the worker offline after missed heartbeats.
    for _ in 0..10 {
        let status = harness.db_worker_status(&worker_id).await?;
        if status == "OFFLINE" {
            return Ok(());
        }
        sleep(Duration::from_millis(300)).await;
    }
    anyhow::bail!("worker was not marked offline after missed heartbeats");
}

#[tokio::test]
async fn worker_heartbeat_extends_active_status() -> anyhow::Result<()> {
    let harness = TestHarness::with_config(TestConfig {
        worker_stale_threshold_secs: 1,
        watchdog_interval_secs: 1,
        ..Default::default()
    })
    .await;
    let queue = "e2e-worker-heartbeat-active";

    let mut worker = harness.worker(queue).await;
    worker.workflows([("noop", |_ctx: Context, _input: Empty| async move {
        Ok::<_, anyhow::Error>(())
    })]);
    let shutdown = worker.shutdown_token();
    let handle = tokio::spawn(async move { worker.run().await });

    let worker_id = wait_for_worker_row(&harness.pool, queue).await?;

    sleep(Duration::from_secs(3)).await;
    let status = harness.db_worker_status(&worker_id).await?;
    assert_eq!(
        status, "ONLINE",
        "regular heartbeats should keep worker online"
    );

    shutdown.cancel();
    let _ = handle.await;
    Ok(())
}

#[tokio::test]
async fn worker_heartbeat_updates_active_count() -> anyhow::Result<()> {
    let harness = TestHarness::with_config(TestConfig {
        worker_stale_threshold_secs: 5,
        watchdog_interval_secs: 1,
        ..Default::default()
    })
    .await;
    let queue = "e2e-worker-active-count";

    let mut worker = kagzi::Worker::builder(&harness.server_url, queue)
        .max_concurrent(2)
        .build()
        .await?;
    worker.register("slow_active", |_ctx: Context, _input: Empty| async move {
        sleep(Duration::from_millis(800)).await;
        Ok::<_, anyhow::Error>(())
    });
    let shutdown = worker.shutdown_token();
    let handle = tokio::spawn(async move { worker.run().await });

    let mut client = harness.client().await;
    let run_a = client
        .start("slow_active")
        .namespace(queue)
        .input(Empty)
        .r#await()
        .await?
        .await?;
    let run_b = client
        .start("slow_active")
        .namespace(queue)
        .input(Empty)
        .r#await()
        .await?
        .await?;
    let run_a_uuid = Uuid::parse_str(&run_a)?;
    let run_b_uuid = Uuid::parse_str(&run_b)?;

    harness
        .wait_for_db_status(&run_a_uuid, "RUNNING", 30, Duration::from_millis(100))
        .await?;
    harness
        .wait_for_db_status(&run_b_uuid, "RUNNING", 30, Duration::from_millis(100))
        .await?;

    // Wait for heartbeat to publish active_count=2.
    let worker_id = wait_for_worker_row(&harness.pool, queue).await?;

    let mut observed_two = false;
    for _ in 0..10 {
        let active: i32 =
            sqlx::query_scalar("SELECT active_count FROM kagzi.workers WHERE worker_id = $1")
                .bind(worker_id)
                .fetch_one(&harness.pool)
                .await?;
        if active >= 2 {
            observed_two = true;
            break;
        }
        sleep(Duration::from_millis(150)).await;
    }
    assert!(
        observed_two,
        "active_count should reflect running workflows"
    );

    wait_for_status(
        &harness.server_url,
        &run_a,
        ProtoWorkflowStatus::Completed,
        40,
    )
    .await?;
    wait_for_status(
        &harness.server_url,
        &run_b,
        ProtoWorkflowStatus::Completed,
        40,
    )
    .await?;

    shutdown.cancel();
    let _ = handle.await;
    Ok(())
}

#[tokio::test]
async fn stale_worker_detected_by_watchdog() -> anyhow::Result<()> {
    let harness = TestHarness::with_config(TestConfig {
        worker_stale_threshold_secs: 1,
        watchdog_interval_secs: 1,
        ..Default::default()
    })
    .await;
    let queue = "e2e-worker-stale-detect";

    let mut worker = kagzi::Worker::builder(&harness.server_url, queue)
        .max_concurrent(1)
        .build()
        .await?;
    worker.workflows([("noop", |_ctx: Context, _input: Empty| async move {
        Ok::<_, anyhow::Error>(())
    })]);
    let shutdown = worker.shutdown_token();
    let handle = tokio::spawn(async move { worker.run().await });

    let worker_id = wait_for_worker_row(&harness.pool, queue).await?;

    shutdown.cancel();
    let _ = handle.await;

    for _ in 0..10 {
        let status = harness.db_worker_status(&worker_id).await?;
        if status == "OFFLINE" {
            return Ok(());
        }
        sleep(Duration::from_millis(300)).await;
    }
    anyhow::bail!("watchdog did not mark worker offline");
}

#[tokio::test]
async fn stale_worker_workflows_rescheduled() -> anyhow::Result<()> {
    let harness = TestHarness::with_config(TestConfig {
        worker_stale_threshold_secs: 1,
        watchdog_interval_secs: 1,
        poll_timeout_secs: 1,
        ..Default::default()
    })
    .await;
    let queue = "e2e-worker-stale-reschedule";

    let mut worker = harness.worker(queue).await;
    worker.register("long_running", |_ctx: Context, _input: Empty| async move {
        sleep(Duration::from_secs(10)).await;
        Ok::<_, anyhow::Error>(())
    });
    let shutdown = worker.shutdown_token();
    let handle = tokio::spawn(async move { worker.run().await });

    let mut client = harness.client().await;
    let run_id = client
        .start("long_running")
        .namespace(queue)
        .input(Empty)
        .r#await()
        .await?
        .await?;
    let run_uuid = Uuid::parse_str(&run_id)?;

    harness
        .wait_for_db_status(&run_uuid, "RUNNING", 30, Duration::from_millis(150))
        .await?;

    // Stop the worker abruptly so heartbeats cease and the running task is abandoned.
    shutdown.cancel();
    handle.abort();

    // Expire the lock to accelerate orphan detection.
    sqlx::query("UPDATE kagzi.workflow_runs SET locked_until = NOW() - INTERVAL '5 seconds' WHERE run_id = $1")
        .bind(run_uuid)
        .execute(&harness.pool)
        .await?;

    // Wait for orphan recovery to reschedule the workflow.
    harness
        .wait_for_db_status(&run_uuid, "PENDING", 40, Duration::from_millis(300))
        .await?;

    // Bring up a new worker to finish the workflow.
    let mut worker2 = harness.worker(queue).await;
    worker2.register("long_running", |_ctx: Context, _input: Empty| async move {
        Ok::<_, anyhow::Error>(())
    });
    let shutdown2 = worker2.shutdown_token();
    let handle2 = tokio::spawn(async move { worker2.run().await });

    wait_for_status(
        &harness.server_url,
        &run_id,
        ProtoWorkflowStatus::Completed,
        40,
    )
    .await?;

    shutdown2.cancel();
    let _ = handle2.await;
    Ok(())
}

#[tokio::test]
async fn worker_with_different_workflow_types_only_receives_matching() -> anyhow::Result<()> {
    let harness = TestHarness::new().await;
    let queue = "e2e-worker-type-filter";

    let a_count = Arc::new(AtomicUsize::new(0));
    let b_count = Arc::new(AtomicUsize::new(0));

    let mut worker_a = kagzi::Worker::builder(&harness.server_url, queue)
        .hostname("host-alpha")
        .build()
        .await?;
    let a_counter = a_count.clone();
    worker_a.register("alpha", move |_ctx: Context, _input: Empty| {
        let a_counter = a_counter.clone();
        async move {
            a_counter.fetch_add(1, Ordering::SeqCst);
            Ok::<_, anyhow::Error>(())
        }
    });
    let shutdown_a = worker_a.shutdown_token();
    let handle_a = tokio::spawn(async move { worker_a.run().await });

    let mut worker_b = kagzi::Worker::builder(&harness.server_url, queue)
        .hostname("host-beta")
        .build()
        .await?;
    let b_counter = b_count.clone();
    worker_b.register("beta", move |_ctx: Context, _input: Empty| {
        let b_counter = b_counter.clone();
        async move {
            b_counter.fetch_add(1, Ordering::SeqCst);
            Ok::<_, anyhow::Error>(())
        }
    });
    let shutdown_b = worker_b.shutdown_token();
    let handle_b = tokio::spawn(async move { worker_b.run().await });

    // Ensure both workers are registered before submitting work.
    wait_for_worker_count(&harness.pool, queue, 2).await?;

    let mut client = harness.client().await;
    let alpha_run = client
        .start("alpha")
        .namespace(queue)
        .input(Empty)
        .r#await()
        .await?
        .await?;
    let beta_run = client
        .start("beta")
        .namespace(queue)
        .input(Empty)
        .r#await()
        .await?
        .await?;

    wait_for_status(
        &harness.server_url,
        &alpha_run,
        ProtoWorkflowStatus::Completed,
        40,
    )
    .await?;
    wait_for_status(
        &harness.server_url,
        &beta_run,
        ProtoWorkflowStatus::Completed,
        40,
    )
    .await?;

    assert_eq!(a_count.load(Ordering::SeqCst), 1);
    assert_eq!(b_count.load(Ordering::SeqCst), 1);

    shutdown_a.cancel();
    shutdown_b.cancel();
    let _ = handle_a.await;
    let _ = handle_b.await;
    Ok(())
}

#[tokio::test]
async fn worker_concurrency_limit_respected() -> anyhow::Result<()> {
    let harness = TestHarness::new().await;
    let queue = "e2e-worker-queue-limit";

    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));

    let mut worker = kagzi::Worker::builder(&harness.server_url, queue)
        .max_concurrent(1)
        .build()
        .await?;
    let active_clone = active.clone();
    let peak_clone = peak.clone();
    worker.register("queue_limited", move |_ctx: Context, _input: Empty| {
        let active_clone = active_clone.clone();
        let peak_clone = peak_clone.clone();
        async move {
            let current = active_clone.fetch_add(1, Ordering::SeqCst) + 1;
            peak_clone.fetch_max(current, Ordering::SeqCst);
            sleep(Duration::from_millis(400)).await;
            active_clone.fetch_sub(1, Ordering::SeqCst);
            Ok::<_, anyhow::Error>(())
        }
    });
    let shutdown = worker.shutdown_token();
    let handle = tokio::spawn(async move { worker.run().await });

    let mut client = harness.client().await;
    let mut run_ids = Vec::new();
    for _ in 0..4 {
        run_ids.push(
            client
                .start("queue_limited")
                .namespace(queue)
                .input(Empty)
                .r#await()
                .await?
                .await?,
        );
    }

    for run_id in &run_ids {
        wait_for_status(
            &harness.server_url,
            run_id,
            ProtoWorkflowStatus::Completed,
            40,
        )
        .await?;
    }

    shutdown.cancel();
    let _ = handle.await;

    let observed = peak.load(Ordering::SeqCst);
    assert!(
        observed <= 1,
        "worker max_concurrent should cap active workflows to 1, saw {}",
        observed
    );
    Ok(())
}

#[tokio::test]
async fn workflow_type_concurrency_limit_per_worker() -> anyhow::Result<()> {
    let harness = TestHarness::new().await;
    let queue = "e2e-worker-type-limit";

    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));

    let mut worker = kagzi::Worker::builder(&harness.server_url, queue)
        .max_concurrent(1)
        .workflow_type_concurrency("limited", 1)
        .build()
        .await?;
    let active_clone = active.clone();
    let peak_clone = peak.clone();
    worker.register("limited", move |_ctx: Context, _input: Empty| {
        let active_clone = active_clone.clone();
        let peak_clone = peak_clone.clone();
        async move {
            let current = active_clone.fetch_add(1, Ordering::SeqCst) + 1;
            peak_clone.fetch_max(current, Ordering::SeqCst);
            sleep(Duration::from_millis(500)).await;
            active_clone.fetch_sub(1, Ordering::SeqCst);
            Ok::<_, anyhow::Error>(())
        }
    });
    let shutdown = worker.shutdown_token();
    let handle = tokio::spawn(async move { worker.run().await });

    let mut client = harness.client().await;
    let mut run_ids = Vec::new();
    for _ in 0..3 {
        run_ids.push(
            client
                .start("limited")
                .namespace(queue)
                .input(Empty)
                .r#await()
                .await?
                .await?,
        );
    }

    for run_id in &run_ids {
        wait_for_status(
            &harness.server_url,
            run_id,
            ProtoWorkflowStatus::Completed,
            50,
        )
        .await?;
    }

    shutdown.cancel();
    let _ = handle.await;

    let observed = peak.load(Ordering::SeqCst);
    assert!(
        observed <= 1,
        "workflow_type_concurrency should cap per-type parallelism to 1, saw {}",
        observed
    );
    Ok(())
}
