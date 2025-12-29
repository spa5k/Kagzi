use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;
use sqlx::PgPool;
use sqlx::postgres::PgListener;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::error::QueueError;
use crate::traits::QueueNotifier;

const CHANNEL_CAPACITY: usize = 64;

#[derive(Clone)]
pub struct PostgresNotifier {
    pool: PgPool,
    channels: Arc<DashMap<String, broadcast::Sender<()>>>,
    cleanup_interval_secs: u64,
    max_reconnect_secs: u64,
}

impl PostgresNotifier {
    pub fn new(pool: PgPool, cleanup_interval_secs: u64, max_reconnect_secs: u64) -> Self {
        Self {
            pool,
            channels: Arc::new(DashMap::new()),
            cleanup_interval_secs,
            max_reconnect_secs,
        }
    }

    fn queue_key(namespace: &str, task_queue: &str) -> String {
        format!("{}:{}", namespace, task_queue)
    }

    fn get_or_create_channel(&self, key: &str) -> broadcast::Sender<()> {
        self.channels
            .entry(key.to_string())
            .or_insert_with(|| {
                let (tx, _) = broadcast::channel(CHANNEL_CAPACITY);
                tx
            })
            .clone()
    }

    fn cleanup_stale_channels(&self) {
        let mut removed = 0;
        self.channels.retain(|key, tx| {
            if tx.receiver_count() == 0 {
                debug!(queue = %key, "Removing stale channel for queue");
                removed += 1;
                false
            } else {
                true
            }
        });
        if removed > 0 {
            info!(count = removed, "Cleaned up stale notification channels");
        }
    }
}

#[async_trait]
impl QueueNotifier for PostgresNotifier {
    async fn notify(&self, namespace: &str, task_queue: &str) -> Result<(), QueueError> {
        let key = Self::queue_key(namespace, task_queue);

        sqlx::query("SELECT pg_notify('kagzi_work', $1)")
            .bind(&key)
            .execute(&self.pool)
            .await?;

        debug!(queue = %key, "Sent pg_notify");

        if let Some(tx) = self.channels.get(&key) {
            let _ = tx.send(());
        }

        Ok(())
    }

    fn subscribe(&self, namespace: &str, task_queue: &str) -> broadcast::Receiver<()> {
        let key = Self::queue_key(namespace, task_queue);
        let tx = self.get_or_create_channel(&key);
        tx.subscribe()
    }

    async fn start(&self, shutdown: CancellationToken) -> Result<(), QueueError> {
        let mut listener = PgListener::connect_with(&self.pool).await?;
        listener.listen("kagzi_work").await?;

        info!("Queue listener started on channel 'kagzi_work'");

        let mut cleanup_interval =
            tokio::time::interval(std::time::Duration::from_secs(self.cleanup_interval_secs));

        loop {
            tokio::select! {
                biased;

                _ = shutdown.cancelled() => {
                    info!("Queue listener shutting down");
                    break;
                }

                _ = cleanup_interval.tick() => {
                    self.cleanup_stale_channels();
                }

                result = listener.recv() => {
                    match result {
                        Ok(notification) => {
                            let key = notification.payload();
                            debug!(queue = %key, "Received pg_notify");

                            if let Some(tx) = self.channels.get(key) {
                                let _ = tx.send(());
                            }
                        }
                        Err(e) => {
                            error!(error = %e, "Error receiving notification, attempting to reconnect");

                            let mut backoff = backoff::ExponentialBackoff {
                                initial_interval: std::time::Duration::from_secs(1),
                                max_interval: std::time::Duration::from_secs(30),
                                max_elapsed_time: Some(std::time::Duration::from_secs(self.max_reconnect_secs)),
                                ..Default::default()
                            };

                            loop {
                                if shutdown.is_cancelled() {
                                    return Ok(());
                                }

                                match PgListener::connect_with(&self.pool).await {
                                    Ok(mut new_listener) => {
                                        if let Err(e) = new_listener.listen("kagzi_work").await {
                                            warn!(error = %e, "Failed to re-listen after reconnect, retrying");
                                            if let Some(delay) = backoff::backoff::Backoff::next_backoff(&mut backoff) {
                                                tokio::time::sleep(delay).await;
                                                continue;
                                            } else {
                                                error!("Exhausted reconnection attempts after {} seconds", self.max_reconnect_secs);
                                                return Err(QueueError::Other(format!(
                                                    "Failed to reconnect queue listener after {} seconds",
                                                    self.max_reconnect_secs
                                                )));
                                            }
                                        }
                                        listener = new_listener;
                                        info!("Queue listener reconnected");
                                        break;
                                    }
                                    Err(e) => {
                                        warn!(error = %e, "Failed to reconnect listener, retrying");
                                        if let Some(delay) = backoff::backoff::Backoff::next_backoff(&mut backoff) {
                                            tokio::time::sleep(delay).await;
                                            continue;
                                        } else {
                                            error!("Exhausted reconnection attempts after {} seconds", self.max_reconnect_secs);
                                            return Err(QueueError::Database(e));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_queue_key() {
        assert_eq!(
            PostgresNotifier::queue_key("default", "main"),
            "default:main"
        );
    }
}
