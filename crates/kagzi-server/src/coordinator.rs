//! Coordinator - unified background task for Kagzi server.
//!
//! Handles:
//! - Firing due cron schedules
//! - Marking stale workers offline

use std::num::NonZeroU32;
use std::str::FromStr;
use std::time::Duration;

use chrono::Utc;
use governor::clock::DefaultClock;
use governor::state::{InMemoryState, NotKeyed};
use governor::{Quota, RateLimiter};
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

    // Initialize rate limiter for schedule backfilling
    let rate_limiter = RateLimiter::direct(Quota::per_second(
        NonZeroU32::new(settings.max_backfill_per_second.max(1) as u32)
            .expect("max(1) guarantees non-zero"),
    ));

    info!(
        interval_secs = settings.interval_secs,
        batch_size = settings.batch_size,
        worker_stale_secs = settings.worker_stale_threshold_secs,
        default_max_catchup = settings.default_max_catchup,
        max_backfill_per_second = settings.max_backfill_per_second,
        "Coordinator started"
    );

    loop {
        tokio::select! {
            _ = shutdown.cancelled() => {
                info!("Coordinator shutting down");
                break;
            }
            _ = ticker.tick() => {
                if let Err(e) = fire_due_schedules(&store, &queue, &settings, &rate_limiter).await {
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
    rate_limiter: &RateLimiter<NotKeyed, InMemoryState, DefaultClock>,
) -> Result<(), kagzi_store::StoreError> {
    let now = Utc::now();
    let templates = store
        .workflows()
        .find_due_schedules("*", now, settings.batch_size as i64)
        .await?;

    if templates.is_empty() {
        return Ok(());
    }

    // Only log count if there are many schedules being processed
    if templates.len() > 5 {
        info!(count = templates.len(), "Processing due schedules");
    }

    let mut fired = 0;

    for template in templates {
        let Some(current_fire_at) = template.available_at else {
            warn!(
                run_id = %template.run_id,
                namespace_id = %template.namespace_id,
                "Schedule template missing available_at"
            );
            continue;
        };
        let Some(ref cron_expr) = template.cron_expr else {
            warn!(
                run_id = %template.run_id,
                namespace_id = %template.namespace_id,
                "Schedule template missing cron_expr"
            );
            continue;
        };

        let cron = cron::Schedule::from_str(cron_expr).map_err(|e| {
            kagzi_store::StoreError::invalid_state(format!(
                "Schedule {}: Invalid cron expression '{}': {}",
                template.run_id, cron_expr, e
            ))
        })?;

        // Backfill-aware logic:
        // If max_catchup=0, skip all missed runs and jump to current time
        if template.max_catchup == 0 {
            let next_fire = cron
                .after(&now)
                .next()
                .unwrap_or(now + chrono::Duration::days(365));
            info!(
                schedule_id = %template.run_id,
                namespace_id = %template.namespace_id,
                "max_catchup=0, skipping missed runs"
            );
            store
                .workflows()
                .update_next_fire(template.run_id, next_fire, Some(now))
                .await?;
            continue;
        }

        // Calculate how many runs were missed (for logging and limiting)
        let cursor = template
            .last_fired_at
            .or(template.created_at)
            .unwrap_or(current_fire_at);
        let missed_count = cron.after(&cursor).take_while(|t| *t <= now).count();

        // If too many missed runs, skip excess and warn
        if missed_count > template.max_catchup as usize {
            warn!(
                schedule_id = %template.run_id,
                namespace_id = %template.namespace_id,
                missed = missed_count,
                max_catchup = template.max_catchup,
                "Too many missed runs, skipping to recent"
            );
            // Skip to current time minus catchup window
            let skip_to = cron
                .after(&cursor)
                .nth(missed_count.saturating_sub(template.max_catchup as usize))
                .unwrap_or(now);
            store
                .workflows()
                .update_next_fire(template.run_id, skip_to, Some(now))
                .await?;
            continue;
        }

        // Check global rate limit before creating instance
        if rate_limiter.check().is_err() {
            warn!(
                max_backfill_per_second = settings.max_backfill_per_second,
                "Rate limit reached for schedule backfill, pausing until next tick"
            );
            // Don't process this schedule now, will be picked up next tick
            continue;
        }

        // Fire the current occurrence (which is current_fire_at)
        match store
            .workflows()
            .create_schedule_instance(template.run_id, current_fire_at)
            .await?
        {
            Some(run_id) => {
                info!(
                    schedule_id = %template.run_id,
                    namespace_id = %template.namespace_id,
                    run_id = %run_id,
                    fire_at = %current_fire_at,
                    missed_count = missed_count,
                    "Fired schedule"
                );
                if let Err(e) = queue
                    .notify(&template.namespace_id, &template.task_queue)
                    .await
                {
                    error!(
                        schedule_id = %template.run_id,
                        run_id = %run_id,
                        namespace_id = %template.namespace_id,
                        task_queue = %template.task_queue,
                        error = ?e,
                        "Failed to notify queue after firing schedule"
                    );
                }
                fired += 1;
            }
            None => {
                // Instance already exists (deduplication via ON CONFLICT)
                // Still need to update next_fire time
            }
        }

        // Calculate NEXT fire time from the time slot we just fired (not from now)
        // This enables sequential catchup: next tick will pick up the following missed run
        let next_fire = cron
            .after(&current_fire_at)
            .next()
            .unwrap_or(now + chrono::Duration::days(365));

        store
            .workflows()
            .update_next_fire(template.run_id, next_fire, Some(current_fire_at))
            .await?;
    }

    // Only log if we actually fired something
    if fired > 0 {
        info!(fired, "Fired scheduled workflows");
    }

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
