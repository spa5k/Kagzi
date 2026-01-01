//! Coordinator - unified background task for Kagzi server.
//!
//! Handles:
//! - Firing due cron schedules
//! - Marking stale workers offline

use std::str::FromStr;
use std::time::Duration;

use chrono::Utc;
use kagzi_queue::QueueNotifier;
use kagzi_store::{PgStore, WorkerRepository, WorkflowRepository};
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
                if let Err(e) = fire_due_schedules(&store, &queue, &settings).await {
                    error!("Failed to fire schedules: {:?}", e);
                }

                if let Err(e) = mark_stale_workers(&store, settings.worker_stale_threshold_secs).await {
                    error!("Failed to mark stale workers: {:?}", e);
                }
            }
        }
    }
}

async fn fire_due_schedules<Q: QueueNotifier>(
    store: &PgStore,
    queue: &Q,
    settings: &CoordinatorSettings,
) -> Result<(), kagzi_store::StoreError> {
    let now = Utc::now();
    let templates = store
        .workflows()
        .find_due_schedules("*", now, settings.batch_size as i64)
        .await?;

    if templates.is_empty() {
        return Ok(());
    }

    info!(count = templates.len(), "Processing due schedules");

    let mut fired = 0;

    for template in templates {
        if let Some(run_id) = store
            .workflows()
            .create_schedule_instance(template.run_id, template.available_at.unwrap())
            .await?
        {
            info!(
                schedule_id = %template.run_id,
                run_id = %run_id,
                "Fired schedule"
            );
            if let Err(e) = queue
                .notify(&template.namespace_id, &template.task_queue)
                .await
            {
                warn!("Failed to notify queue: {:?}", e);
            }
            fired += 1;
        }

        let cron = cron::Schedule::from_str(template.cron_expr.as_ref().unwrap())
            .map_err(|e| kagzi_store::StoreError::invalid_state(format!("Invalid cron: {}", e)))?;
        let next_fire = cron
            .after(&now)
            .next()
            .unwrap_or(now + chrono::Duration::days(365));

        store
            .workflows()
            .update_next_fire(template.run_id, next_fire)
            .await?;
    }

    info!(fired, "Finished processing due schedules");
    Ok(())
}

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
