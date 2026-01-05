use std::sync::Arc;

use async_trait::async_trait;
use backon::BackoffBuilder;
use dashmap::DashMap;
use sqlx::PgPool;
use sqlx::postgres::PgListener;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, instrument, warn};

use crate::error::QueueError;
use crate::traits::QueueNotifier;

#[derive(Clone)]
pub struct PostgresNotifier {
    pool: PgPool,
    channels: Arc<DashMap<String, broadcast::Sender<()>>>,
    channel_capacity: usize,
    cleanup_interval_secs: u64,
    max_reconnect_secs: u64,
}

impl PostgresNotifier {
    pub fn new(
        pool: PgPool,
        channel_capacity: usize,
        cleanup_interval_secs: u64,
        max_reconnect_secs: u64,
    ) -> Self {
        Self {
            pool,
            channels: Arc::new(DashMap::new()),
            channel_capacity,
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
                let (tx, _) = broadcast::channel(self.channel_capacity);
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

    async fn reconnect_listener(
        &self,
        shutdown: &CancellationToken,
    ) -> Result<PgListener, QueueError> {
        let max_attempts = (self.max_reconnect_secs / 10).max(3) as usize;

        let mut backoff = backon::ExponentialBuilder::default()
            .with_min_delay(std::time::Duration::from_secs(1))
            .with_max_delay(std::time::Duration::from_secs(30))
            .with_max_times(max_attempts)
            .with_jitter()
            .build();

        loop {
            if shutdown.is_cancelled() {
                return Ok(PgListener::connect_with(&self.pool).await?);
            }

            match PgListener::connect_with(&self.pool).await {
                Ok(mut listener) => match listener.listen("kagzi_work").await {
                    Ok(_) => {
                        info!("Queue listener reconnected");
                        return Ok(listener);
                    }
                    Err(e) => {
                        warn!(error = %e, "Failed to re-listen after reconnect, retrying");
                        if let Some(delay) = backoff.next() {
                            tokio::select! {
                                _ = shutdown.cancelled() => {
                                    return Ok(PgListener::connect_with(&self.pool).await?);
                                }
                                _ = tokio::time::sleep(delay) => {}
                            }
                        } else {
                            error!(
                                "Exhausted reconnection attempts after {} seconds",
                                self.max_reconnect_secs
                            );
                            return Err(QueueError::Database(sqlx::Error::PoolClosed));
                        }
                    }
                },
                Err(e) => {
                    warn!(error = %e, "Failed to reconnect listener, retrying");
                    if let Some(delay) = backoff.next() {
                        tokio::select! {
                            _ = shutdown.cancelled() => {
                                return Ok(PgListener::connect_with(&self.pool).await?);
                            }
                            _ = tokio::time::sleep(delay) => {}
                        }
                    } else {
                        error!(
                            "Exhausted reconnection attempts after {} seconds",
                            self.max_reconnect_secs
                        );
                        return Err(QueueError::Database(e));
                    }
                }
            }
        }
    }
}

#[async_trait]
impl QueueNotifier for PostgresNotifier {
    #[instrument(skip(self), fields(queue_key))]
    async fn notify(&self, namespace: &str, task_queue: &str) -> Result<(), QueueError> {
        let key = Self::queue_key(namespace, task_queue);
        tracing::Span::current().record("queue_key", &key);

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
                            listener = self.reconnect_listener(&shutdown).await?;
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
