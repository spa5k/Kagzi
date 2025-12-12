use kagzi_store::{PgStore, StoreConfig};
use sqlx::PgPool;
use uuid::Uuid;
use chrono::Utc;
use std::time::Duration;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| "postgres://postgres:postgres@localhost:54122/postgres".to_string());

    let pool = PgPool::connect(&database_url).await?;
    let store = PgStore::new(pool, StoreConfig::default());

    // Create a test workflow in SLEEPING status with wake_up_at in the past
    let run_id = Uuid::new_v4();
    let wake_up_at = Utc::now() - chrono::Duration::seconds(1); // 1 second ago

    sqlx::query!(
        r#"
        INSERT INTO kagzi.workflow_runs (
            run_id, namespace_id, task_queue, workflow_type,
            status, wake_up_at, created_at, updated_at
        ) VALUES (
            $1, 'default', 'test', 'test_workflow',
            'SLEEPING', $2, NOW(), NOW()
        )
        "#,
        run_id,
        wake_up_at
    )
    .execute(&store.pool())
    .await?;

    println!("Inserted sleeping workflow with run_id: {} and wake_up_at: {}", run_id, wake_up_at);

    // Wait a bit
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Call wake_sleeping
    let count = store.workflows().wake_sleeping(10).await?;
    println!("wake_sleeping affected {} rows", count);

    // Check the status
    let status: String = sqlx::query_scalar(
        "SELECT status FROM kagzi.workflow_runs WHERE run_id = $1"
    )
    .bind(run_id)
    .fetch_one(&store.pool())
    .await?;

    println!("Workflow status after wake_sleeping: {}", status);

    // Clean up
    sqlx::query!("DELETE FROM kagzi.workflow_runs WHERE run_id = $1", run_id)
        .execute(&store.pool())
        .await?;

    Ok(())
}