use sqlx::PgPool;
use tracing::{error, info};

pub async fn run_reaper(pool: PgPool) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
    info!("Background Reaper started");

    loop {
        interval.tick().await;

        let result = sqlx::query!(
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

        match result {
            Ok(r) => {
                if r.rows_affected() > 0 {
                    info!("Reaper woke up {} workflows", r.rows_affected());
                }
            }
            Err(e) => {
                error!("Reaper failed to wake workflows: {:?}", e);
            }
        }
    }
}
