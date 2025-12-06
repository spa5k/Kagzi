use chrono::{DateTime, Utc};
use cron::Schedule as CronSchedule;
use kagzi_store::{CreateWorkflow, PgStore, Schedule, ScheduleRepository, WorkflowRepository};
use std::str::FromStr;
use std::time::Duration;
use tracing::{error, info, warn};

#[derive(Debug, Clone)]
struct SchedulerConfig {
    interval: Duration,
    batch_size: i32,
    max_workflows_per_tick: i32,
}

impl SchedulerConfig {
    /// Constructs a SchedulerConfig from environment variables, using sensible defaults when variables are absent or invalid.
    ///
    /// Reads these environment variables:
    /// - `KAGZI_SCHEDULER_INTERVAL_SECS`: positive integer seconds for the scheduler tick interval (default: 5).
    /// - `KAGZI_SCHEDULER_BATCH_SIZE`: positive integer batch size for fetching due schedules (default: 100).
    /// - `KAGZI_SCHEDULER_MAX_WORKFLOWS_PER_TICK`: positive integer cap on workflows created per tick (default: 1000).
    ///
    /// # Returns
    ///
    /// A `SchedulerConfig` populated from the environment or with default values when parsing fails or values are non-positive.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    ///
    /// std::env::set_var("KAGZI_SCHEDULER_INTERVAL_SECS", "2");
    /// std::env::set_var("KAGZI_SCHEDULER_BATCH_SIZE", "50");
    /// std::env::set_var("KAGZI_SCHEDULER_MAX_WORKFLOWS_PER_TICK", "200");
    ///
    /// let cfg = SchedulerConfig::from_env();
    /// assert_eq!(cfg.interval, Duration::from_secs(2));
    /// assert_eq!(cfg.batch_size, 50);
    /// assert_eq!(cfg.max_workflows_per_tick, 200);
    /// ```
    fn from_env() -> Self {
        let interval = std::env::var("KAGZI_SCHEDULER_INTERVAL_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|v| *v > 0)
            .map(Duration::from_secs)
            .unwrap_or_else(|| Duration::from_secs(5));

        let batch_size = std::env::var("KAGZI_SCHEDULER_BATCH_SIZE")
            .ok()
            .and_then(|v| v.parse::<i32>().ok())
            .filter(|v| *v > 0)
            .unwrap_or(100);

        let max_workflows_per_tick = std::env::var("KAGZI_SCHEDULER_MAX_WORKFLOWS_PER_TICK")
            .ok()
            .and_then(|v| v.parse::<i32>().ok())
            .filter(|v| *v > 0)
            .unwrap_or(1000);

        Self {
            interval,
            batch_size,
            max_workflows_per_tick,
        }
    }
}

/// Parse a cron expression into a CronSchedule.
///
/// Returns `Some(CronSchedule)` when `expr` is a valid cron expression, `None` when parsing fails.
///
/// # Examples
///
/// ```
/// let sched = parse_cron("0 */2 * * * *");
/// assert!(sched.is_some());
/// ```
fn parse_cron(expr: &str) -> Option<CronSchedule> {
    CronSchedule::from_str(expr).ok()
}

/// Compute the next scheduled fire time after a given instant for a cron expression.
///
/// If `expr` parses as a cron schedule and has an occurrence strictly after `after`,
/// returns that occurrence converted to UTC; otherwise returns `None`.
///
/// # Parameters
///
/// - `expr`: a cron expression string.
/// - `after`: the instant after which to find the next occurrence (UTC).
///
/// # Examples
///
/// ```
/// use chrono::{DateTime, Utc, TimeZone};
/// let expr = "0 0 0 * * *"; // daily at midnight
/// let after: DateTime<Utc> = Utc.ymd(2025, 12, 5).and_hms(0, 0, 0);
/// let next = compute_next_tick(expr, after).unwrap();
/// assert_eq!(next, Utc.ymd(2025, 12, 6).and_hms(0, 0, 0));
/// ```
fn compute_next_tick(expr: &str, after: DateTime<Utc>) -> Option<DateTime<Utc>> {
    parse_cron(expr)
        .and_then(|sched| sched.after(&after).next())
        .map(|dt| dt.with_timezone(&Utc))
}

/// Computes at most `max_count` scheduled firing times for `cron_expr` that occur after `from` and not later than `to`.
///
/// If `cron_expr` is invalid, an empty vector is returned. The function returns occurrences that are strictly after `from` and less than or equal to `to`, in ascending order, up to `max_count`.
///
/// # Examples
///
/// ```
/// use chrono::{TimeZone, Utc};
/// let cron = "0 */2 * * * *"; // every 2 minutes at second 0
/// let from = Utc.ymd(2025, 12, 6).and_hms(12, 0, 0);
/// let to = Utc.ymd(2025, 12, 6).and_hms(12, 10, 0);
/// let fires = compute_missed_fires(cron, from, to, 3);
/// assert_eq!(fires.len(), 3);
/// assert_eq!(fires[0].minute(), 2);
/// assert_eq!(fires[1].minute(), 4);
/// assert_eq!(fires[2].minute(), 6);
/// ```
fn compute_missed_fires(
    cron_expr: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    max_count: usize,
) -> Vec<DateTime<Utc>> {
    let Some(schedule) = parse_cron(cron_expr) else {
        return vec![];
    };

    let mut fires = Vec::new();
    for ts in schedule.after(&from) {
        let ts = ts.with_timezone(&Utc);
        if ts > to || fires.len() >= max_count {
            break;
        }
        fires.push(ts);
    }
    fires
}

/// Attempts to create a workflow run for the given schedule at the specified firing time, ensuring idempotency.
///
/// The function uses an idempotency key derived from the schedule's ID and `fire_at` to avoid creating duplicate runs:
/// if a workflow with the same idempotency key already exists in the schedule's namespace, the call is skipped.
///
/// # Examples
///
/// ```no_run
/// # use chrono::Utc;
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let workflows = /* impl of WorkflowRepository */ unimplemented!();
/// let schedule = /* Schedule instance */ unimplemented!();
/// let fire_at = Utc::now();
/// let created = fire_workflow(&workflows, &schedule, fire_at).await?;
/// match created {
///     Some(run_id) => println!("Created workflow run {}", run_id),
///     None => println!("Skipped: idempotency hit"),
/// }
/// # Ok(()) }
/// ```
—
/// # Returns
///
/// `Some(run_id)` if a new workflow run was created, `None` if a run with the same idempotency key already exists.
async fn fire_workflow(
    workflows: &impl WorkflowRepository,
    schedule: &Schedule,
    fire_at: DateTime<Utc>,
) -> Result<Option<uuid::Uuid>, kagzi_store::StoreError> {
    let idempotency_key = format!("schedule:{}:{}", schedule.schedule_id, fire_at.to_rfc3339());

    if let Some(existing) = workflows
        .find_by_idempotency_key(&schedule.namespace_id, &idempotency_key)
        .await?
    {
        warn!(
            schedule_id = %schedule.schedule_id,
            fire_at = %fire_at,
            run_id = %existing,
            "Skipping schedule fire due to idempotency hit"
        );
        return Ok(None);
    }

    let run_id = workflows
        .create(CreateWorkflow {
            business_id: schedule.schedule_id.to_string(),
            task_queue: schedule.task_queue.clone(),
            workflow_type: schedule.workflow_type.clone(),
            input: schedule.input.clone(),
            namespace_id: schedule.namespace_id.clone(),
            idempotency_key: Some(idempotency_key),
            context: schedule.context.clone(),
            deadline_at: None,
            version: schedule.version.clone().unwrap_or_else(|| "1".to_string()),
            retry_policy: None,
        })
        .await?;

    Ok(Some(run_id))
}

/// Starts the scheduler loop that periodically scans due schedules and fires workflows.
///
/// This function runs indefinitely. On each tick it queries `store` for schedules due to run, computes any missed cron firings within configured limits, attempts idempotent workflow creation for each firing, records successful firings, and advances schedules to their next occurrence.
///
/// # Parameters
/// - `store`: Postgres-backed repository implementing schedule and workflow operations used by the scheduler.
///
/// # Examples
///
/// ```
/// // Spawn the scheduler on the tokio runtime.
/// tokio::spawn(async move {
///     run(store).await;
/// });
/// ```
pub async fn run(store: PgStore) {
    let config = SchedulerConfig::from_env();
    let mut ticker = tokio::time::interval(config.interval);

    info!(
        interval_secs = config.interval.as_secs(),
        batch_size = config.batch_size,
        max_workflows_per_tick = config.max_workflows_per_tick,
        "Scheduler started"
    );

    loop {
        ticker.tick().await;
        let now = Utc::now();

        let due: Vec<Schedule> = match store
            .schedules()
            .due_schedules(now, config.batch_size)
            .await
        {
            Ok(due) => due,
            Err(e) => {
                error!("Failed to fetch due schedules: {:?}", e);
                continue;
            }
        };

        if due.is_empty() {
            continue;
        }

        let mut created_this_tick = 0;
        let workflows_repo = store.workflows();

        for schedule in due {
            if created_this_tick >= config.max_workflows_per_tick {
                break;
            }

            if parse_cron(&schedule.cron_expr).is_none() {
                warn!(
                    schedule_id = %schedule.schedule_id,
                    cron = %schedule.cron_expr,
                    "Skipping schedule with invalid cron expression"
                );
                continue;
            }

            let remaining_budget = (config.max_workflows_per_tick - created_this_tick) as usize;
            // Use `last_fired_at` if available; otherwise, shift `next_fire_at` back by 1ns
            // so that `schedule.after(&from)` includes the scheduled fire time itself.
            let from = schedule
                .last_fired_at
                .unwrap_or_else(|| schedule.next_fire_at - chrono::Duration::nanoseconds(1));
            let max_count = remaining_budget.min(schedule.max_catchup.max(1) as usize);
            let fires = compute_missed_fires(&schedule.cron_expr, from, now, max_count);

            let mut last_fired = None;
            for fire_at in fires {
                match fire_workflow(&workflows_repo, &schedule, fire_at).await {
                    Ok(Some(run_id)) => {
                        created_this_tick += 1;
                        last_fired = Some(fire_at);
                        if let Err(e) = store
                            .schedules()
                            .record_firing(schedule.schedule_id, fire_at, run_id)
                            .await
                        {
                            warn!(
                                schedule_id = %schedule.schedule_id,
                                run_id = %run_id,
                                "Failed to record firing: {:?}", e
                            );
                        }
                    }
                    Ok(None) => {
                        last_fired = Some(fire_at);
                    }
                    Err(e) => {
                        error!(
                            schedule_id = %schedule.schedule_id,
                            fire_at = %fire_at,
                            "Failed to create workflow for schedule: {:?}", e
                        );
                    }
                }

                if created_this_tick >= config.max_workflows_per_tick {
                    break;
                }
            }

            let advance_from = last_fired.unwrap_or(from);
            if let Some(next_fire) = compute_next_tick(&schedule.cron_expr, advance_from) {
                if let Err(e) = store
                    .schedules()
                    .advance_schedule(schedule.schedule_id, advance_from, next_fire)
                    .await
                {
                    error!(
                        schedule_id = %schedule.schedule_id,
                        "Failed to advance schedule: {:?}", e
                    );
                }
            } else {
                warn!(
                    schedule_id = %schedule.schedule_id,
                    "Cron expression has no future occurrences; not advancing schedule"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Days, TimeZone, Timelike};

    #[test]
    fn missed_fires_respects_bounds_and_limit() {
        let cron = "0 */2 * * * *"; // every 2 minutes at second 0
        let from = Utc
            .with_ymd_and_hms(2025, 12, 6, 12, 0, 0)
            .single()
            .unwrap();
        let to = Utc
            .with_ymd_and_hms(2025, 12, 6, 12, 10, 0)
            .single()
            .unwrap();

        let fires = compute_missed_fires(cron, from, to, 3);
        assert_eq!(fires.len(), 3);
        assert_eq!(fires[0].minute(), 2);
        assert_eq!(fires[1].minute(), 4);
        assert_eq!(fires[2].minute(), 6);
    }

    #[test]
    fn compute_next_tick_advances_after_last_fire() {
        let cron = "0 0 0 * * *"; // daily at midnight
        let last = Utc.with_ymd_and_hms(2025, 12, 5, 0, 0, 0).single().unwrap();
        let next = compute_next_tick(cron, last).unwrap();
        assert_eq!(next.date_naive(), last.date_naive() + Days::new(1));
    }
}