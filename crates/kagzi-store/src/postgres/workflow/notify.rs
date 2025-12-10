use std::time::Duration;

use sqlx::PgPool;
use sqlx::postgres::PgListener;

use crate::error::StoreError;

pub(super) async fn wait_for_new_work(
    pool: &PgPool,
    task_queue: &str,
    namespace_id: &str,
    timeout: Duration,
) -> Result<bool, StoreError> {
    let channel = format!("kagzi_work_{}_{}", namespace_id, task_queue);
    let mut listener = PgListener::connect_with(pool).await?;
    listener.listen(&channel).await?;

    let notified = tokio::select! {
        result = listener.recv() => {
            result?;
            true
        }
        _ = tokio::time::sleep(timeout) => false,
    };

    Ok(notified)
}
