use crate::service::RetryPolicyConfig;
use sqlx::PgPool;
use tracing::{error, info, warn};

pub async fn run_reaper(pool: PgPool) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
    info!("Background Reaper started");

    loop {
        interval.tick().await;

        // 1. Wake up sleeping workflows whose wake_up_at has passed
        let wake_result = sqlx::query!(
            r#"
            UPDATE kagzi.workflow_runs
            SET status = 'PENDING',
                wake_up_at = NULL
            WHERE status = 'SLEEPING'
              AND wake_up_at <= NOW()
            "#
        )
        .execute(&pool)
        .await;

        match wake_result {
            Ok(r) => {
                if r.rows_affected() > 0 {
                    info!("Reaper woke up {} sleeping workflows", r.rows_affected());
                }
            }
            Err(e) => {
                error!("Reaper failed to wake sleeping workflows: {:?}", e);
            }
        }

        // 2. Process step retries (steps waiting for retry_at)
        let retry_result = sqlx::query!(
            r#"
            UPDATE kagzi.step_runs
            SET status = 'PENDING', retry_at = NULL
            WHERE status = 'PENDING'
              AND retry_at IS NOT NULL
              AND retry_at <= NOW()
            RETURNING run_id, step_id, attempt_number
            "#
        )
        .fetch_all(&pool)
        .await;

        match retry_result {
            Ok(rows) => {
                for row in &rows {
                    info!(run_id = %row.run_id, step_id = %row.step_id,
                          attempt = row.attempt_number, "Reaper triggered step retry");
                }
            }
            Err(e) => error!("Reaper failed to process step retries: {:?}", e),
        }

        // 3. Recover orphaned workflows (RUNNING but lock expired)
        // This happens when a worker crashes or loses connection without completing
        let orphan_result = sqlx::query!(
            r#"
            SELECT run_id, locked_by, attempts, retry_policy
            FROM kagzi.workflow_runs
            WHERE status = 'RUNNING'
              AND locked_until IS NOT NULL
              AND locked_until < NOW()
            FOR UPDATE SKIP LOCKED
            "#
        )
        .fetch_all(&pool)
        .await;

        match orphan_result {
            Ok(rows) => {
                for row in rows {
                    let policy: RetryPolicyConfig = row
                        .retry_policy
                        .map(|v| serde_json::from_value(v).unwrap_or_default())
                        .unwrap_or_default();

                    if policy.should_retry(row.attempts) {
                        // Calculate backoff delay
                        let delay = policy.calculate_delay(row.attempts);

                        // Schedule retry with backoff (not immediate restart!)
                        let update_result = sqlx::query!(
                            r#"
                            UPDATE kagzi.workflow_runs
                            SET status = 'PENDING',
                                locked_by = NULL,
                                locked_until = NULL,
                                wake_up_at = NOW() + ($2 * INTERVAL '1 millisecond'),
                                attempts = attempts + 1
                            WHERE run_id = $1
                            "#,
                            row.run_id,
                            delay.as_millis() as f64
                        )
                        .execute(&pool)
                        .await;

                        match update_result {
                            Ok(_) => {
                                warn!(run_id = %row.run_id, previous_worker = ?row.locked_by,
                                      delay_ms = delay.as_millis(),
                                      "Recovered orphaned workflow - scheduling retry with backoff");
                            }
                            Err(e) => error!(
                                "Failed to schedule retry for orphaned workflow {}: {:?}",
                                row.run_id, e
                            ),
                        }
                    } else {
                        // Max retries exhausted - mark as failed
                        let fail_result = sqlx::query!(
                            r#"
                            UPDATE kagzi.workflow_runs
                            SET status = 'FAILED',
                                error = 'Workflow crashed and exhausted all retry attempts',
                                finished_at = NOW(),
                                locked_by = NULL,
                                locked_until = NULL
                            WHERE run_id = $1
                            "#,
                            row.run_id
                        )
                        .execute(&pool)
                        .await;

                        match fail_result {
                            Ok(_) => {
                                error!(run_id = %row.run_id, attempts = row.attempts,
                                       "Orphaned workflow exhausted retries - marked as failed");
                            }
                            Err(e) => error!(
                                "Failed to mark orphaned workflow {} as failed: {:?}",
                                row.run_id, e
                            ),
                        }
                    }
                }
            }
            Err(e) => {
                error!("Reaper failed to recover orphaned workflows: {:?}", e);
            }
        }
    }
}
