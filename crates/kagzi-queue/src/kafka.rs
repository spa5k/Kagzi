use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;
use futures::StreamExt;
use rdkafka::ClientConfig;
use rdkafka::Message;
use rdkafka::consumer::{CommitMode, Consumer, StreamConsumer};
use rdkafka::producer::{FutureProducer, FutureRecord};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::QueueError;
use crate::bus::{WorkAvailable, WorkSignalBus};

#[derive(Clone)]
pub struct KafkaBus {
    producer: FutureProducer,
    brokers: Arc<str>,
    topic: Arc<str>,
    channels: Arc<DashMap<String, broadcast::Sender<WorkAvailable>>>,
    channel_capacity: usize,
    group_id_prefix: Arc<str>,
}

impl KafkaBus {
    pub fn new(
        brokers: impl Into<String>,
        topic: impl Into<String>,
        channel_capacity: usize,
        group_id_prefix: impl Into<String>,
    ) -> Result<Self, QueueError> {
        let brokers = brokers.into();
        let topic = topic.into();

        let producer: FutureProducer = ClientConfig::new()
            .set("bootstrap.servers", &brokers)
            .set("message.timeout.ms", "5000")
            .create()
            .map_err(|e| QueueError::Other(e.to_string()))?;

        Ok(Self {
            producer,
            brokers: Arc::from(brokers),
            topic: Arc::from(topic),
            channels: Arc::new(DashMap::new()),
            channel_capacity,
            group_id_prefix: Arc::from(group_id_prefix.into()),
        })
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
impl WorkSignalBus for KafkaBus {
    async fn publish(&self, namespace: &str, task_queue: &str) -> Result<(), QueueError> {
        let key = Self::key(namespace, task_queue);
        let record = FutureRecord::to(self.topic.as_ref())
            .key(key.as_str())
            .payload(b"");

        self.producer
            .send(record, std::time::Duration::from_secs(2))
            .await
            .map_err(|(e, _)| QueueError::Other(e.to_string()))?;

        Ok(())
    }

    fn subscribe(&self, namespace: &str, task_queue: &str) -> broadcast::Receiver<WorkAvailable> {
        let key = Self::key(namespace, task_queue);
        let tx = self.get_or_create_channel(&key);
        let rx = tx.subscribe();

        if tx.receiver_count() == 1 {
            let brokers = self.brokers.clone();
            let topic = self.topic.clone();
            let group_id = format!("{}-{}", self.group_id_prefix, key.replace(':', "_"));
            let tx2 = tx.clone();
            let ns = namespace.to_string();
            let tq = task_queue.to_string();

            tokio::spawn(async move {
                let mut delay_ms: u64 = 200;
                loop {
                    let consumer: StreamConsumer = match ClientConfig::new()
                        .set("bootstrap.servers", brokers.as_ref())
                        .set("group.id", &group_id)
                        .set("enable.auto.commit", "false")
                        .set("auto.offset.reset", "earliest")
                        .create()
                    {
                        Ok(c) => c,
                        Err(e) => {
                            warn!(error = %e, "Failed to create Kafka consumer, retrying");
                            tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                            delay_ms = (delay_ms.saturating_mul(2)).min(10_000);
                            continue;
                        }
                    };

                    if let Err(e) = consumer.subscribe(&[topic.as_ref()]) {
                        warn!(error = %e, "Failed to subscribe to Kafka topic, retrying");
                        tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                        delay_ms = (delay_ms.saturating_mul(2)).min(10_000);
                        continue;
                    }

                    delay_ms = 200;
                    info!(
                        namespace = %ns,
                        task_queue = %tq,
                        group_id = %group_id,
                        "Kafka wakeup subscription started"
                    );

                    let mut stream = consumer.stream();
                    while let Some(msg) = stream.next().await {
                        let msg = match msg {
                            Ok(m) => m,
                            Err(e) => {
                                warn!(error = %e, "Kafka consume error");
                                break;
                            }
                        };

                        let Some(k) = msg.key() else {
                            let _ = consumer.commit_message(&msg, CommitMode::Async);
                            continue;
                        };

                        if k == key.as_bytes() {
                            let _ = tx2.send(WorkAvailable {
                                namespace: ns.clone(),
                                task_queue: tq.clone(),
                            });
                        }

                        let _ = consumer.commit_message(&msg, CommitMode::Async);
                    }

                    warn!(namespace = %ns, task_queue = %tq, "Kafka subscription ended, recreating consumer");
                }
            });
        }

        rx
    }

    async fn start(&self, _shutdown: CancellationToken) -> Result<(), QueueError> {
        // Kafka subscriptions are started lazily on first subscribe().
        Ok(())
    }
}
