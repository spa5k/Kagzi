use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;
use tokio::sync::OnceCell;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::QueueError;
use crate::bus::{WorkAvailable, WorkSignalBus, queue_key};
use crate::registry::ChannelRegistry;

#[derive(Clone)]
pub struct NatsBus {
    client: async_nats::Client,
    subject_prefix: Arc<str>,
    registry: ChannelRegistry,
    queue_group: Option<Arc<str>>,
    shutdown: Arc<OnceCell<CancellationToken>>,
}

impl NatsBus {
    pub async fn connect(
        url: impl AsRef<str>,
        subject_prefix: impl Into<String>,
        channel_capacity: usize,
        queue_group: Option<String>,
    ) -> Result<Self, QueueError> {
        let mut url = url.as_ref().to_string();
        if !url.contains("://") {
            url = format!("nats://{url}");
        }

        let client = async_nats::connect(&url)
            .await
            .map_err(|e| QueueError::Other(format!("nats connect to '{url}' failed: {e}")))?;
        Ok(Self {
            client,
            subject_prefix: Arc::from(subject_prefix.into()),
            registry: ChannelRegistry::new(channel_capacity),
            queue_group: queue_group.map(Arc::from),
            shutdown: Arc::new(OnceCell::new()),
        })
    }

    fn subject(&self, namespace: &str, task_queue: &str) -> String {
        // Keep subjects simple: prefix.namespace.task_queue
        // Workflow/queue names in Kagzi are expected to be snake_case; we avoid extra encoding here.
        format!("{}.{}.{}", self.subject_prefix, namespace, task_queue)
    }

    fn key(namespace: &str, task_queue: &str) -> String {
        queue_key(namespace, task_queue)
    }

    fn shutdown_token(&self) -> CancellationToken {
        self.shutdown
            .get()
            .cloned()
            .unwrap_or_else(CancellationToken::new)
    }
}

#[async_trait]
impl WorkSignalBus for NatsBus {
    async fn publish(&self, namespace: &str, task_queue: &str) -> Result<(), QueueError> {
        let subject = self.subject(namespace, task_queue);
        // Payload is intentionally empty; the subject identifies the queue.
        self.client
            .publish(subject, Vec::new().into())
            .await
            .map_err(|e| QueueError::Other(e.to_string()))?;
        Ok(())
    }

    fn subscribe(&self, namespace: &str, task_queue: &str) -> broadcast::Receiver<WorkAvailable> {
        let key = Self::key(namespace, task_queue);
        let subject = self.subject(namespace, task_queue);

        let (tx, rx, first) = self.registry.subscribe_start_once(&key);

        if first {
            let client = self.client.clone();
            let ns = namespace.to_string();
            let tq = task_queue.to_string();
            let group = self.queue_group.clone();
            let shutdown = self.shutdown_token();
            let registry = self.registry.clone();
            let key2 = key.clone();

            tokio::spawn(async move {
                let mut delay_ms: u64 = 100;
                let mut gc = tokio::time::interval(std::time::Duration::from_secs(30));
                loop {
                    if tx.receiver_count() == 0 {
                        registry.remove(&key2);
                        break;
                    }

                    let sub_res = match &group {
                        Some(g) => {
                            client
                                .queue_subscribe(subject.clone(), g.as_ref().to_string())
                                .await
                        }
                        None => client.subscribe(subject.clone()).await,
                    };

                    let mut sub = match sub_res {
                        Ok(s) => s,
                        Err(e) => {
                            warn!(error = %e, "Failed to subscribe to NATS subject, retrying");
                            tokio::select! {
                                _ = shutdown.cancelled() => {
                                    registry.remove(&key2);
                                    return;
                                }
                                _ = tokio::time::sleep(std::time::Duration::from_millis(delay_ms)) => {}
                            }
                            delay_ms = (delay_ms.saturating_mul(2)).min(10_000);
                            continue;
                        }
                    };

                    delay_ms = 100;
                    info!(namespace = %ns, task_queue = %tq, "NATS wakeup subscription started");

                    loop {
                        tokio::select! {
                            _ = shutdown.cancelled() => {
                                registry.remove(&key2);
                                return;
                            }
                            _ = gc.tick() => {
                                if tx.receiver_count() == 0 {
                                    registry.remove(&key2);
                                    return;
                                }
                            }
                            msg = sub.next() => {
                                if msg.is_none() {
                                    break;
                                }
                                let _ = tx.send(WorkAvailable {
                                    namespace: ns.clone(),
                                    task_queue: tq.clone(),
                                });
                            }
                        }
                    }

                    warn!(namespace = %ns, task_queue = %tq, "NATS subscription ended, resubscribing");
                }
            });
        }

        rx
    }

    async fn start(&self, shutdown: CancellationToken) -> Result<(), QueueError> {
        let _ = self.shutdown.set(shutdown);
        // NATS subscriptions are started lazily on first subscribe().
        Ok(())
    }
}
