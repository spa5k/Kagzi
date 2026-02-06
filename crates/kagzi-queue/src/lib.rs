//! Kagzi queue signaling (work-signal bus).
//!
//! This crate provides a pluggable, lossy wakeup distribution mechanism used by Kagzi workers to
//! reduce DB polling. Signals are never claims; the authoritative store remains the only gateway
//! to execution.
//!
//! - Default backend: Postgres LISTEN/NOTIFY (`PostgresNotifier`)
//! - Future backends (not implemented yet): Kafka / NATS JetStream / RabbitMQ

mod bus;
mod error;
#[cfg(feature = "kafka")]
mod kafka;
#[cfg(feature = "nats")]
mod nats;
mod postgres;
mod registry;

pub use bus::{WorkAvailable, WorkSignalBus};
pub use error::QueueError;
#[cfg(feature = "kafka")]
pub use kafka::KafkaBus;
#[cfg(feature = "nats")]
pub use nats::NatsBus;
pub use postgres::PostgresNotifier;

/// Convenience enum for runtime selection of a work-signal bus backend.
///
/// Postgres remains the default; NATS/Kafka are optional and feature-gated.
#[derive(Clone)]
pub enum WorkBus {
    Postgres(PostgresNotifier),
    #[cfg(feature = "nats")]
    Nats(NatsBus),
    #[cfg(feature = "kafka")]
    Kafka(KafkaBus),
}

#[async_trait::async_trait]
impl WorkSignalBus for WorkBus {
    async fn publish(&self, namespace: &str, task_queue: &str) -> Result<(), QueueError> {
        match self {
            Self::Postgres(b) => b.publish(namespace, task_queue).await,
            #[cfg(feature = "nats")]
            Self::Nats(b) => b.publish(namespace, task_queue).await,
            #[cfg(feature = "kafka")]
            Self::Kafka(b) => b.publish(namespace, task_queue).await,
        }
    }

    fn subscribe(
        &self,
        namespace: &str,
        task_queue: &str,
    ) -> tokio::sync::broadcast::Receiver<WorkAvailable> {
        match self {
            Self::Postgres(b) => b.subscribe(namespace, task_queue),
            #[cfg(feature = "nats")]
            Self::Nats(b) => b.subscribe(namespace, task_queue),
            #[cfg(feature = "kafka")]
            Self::Kafka(b) => b.subscribe(namespace, task_queue),
        }
    }

    async fn start(&self, shutdown: tokio_util::sync::CancellationToken) -> Result<(), QueueError> {
        match self {
            Self::Postgres(b) => b.start(shutdown).await,
            #[cfg(feature = "nats")]
            Self::Nats(b) => b.start(shutdown).await,
            #[cfg(feature = "kafka")]
            Self::Kafka(b) => b.start(shutdown).await,
        }
    }
}
