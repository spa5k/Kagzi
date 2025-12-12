use std::time::Duration;

use kagzi_store::{PgStore, StepRepository, WorkerRepository, WorkflowRepository};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::config::WatchdogSettings;

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

async fn run_find_orphaned(store: PgStore, shutdown: CancellationToken, interval: Duration) {
    let mut ticker = tokio::time::interval(interval);
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => {
                info!("Watchdog find_orphaned exiting");
                break;
            }
            _ = ticker.tick() => {
                match store.workflows().find_orphaned().await {
                    Ok(orphans) => {
                        for orphan in orphans {
                            let policy = orphan.retry_policy.unwrap_or_default();

                            if policy.should_retry(orphan.attempts) {
                                let delay_ms = policy.calculate_delay_ms(orphan.attempts) as u64;

                                match store
                                    .workflows()
                                    .schedule_retry(orphan.run_id, delay_ms)
                                    .await
                                {
                                    Ok(_) => {
                                        warn!(
                                            run_id = %orphan.run_id,
                                            previous_worker = ?orphan.locked_by,
                                            delay_ms = delay_ms,
                                            "Recovered orphaned workflow - scheduling retry with backoff"
                                        );
                                    }
                                    Err(e) => {
                                        error!(
                                            "Failed to schedule retry for orphaned workflow {}: {:?}",
                                            orphan.run_id, e
                                        );
                                    }
                                }
                            } else {
                                match store
                                    .workflows()
                                    .mark_exhausted(
                                        orphan.run_id,
                                        "Workflow crashed and exhausted all retry attempts",
                                    )
                                    .await
                                {
                                    Ok(_) => {
                                        error!(
                                            run_id = %orphan.run_id,
                                            attempts = orphan.attempts,
                                            "Orphaned workflow exhausted retries - marked as failed"
                                        );
                                    }
                                    Err(e) => {
                                        error!(
                                            "Failed to mark orphaned workflow {} as failed: {:?}",
                                            orphan.run_id, e
                                        );
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        error!("Watchdog failed to recover orphaned workflows: {:?}", e);
                    }
                }
            }
        }
    }
}

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
                match store.workers().mark_stale_offline(stale_threshold_secs).await {
                    Ok(count) if count > 0 => {
                        warn!("Marked {} stale workers as offline", count);
                    }
                    Err(e) => error!("Failed to mark stale workers: {:?}", e),
                    _ => {}
                }
            }
        }
    }
}
