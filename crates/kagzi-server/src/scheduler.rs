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

fn parse_cron(expr: &str) -> Option<CronSchedule> {
    CronSchedule::from_str(expr).ok()
}

fn compute_next_tick(expr: &str, after: DateTime<Utc>) -> Option<DateTime<Utc>> {
    parse_cron(expr)
        .and_then(|sched| sched.after(&after).next())
        .map(|dt| dt.with_timezone(&Utc))
}

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
            let from = schedule.last_fired_at.unwrap_or(schedule.next_fire_at);
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
        let cron = "*/2 * * * *"; // every 2 minutes
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
        let cron = "0 0 * * *"; // daily
        let last = Utc.with_ymd_and_hms(2025, 12, 5, 0, 0, 0).single().unwrap();
        let next = compute_next_tick(cron, last).unwrap();
        assert_eq!(next.date_naive(), last.date_naive() + Days::new(1));
    }
}
