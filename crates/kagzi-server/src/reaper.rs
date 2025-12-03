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

        // 2. Recover orphaned workflows (RUNNING but lock expired)
        // This happens when a worker crashes or loses connection without completing
        let orphan_result = sqlx::query!(
            r#"
            UPDATE kagzi.workflow_runs
            SET status = 'PENDING',
                locked_by = NULL,
                locked_until = NULL,
                attempts = attempts + 1
            WHERE status = 'RUNNING'
              AND locked_until IS NOT NULL
              AND locked_until < NOW()
            RETURNING run_id, locked_by
            "#
        )
        .fetch_all(&pool)
        .await;

        match orphan_result {
            Ok(rows) => {
                for row in &rows {
                    warn!(
                        run_id = %row.run_id,
                        previous_worker = ?row.locked_by,
                        "Recovered orphaned workflow - worker lock expired"
                    );
                }
                if !rows.is_empty() {
                    info!("Reaper recovered {} orphaned workflows", rows.len());
                }
            }
            Err(e) => {
                error!("Reaper failed to recover orphaned workflows: {:?}", e);
            }
        }
    }
}
