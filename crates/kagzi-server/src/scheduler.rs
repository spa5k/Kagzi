use std::str::FromStr;
use std::time::Duration;

use chrono::{DateTime, Utc};
use cron::Schedule as CronSchedule;
use kagzi_queue::QueueNotifier;
use kagzi_store::{
    CreateWorkflow, PgStore, Schedule as WorkflowSchedule, WorkflowRepository,
    WorkflowScheduleRepository,
};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::config::SchedulerSettings;
use crate::constants::DEFAULT_VERSION;

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

async fn fire_workflow<Q: QueueNotifier>(
    workflows: &impl WorkflowRepository,
    queue: &Q,
    schedule: &WorkflowSchedule,
    fire_at: DateTime<Utc>,
) -> Result<Option<uuid::Uuid>, kagzi_store::StoreError> {
    let external_id = format!("{}:{}", schedule.schedule_id, fire_at.to_rfc3339());

    if let Some(existing) = workflows
        .find_active_by_external_id(&schedule.namespace_id, &external_id)
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
            external_id,
            task_queue: schedule.task_queue.clone(),
            workflow_type: schedule.workflow_type.clone(),
            input: schedule.input.clone(),
            namespace_id: schedule.namespace_id.clone(),
            version: schedule
                .version
                .clone()
                .unwrap_or_else(|| DEFAULT_VERSION.to_string()),
            retry_policy: None,
        })
        .await?;

    if let Err(e) = queue
        .notify(&schedule.namespace_id, &schedule.task_queue)
        .await
    {
        warn!(error = %e, "Failed to notify queue from scheduler");
    }

    Ok(Some(run_id))
}

pub async fn run<Q: QueueNotifier>(
    store: PgStore,
    queue: Q,
    settings: SchedulerSettings,
    shutdown: CancellationToken,
) {
    let interval_secs = settings.interval_secs.max(1);
    let batch_size = settings.batch_size.max(1);
    let max_workflows_per_tick = settings.max_workflows_per_tick.max(1);
    let interval = Duration::from_secs(interval_secs);
    let mut ticker = tokio::time::interval(interval);

    info!(
        interval_secs,
        batch_size, max_workflows_per_tick, "Scheduler started"
    );

    loop {
        tokio::select! {
            _ = shutdown.cancelled() => {
                info!("Scheduler shutting down");
                break;
            }
            _ = ticker.tick() => {}
        }

        match store.workflows().wake_sleeping(batch_size).await {
            Ok(woken) if woken.is_empty() => {
                debug!("No sleeping workflows to wake");
            }
            Ok(woken) => {
                info!(count = woken.len(), "Scheduler woke up sleeping workflows");
                // Notify queues for each woken workflow
                for workflow in &woken {
                    if let Err(e) = queue
                        .notify(&workflow.namespace_id, &workflow.task_queue)
                        .await
                    {
                        warn!(
                            run_id = %workflow.run_id,
                            task_queue = %workflow.task_queue,
                            error = %e,
                            "Failed to notify queue for woken workflow"
                        );
                    }
                }
            }
            Err(e) => {
                error!("Failed to wake sleeping workflows: {:?}", e);
            }
        }

        let now = Utc::now();

        let due: Vec<WorkflowSchedule> = match store
            .schedules()
            .due_schedules(now, batch_size as i64)
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
            if created_this_tick >= max_workflows_per_tick {
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

            let remaining_budget = (max_workflows_per_tick - created_this_tick) as usize;
            let from = schedule
                .last_fired_at
                .unwrap_or_else(|| schedule.next_fire_at - chrono::Duration::nanoseconds(1));
            let max_count = remaining_budget.min(schedule.max_catchup.max(1) as usize);
            let fires = compute_missed_fires(&schedule.cron_expr, from, now, max_count);

            let mut last_fired = None;
            for fire_at in fires {
                match fire_workflow(&workflows_repo, &queue, &schedule, fire_at).await {
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

                if created_this_tick >= max_workflows_per_tick {
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
    use chrono::{Days, TimeZone, Timelike};

    use super::*;

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
