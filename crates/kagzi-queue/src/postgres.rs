use async_trait::async_trait;
use backon::BackoffBuilder;
use sqlx::PgPool;
use sqlx::postgres::PgListener;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, instrument, warn};

use crate::bus::{WorkAvailable, WorkSignalBus, queue_key};
use crate::error::QueueError;

#[derive(Clone)]
pub struct PostgresNotifier {
    pool: PgPool,
    channels: std::sync::Arc<dashmap::DashMap<String, broadcast::Sender<WorkAvailable>>>,
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
            channels: std::sync::Arc::new(dashmap::DashMap::new()),
            channel_capacity,
            cleanup_interval_secs,
            max_reconnect_secs,
        }
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
        if removed != 0 {
            info!(count = removed, "Cleaned up stale notification channels");
        }
    }

    async fn reconnect_listener(
        &self,
        shutdown: &CancellationToken,
    ) -> Result<PgListener, QueueError> {
        async fn connect_and_listen(pool: &PgPool) -> Result<PgListener, sqlx::Error> {
            let mut listener = PgListener::connect_with(pool).await?;
            listener.listen("kagzi_work").await?;
            Ok(listener)
        }

        let max_attempts = (self.max_reconnect_secs / 10).max(3) as usize;

        let mut backoff = backon::ExponentialBuilder::default()
            .with_min_delay(std::time::Duration::from_secs(1))
            .with_max_delay(std::time::Duration::from_secs(30))
            .with_max_times(max_attempts)
            .with_jitter()
            .build();

        loop {
            tokio::select! {
                _ = shutdown.cancelled() => return Err(QueueError::Other("shutdown".to_string())),
                res = connect_and_listen(&self.pool) => {
                    match res {
                        Ok(listener) => {
                            info!("Queue listener reconnected");
                            return Ok(listener);
                        }
                        Err(e) => {
                            warn!(error = %e, "Failed to reconnect listener, retrying");
                        }
                    }
                }
            }

            let Some(delay) = backoff.next() else {
                error!(
                    "Exhausted reconnection attempts after {} seconds",
                    self.max_reconnect_secs
                );
                return Err(QueueError::Other(
                    "queue listener reconnect exhausted".to_string(),
                ));
            };

            tokio::select! {
                _ = shutdown.cancelled() => return Err(QueueError::Other("shutdown".to_string())),
                _ = tokio::time::sleep(delay) => {}
            }
        }
    }
}

#[async_trait]
impl WorkSignalBus for PostgresNotifier {
    #[instrument(skip(self), fields(queue_key))]
    async fn publish(&self, namespace: &str, task_queue: &str) -> Result<(), QueueError> {
        let key = queue_key(namespace, task_queue);
        tracing::Span::current().record("queue_key", &key);

        sqlx::query("SELECT pg_notify('kagzi_work', $1)")
            .bind(&key)
            .execute(&self.pool)
            .await?;

        debug!(queue = %key, "Sent pg_notify");

        if let Some(tx) = self.channels.get(&key) {
            let _ = tx.send(WorkAvailable {
                namespace: namespace.to_string(),
                task_queue: task_queue.to_string(),
            });
        }

        Ok(())
    }

    fn subscribe(&self, namespace: &str, task_queue: &str) -> broadcast::Receiver<WorkAvailable> {
        let key = queue_key(namespace, task_queue);
        self.channels
            .entry(key)
            .or_insert_with(|| {
                let (tx, _) = broadcast::channel(self.channel_capacity);
                tx
            })
            .subscribe()
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

                            if let Some((namespace, task_queue)) = key.split_once(':')
                                && let Some(tx) = self.channels.get(key)
                            {
                                let _ = tx.send(WorkAvailable {
                                    namespace: namespace.to_string(),
                                    task_queue: task_queue.to_string(),
                                });
                            }
                        }
                        Err(e) => {
                            error!(error = %e, "Error receiving notification, attempting to reconnect");
                            match self.reconnect_listener(&shutdown).await {
                                Ok(l) => listener = l,
                                Err(e) if shutdown.is_cancelled() => break,
                                Err(e) => return Err(e),
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
        assert_eq!(queue_key("default", "main"), "default:main");
    }
}
