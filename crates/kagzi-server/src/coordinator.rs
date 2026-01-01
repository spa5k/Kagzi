//! Coordinator - unified background task for Kagzi server.
//!
//! Handles:
//! - Firing due cron schedules
//! - Marking stale workers offline

use std::time::Duration;

use chrono::Utc;
use kagzi_queue::QueueNotifier;
use kagzi_store::{PgStore, WorkerRepository, WorkflowScheduleRepository};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::config::CoordinatorSettings;

/// Run the coordinator loop.
///
/// This is a single background task that replaces the separate scheduler and watchdog tasks.
/// It runs on a configurable interval and handles:
/// 1. Firing due cron schedules (creates workflow runs for schedules that are ready)
/// 2. Marking stale workers offline (workers that haven't sent heartbeat)
pub async fn run<Q: QueueNotifier>(
    store: PgStore,
    queue: Q,
    settings: CoordinatorSettings,
    shutdown: CancellationToken,
) {
    let interval = Duration::from_secs(settings.interval_secs);
    let mut ticker = tokio::time::interval(interval);
    // Don't burst-fire catchup ticks on startup
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    info!(
        interval_secs = settings.interval_secs,
        batch_size = settings.batch_size,
        worker_stale_secs = settings.worker_stale_threshold_secs,
        "Coordinator started"
    );

    loop {
        tokio::select! {
            _ = shutdown.cancelled() => {
                info!("Coordinator shutting down");
                break;
            }
            _ = ticker.tick() => {
                // 1. Fire due cron schedules
                if let Err(e) = fire_due_schedules(&store, &queue, &settings).await {
                    error!("Failed to fire schedules: {:?}", e);
                }

                // 2. Mark stale workers offline
                if let Err(e) = mark_stale_workers(&store, settings.worker_stale_threshold_secs).await {
                    error!("Failed to mark stale workers: {:?}", e);
                }
            }
        }
    }
}

/// Fire all cron schedules that are due.
async fn fire_due_schedules<Q: QueueNotifier>(
    store: &PgStore,
    queue: &Q,
    settings: &CoordinatorSettings,
) -> Result<(), kagzi_store::StoreError> {
    let now = Utc::now();
    let schedules = store
        .schedules()
        .due_schedules(now, settings.batch_size as i64)
        .await?;

    if schedules.is_empty() {
        return Ok(());
    }

    info!(count = schedules.len(), "Processing due schedules");

    for schedule in schedules {
        // Notify queue that work may be available for this schedule's task queue
        // The actual workflow creation happens through the schedule's next_fire logic
        let _ = queue
            .notify(&schedule.namespace_id, &schedule.task_queue)
            .await;

        info!(
            schedule_id = %schedule.schedule_id,
            task_queue = %schedule.task_queue,
            "Notified queue for due schedule"
        );
    }

    Ok(())
}

/// Mark workers that haven't sent heartbeat as offline.
async fn mark_stale_workers(
    store: &PgStore,
    threshold_secs: i64,
) -> Result<(), kagzi_store::StoreError> {
    let count = store.workers().mark_stale_offline(threshold_secs).await?;
    if count > 0 {
        warn!(count, "Marked stale workers offline");
    }
    Ok(())
}
