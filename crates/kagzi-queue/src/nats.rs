use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;
use futures::StreamExt;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::QueueError;
use crate::bus::{WorkAvailable, WorkSignalBus};

#[derive(Clone)]
pub struct NatsBus {
    client: async_nats::Client,
    subject_prefix: Arc<str>,
    channels: Arc<DashMap<String, broadcast::Sender<WorkAvailable>>>,
    channel_capacity: usize,
    queue_group: Option<Arc<str>>,
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
            channels: Arc::new(DashMap::new()),
            channel_capacity,
            queue_group: queue_group.map(Arc::from),
        })
    }

    fn subject(&self, namespace: &str, task_queue: &str) -> String {
        // Keep subjects simple: prefix.namespace.task_queue
        // Workflow/queue names in Kagzi are expected to be snake_case; we avoid extra encoding here.
        format!("{}.{}.{}", self.subject_prefix, namespace, task_queue)
    }

    fn key(namespace: &str, task_queue: &str) -> String {
        format!("{namespace}:{task_queue}")
    }

    fn get_or_create_channel(&self, key: &str) -> broadcast::Sender<WorkAvailable> {
        self.channels
            .entry(key.to_string())
            .or_insert_with(|| {
                let (tx, _) = broadcast::channel(self.channel_capacity);
                tx
            })
            .clone()
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

        let tx = self.get_or_create_channel(&key);
        let rx = tx.subscribe();

        // Spawn per-queue subscription the first time we see this key.
        // We detect "first time" by checking receiver_count before creating; race is acceptable.
        if tx.receiver_count() == 1 {
            let client = self.client.clone();
            let tx2 = tx.clone();
            let ns = namespace.to_string();
            let tq = task_queue.to_string();
            let group = self.queue_group.clone();

            tokio::spawn(async move {
                let mut delay_ms: u64 = 100;
                loop {
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
                            tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                            delay_ms = (delay_ms.saturating_mul(2)).min(10_000);
                            continue;
                        }
                    };

                    delay_ms = 100;
                    info!(namespace = %ns, task_queue = %tq, "NATS wakeup subscription started");

                    while let Some(_msg) = sub.next().await {
                        let _ = tx2.send(WorkAvailable {
                            namespace: ns.clone(),
                            task_queue: tq.clone(),
                        });
                    }

                    warn!(namespace = %ns, task_queue = %tq, "NATS subscription ended, resubscribing");
                }
            });
        }

        rx
    }

    async fn start(&self, _shutdown: CancellationToken) -> Result<(), QueueError> {
        // NATS subscriptions are started lazily on first subscribe().
        Ok(())
    }
}
