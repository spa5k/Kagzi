//! Coordinator - unified background task for Kagzi server.
//!
//! Handles:
//! - Firing due cron schedules
//! - Marking stale workers offline

use std::collections::HashMap;
use std::num::NonZeroU32;
use std::str::FromStr;
use std::time::Duration;
use std::time::Instant;

use chrono::Utc;
use governor::clock::DefaultClock;
use governor::state::{InMemoryState, NotKeyed};
use governor::{Quota, RateLimiter};
use kagzi_queue::WorkSignalBus;
use kagzi_store::{PgStore, WorkerRepository, WorkflowRepository};
use sqlx::Row;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::config::{CoordinatorSettings, WorkerTelemetrySettings};
use crate::telemetry_store;

#[derive(Debug)]
struct PublishDebouncer {
    debounce: Duration,
    ttl: Duration,
    prune_interval: Duration,
    last_prune_at: Instant,
    last_published_at: HashMap<String, Instant>,
}

impl PublishDebouncer {
    fn new(debounce: Duration) -> Self {
        Self {
            debounce,
            ttl: Duration::from_secs(60),
            prune_interval: Duration::from_secs(10),
            last_prune_at: Instant::now(),
            last_published_at: HashMap::new(),
        }
    }

    fn should_publish(&mut self, namespace: &str, task_queue: &str) -> bool {
        let now = Instant::now();
        if now.duration_since(self.last_prune_at) >= self.prune_interval {
            let ttl = self.ttl;
            self.last_published_at
                .retain(|_, t| now.duration_since(*t) <= ttl);
            self.last_prune_at = now;
        }

        let key = format!("{namespace}:{task_queue}");
        if let Some(last) = self.last_published_at.get(&key)
            && now.duration_since(*last) < self.debounce
        {
            return false;
        }

        self.last_published_at.insert(key, now);
        true
    }
}

async fn record_queue_notify(
    store: &PgStore,
    telemetry_enabled: bool,
    namespace: &str,
    task_queue: &str,
    ok: bool,
    error: Option<&str>,
) {
    telemetry_store::record_queue_publish_result(
        store,
        telemetry_enabled,
        namespace,
        task_queue,
        ok,
        error,
    )
    .await;
    telemetry_store::touch_queue_due_work_notified(store, telemetry_enabled, namespace, task_queue)
        .await;
}

/// Run the coordinator loop.
///
/// This is a single background task that replaces the separate scheduler and watchdog tasks.
/// It runs on a configurable interval and handles:
/// 1. Firing due cron schedules (creates workflow runs for schedules that are ready)
/// 2. Marking stale workers offline (workers that haven't sent heartbeat)
pub async fn run<Q: WorkSignalBus>(
    store: PgStore,
    queue: Q,
    settings: CoordinatorSettings,
    worker_telemetry: WorkerTelemetrySettings,
    shutdown: CancellationToken,
) {
    let interval = Duration::from_secs(settings.interval_secs);
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let max_per_second = settings.max_backfill_per_second.max(1) as u32;
    let quota = NonZeroU32::new(max_per_second).expect("max_backfill_per_second >= 1");
    let rate_limiter = RateLimiter::direct(Quota::per_second(quota));

    // Best-effort per-queue debounce for WorkAvailable publishes to avoid signal storms.
    // Signals are lossy by design; correctness does not depend on every publish succeeding.
    let mut publish_debouncer = PublishDebouncer::new(Duration::from_millis(500));

    let prune_interval = Duration::from_secs(worker_telemetry.prune_interval_secs.max(1));
    let mut last_prune_at = Instant::now();

    info!(
        interval_secs = settings.interval_secs,
        batch_size = settings.batch_size,
        worker_stale_secs = settings.worker_stale_threshold_secs,
        default_max_catchup = settings.default_max_catchup,
        max_backfill_per_second = settings.max_backfill_per_second,
        worker_telemetry_enabled = worker_telemetry.enabled,
        worker_telemetry_retention_days = worker_telemetry.events_retention_days,
        "Coordinator started"
    );

    loop {
        tokio::select! {
            _ = shutdown.cancelled() => {
                info!("Coordinator shutting down");
                break;
            }
            _ = ticker.tick() => {
                if let Err(e) = fire_due_schedules(&store, &queue, &settings, &rate_limiter, &mut publish_debouncer, worker_telemetry.enabled).await {
                    error!("Failed to fire schedules: {:?}", e);
                    telemetry_store::record_server_event(
                        &store,
                        worker_telemetry.enabled,
                        "error",
                        "coordinator_fire_due_schedules_error",
                        "Failed to fire due schedules",
                        None,
                        serde_json::json!({ "error": format!("{e:?}") }),
                    )
                    .await;
                }

                if let Err(e) = mark_stale_workers(&store, settings.worker_stale_threshold_secs).await {
                    error!("Failed to mark stale workers: {:?}", e);
                    telemetry_store::record_server_event(
                        &store,
                        worker_telemetry.enabled,
                        "error",
                        "coordinator_mark_stale_workers_error",
                        "Failed to mark stale workers",
                        None,
                        serde_json::json!({ "error": format!("{e:?}") }),
                    )
                    .await;
                }

                if let Err(e) = notify_due_work(&store, &queue, settings.batch_size as i64, &mut publish_debouncer, worker_telemetry.enabled).await {
                    error!("Failed to notify due work: {:?}", e);
                    telemetry_store::record_server_event(
                        &store,
                        worker_telemetry.enabled,
                        "error",
                        "coordinator_notify_due_work_error",
                        "Failed to notify due work",
                        None,
                        serde_json::json!({ "error": format!("{e:?}") }),
                    )
                    .await;
                }

                if let Err(e) = telemetry_store::refresh_worker_active_counts(&store, worker_telemetry.enabled).await {
                    error!("Failed to refresh worker active counts: {:?}", e);
                }

                if let Err(e) = telemetry_store::refresh_queue_depths(&store, worker_telemetry.enabled, 1000).await {
                    error!("Failed to refresh queue depths: {:?}", e);
                }

                if worker_telemetry.enabled
                    && Instant::now().duration_since(last_prune_at) >= prune_interval
                {
                    last_prune_at = Instant::now();
                    if let Err(e) = prune_worker_telemetry_events(&store, worker_telemetry.events_retention_days).await {
                        error!("Failed to prune worker telemetry events: {:?}", e);
                    }
                }
            }
        }
    }
}

async fn prune_worker_telemetry_events(
    store: &PgStore,
    retention_days: i64,
) -> Result<(), kagzi_store::StoreError> {
    let retention_days = retention_days.max(1);
    let result = sqlx::query(
        r#"
        DELETE FROM kagzi.worker_telemetry_events
        WHERE occurred_at < NOW() - ($1 * INTERVAL '1 day')
        "#,
    )
    .bind(retention_days as f64)
    .execute(store.pool())
    .await?;

    if result.rows_affected() > 0 {
        info!(
            deleted = result.rows_affected(),
            retention_days, "Pruned worker telemetry events"
        );
    }

    Ok(())
}

async fn notify_due_work<Q: WorkSignalBus>(
    store: &PgStore,
    queue: &Q,
    limit: i64,
    publish_debouncer: &mut PublishDebouncer,
    telemetry_enabled: bool,
) -> Result<(), kagzi_store::StoreError> {
    let rows = sqlx::query(
        r#"
        SELECT namespace, task_queue, MIN(available_at) AS due_at
        FROM kagzi.workflow_runs
        WHERE status IN ('PENDING', 'SLEEPING', 'RUNNING')
          AND available_at <= NOW()
        GROUP BY namespace, task_queue
        ORDER BY due_at ASC
        LIMIT $1
        "#,
    )
    .bind(limit)
    .fetch_all(store.pool())
    .await?;

    for row in rows {
        let namespace: String = row.try_get("namespace")?;
        let task_queue: String = row.try_get("task_queue")?;

        if !publish_debouncer.should_publish(&namespace, &task_queue) {
            continue;
        }

        match queue.publish(&namespace, &task_queue).await {
            Ok(_) => {
                record_queue_notify(
                    store,
                    telemetry_enabled,
                    &namespace,
                    &task_queue,
                    true,
                    None,
                )
                .await
            }
            Err(e) => {
                let err = format!("{e:?}");
                record_queue_notify(
                    store,
                    telemetry_enabled,
                    &namespace,
                    &task_queue,
                    false,
                    Some(&err),
                )
                .await;
                error!(
                    namespace = %namespace,
                    task_queue = %task_queue,
                    error = ?e,
                    "Failed to notify queue for due work"
                );
            }
        }
    }

    Ok(())
}

async fn fire_due_schedules<Q: WorkSignalBus>(
    store: &PgStore,
    queue: &Q,
    settings: &CoordinatorSettings,
    rate_limiter: &RateLimiter<NotKeyed, InMemoryState, DefaultClock>,
    publish_debouncer: &mut PublishDebouncer,
    telemetry_enabled: bool,
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
                namespace = %template.namespace,
                "Schedule template missing available_at"
            );
            continue;
        };
        let Some(ref cron_expr) = template.cron_expr else {
            warn!(
                run_id = %template.run_id,
                namespace = %template.namespace,
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
                namespace = %template.namespace,
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
                namespace = %template.namespace,
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
                    namespace = %template.namespace,
                    run_id = %run_id,
                    fire_at = %current_fire_at,
                    missed_count = missed_count,
                    "Fired schedule"
                );
                if publish_debouncer.should_publish(&template.namespace, &template.task_queue) {
                    match queue
                        .publish(&template.namespace, &template.task_queue)
                        .await
                    {
                        Ok(_) => {
                            record_queue_notify(
                                store,
                                telemetry_enabled,
                                &template.namespace,
                                &template.task_queue,
                                true,
                                None,
                            )
                            .await;
                        }
                        Err(e) => {
                            let err = format!("{e:?}");
                            record_queue_notify(
                                store,
                                telemetry_enabled,
                                &template.namespace,
                                &template.task_queue,
                                false,
                                Some(&err),
                            )
                            .await;
                            error!(
                                schedule_id = %template.run_id,
                                run_id = %run_id,
                                namespace = %template.namespace,
                                task_queue = %template.task_queue,
                                error = ?e,
                                "Failed to notify queue after firing schedule"
                            );
                        }
                    }
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
