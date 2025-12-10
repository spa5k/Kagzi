use std::time::Duration;

use sqlx::PgPool;
use sqlx::postgres::PgListener;

use crate::error::StoreError;

fn channel_name(namespace_id: &str, task_queue: &str) -> String {
    let digest = md5::compute(format!("{namespace_id}_{task_queue}"));
    format!("kagzi_work_{:x}", digest)
}

pub(super) async fn wait_for_new_work(
    pool: &PgPool,
    task_queue: &str,
    namespace_id: &str,
    timeout: Duration,
) -> Result<bool, StoreError> {
    let channel = channel_name(namespace_id, task_queue);
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
