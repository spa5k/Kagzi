//! PostgreSQL implementation of QueueNotifier using NOTIFY/LISTEN.

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

/// Channel capacity for broadcast senders.
const CHANNEL_CAPACITY: usize = 64;

/// PostgreSQL-based queue notifier.
///
/// Uses a single shared `PgListener` to receive notifications and broadcasts
/// them to subscribers via `tokio::sync::broadcast` channels.
#[derive(Clone)]
pub struct PostgresNotifier {
    pool: PgPool,
    /// Map "namespace:queue" -> broadcast sender
    channels: Arc<DashMap<String, broadcast::Sender<()>>>,
}

impl PostgresNotifier {
    /// Create a new PostgresNotifier.
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            channels: Arc::new(DashMap::new()),
        }
    }

    /// Build the queue key from namespace and task_queue.
    fn queue_key(namespace: &str, task_queue: &str) -> String {
        format!("{}:{}", namespace, task_queue)
    }

    /// Get or create a broadcast channel for the given queue key.
    fn get_or_create_channel(&self, key: &str) -> broadcast::Sender<()> {
        self.channels
            .entry(key.to_string())
            .or_insert_with(|| {
                let (tx, _) = broadcast::channel(CHANNEL_CAPACITY);
                tx
            })
            .clone()
    }
}

#[async_trait]
impl QueueNotifier for PostgresNotifier {
    async fn notify(&self, namespace: &str, task_queue: &str) -> Result<(), QueueError> {
        let key = Self::queue_key(namespace, task_queue);

        // Send NOTIFY to PostgreSQL
        sqlx::query("SELECT pg_notify('kagzi_work', $1)")
            .bind(&key)
            .execute(&self.pool)
            .await?;

        debug!(queue = %key, "Sent pg_notify");

        // Also broadcast locally in case listener hasn't picked it up yet
        if let Some(tx) = self.channels.get(&key) {
            let _ = tx.send(()); // Ignore error if no receivers
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

        loop {
            tokio::select! {
                biased;

                _ = shutdown.cancelled() => {
                    info!("Queue listener shutting down");
                    break;
                }

                result = listener.recv() => {
                    match result {
                        Ok(notification) => {
                            let key = notification.payload();
                            debug!(queue = %key, "Received pg_notify");

                            if let Some(tx) = self.channels.get(key) {
                                // Broadcast to all subscribers
                                // Ignore send errors (no receivers is fine)
                                let _ = tx.send(());
                            }
                        }
                        Err(e) => {
                            error!(error = %e, "Error receiving notification, attempting to reconnect");
                            // Attempt to reconnect
                            match PgListener::connect_with(&self.pool).await {
                                Ok(mut new_listener) => {
                                    if let Err(e) = new_listener.listen("kagzi_work").await {
                                        warn!(error = %e, "Failed to re-listen after reconnect");
                                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                                        continue;
                                    }
                                    listener = new_listener;
                                    info!("Queue listener reconnected");
                                }
                                Err(e) => {
                                    warn!(error = %e, "Failed to reconnect listener, retrying in 1s");
                                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
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
