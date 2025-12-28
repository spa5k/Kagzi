use std::time::Duration;

use kagzi_store::{PgStore, StepRepository, WorkerRepository, WorkflowRepository};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::config::WatchdogSettings;

/// Spawns three concurrent watchdog tasks that monitor and recover system state:
/// - Processes pending step retries
/// - Recovers orphaned workflows from crashed workers
/// - Marks stale workers as offline and recovers their workflows
///
/// All tasks run as independent async tasks on the tokio runtime, each with
/// their own interval ticker. Tasks gracefully shut down when the cancellation
/// token is triggered.
pub fn spawn(store: PgStore, settings: WatchdogSettings, shutdown: CancellationToken) {
    info!("Starting watchdog tasks");

    let interval = Duration::from_secs(settings.interval_secs);

    tokio::spawn(run_process_retries(
        store.clone(),
        shutdown.clone(),
        interval,
    ));

    tokio::spawn(run_find_orphaned(store.clone(), shutdown.clone(), interval));

    tokio::spawn(run_mark_stale(
        store,
        shutdown,
        interval,
        settings.worker_stale_threshold_secs,
    ));
}

/// Periodically processes steps that are due for retry after transient failures.
/// This handles the automatic retry mechanism for failed workflow steps based on
/// their configured retry policies and scheduled retry times.
async fn run_process_retries(store: PgStore, shutdown: CancellationToken, interval: Duration) {
    let mut ticker = tokio::time::interval(interval);
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => {
                info!("Watchdog process_retries exiting");
                break;
            }
            _ = ticker.tick() => {
                match store.steps().process_pending_retries().await {
                    Ok(retries) => {
                        for retry in &retries {
                            info!(
                                run_id = %retry.run_id,
                                step_id = %retry.step_id,
                                attempt = retry.attempt_number,
                                "Watchdog triggered step retry"
                            );
                        }
                    }
                    Err(e) => {
                        error!("Watchdog failed to process step retries: {:?}", e);
                    }
                }
            }
        }
    }
}

/// Periodically scans for workflows that were locked by workers that have crashed
/// or become unresponsive. Orphaned workflows are either retried with exponential
/// backoff or marked as failed if they've exhausted their retry attempts.
async fn run_find_orphaned(store: PgStore, shutdown: CancellationToken, interval: Duration) {
    let mut ticker = tokio::time::interval(interval);
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => {
                info!("Watchdog find_orphaned exiting");
                break;
            }
            _ = ticker.tick() => {
                let orphans = match store.workflows().find_orphaned().await {
                    Ok(orphans) => orphans,
                    Err(e) => {
                        error!("Watchdog failed to recover orphaned workflows: {:?}", e);
                        continue;
                    }
                };

                for orphan in orphans {
                    handle_orphaned_workflow(&store, orphan).await;
                }
            }
        }
    }
}

/// Handles recovery of a single orphaned workflow by checking its retry policy
/// and either scheduling a retry with exponential backoff or marking it as failed.
async fn handle_orphaned_workflow(store: &PgStore, orphan: kagzi_store::OrphanedWorkflow) {
    let policy = orphan.retry_policy.unwrap_or_default();
    let next_attempt = orphan.attempts + 1;

    // Check if we've already exhausted retries
    if !policy.should_retry(orphan.attempts) {
        mark_workflow_exhausted(store, orphan.run_id, orphan.attempts).await;
        return;
    }

    // Check if the next attempt would exhaust retries
    if !policy.should_retry(next_attempt) {
        mark_workflow_exhausted_with_increment(store, orphan.run_id, next_attempt).await;
        return;
    }

    // Schedule retry with backoff
    let delay_ms = policy.calculate_delay_ms(orphan.attempts) as u64;
    if let Err(e) = store
        .workflows()
        .schedule_retry(orphan.run_id, delay_ms)
        .await
    {
        error!(
            "Failed to schedule retry for orphaned workflow {}: {:?}",
            orphan.run_id, e
        );
        return;
    }

    warn!(
        run_id = %orphan.run_id,
        previous_worker = ?orphan.locked_by,
        delay_ms = delay_ms,
        "Recovered orphaned workflow - scheduling retry with backoff"
    );
}

/// Marks an orphaned workflow as exhausted/failed when it has already exceeded
/// its retry limit. Does not increment the attempt counter.
async fn mark_workflow_exhausted(store: &PgStore, run_id: uuid::Uuid, attempts: i32) {
    match store
        .workflows()
        .mark_exhausted(run_id, "Workflow crashed and exhausted all retry attempts")
        .await
    {
        Ok(_) => {
            error!(
                run_id = %run_id,
                attempts = attempts,
                "Orphaned workflow exhausted retries - marked as failed"
            );
        }
        Err(e) => {
            error!(
                "Failed to mark orphaned workflow {} as failed: {:?}",
                run_id, e
            );
        }
    }
}

/// Marks an orphaned workflow as exhausted/failed when the next retry attempt
/// would exceed its retry limit. Increments the attempt counter before marking
/// as failed to reflect the final attempted retry.
async fn mark_workflow_exhausted_with_increment(
    store: &PgStore,
    run_id: uuid::Uuid,
    next_attempt: i32,
) {
    match store
        .workflows()
        .mark_exhausted_with_increment(run_id, "Workflow crashed and exhausted all retry attempts")
        .await
    {
        Ok(_) => {
            error!(
                run_id = %run_id,
                attempts = next_attempt,
                "Orphaned workflow exhausted retries - marked as failed"
            );
        }
        Err(e) => {
            error!(
                "Failed to mark orphaned workflow {} as failed: {:?}",
                run_id, e
            );
        }
    }
}

/// Periodically marks workers as offline if they haven't sent a heartbeat within
/// the stale threshold, then recovers any workflows that were locked by those
/// offline workers. This is a two-phase process:
/// 1. Mark stale workers as offline based on last_heartbeat timestamp
/// 2. Find and recover workflows locked by offline workers (treating them as orphans)
async fn run_mark_stale(
    store: PgStore,
    shutdown: CancellationToken,
    interval: Duration,
    stale_threshold_secs: i64,
) {
    let mut ticker = tokio::time::interval(interval);
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => {
                info!("Watchdog mark_stale exiting");
                break;
            }
            _ = ticker.tick() => {
                // First mark stale workers as offline
                match store.workers().mark_stale_offline(stale_threshold_secs).await {
                    Ok(count) if count > 0 => {
                        warn!("Marked {} stale workers as offline", count);
                    }
                    Err(e) => error!("Failed to mark stale workers: {:?}", e),
                    _ => {}
                }

                // Then recover workflows from offline workers (treat as orphans)
                match store.workflows().find_and_recover_offline_worker_workflows().await {
                    Ok(recovered) if recovered > 0 => {
                        warn!(
                            recovered,
                            "Recovered workflows from offline workers"
                        );
                    }
                    Err(e) => error!("Failed to recover workflows from offline workers: {:?}", e),
                    _ => {}
                }
            }
        }
    }
}
