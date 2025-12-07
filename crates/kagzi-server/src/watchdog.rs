use kagzi_store::{PgStore, StepRepository, WorkerRepository, WorkflowRepository};
use std::time::Duration;
use tracing::{error, info, warn};

pub async fn run(store: PgStore) {
    let interval_secs = std::env::var("KAGZI_WATCHDOG_INTERVAL_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(1);

    let mut interval = tokio::time::interval(Duration::from_secs(interval_secs));
    info!(interval_secs = interval_secs, "Watchdog started");

    loop {
        interval.tick().await;

        match store.workflows().wake_sleeping().await {
            Ok(count) => {
                if count > 0 {
                    info!("Watchdog woke up {} sleeping workflows", count);
                }
            }
            Err(e) => {
                error!("Watchdog failed to wake sleeping workflows: {:?}", e);
            }
        }

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

        // Mark stale workers offline (missed heartbeats)
        match store.workers().mark_stale_offline(30).await {
            Ok(count) if count > 0 => {
                warn!("Marked {} stale workers as offline", count);
            }
            Err(e) => error!("Failed to mark stale workers: {:?}", e),
            _ => {}
        }
    }
}
