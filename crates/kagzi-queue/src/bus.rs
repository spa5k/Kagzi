use async_trait::async_trait;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::QueueError;

/// A lossy wakeup signal indicating that a `{namespace, task_queue}` may have runnable work.
///
/// This is *not* a claim. Postgres (or the authoritative store) remains the source of truth.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkAvailable {
    pub namespace: String,
    pub task_queue: String,
}

pub(crate) fn queue_key(namespace: &str, task_queue: &str) -> String {
    format!("{namespace}:{task_queue}")
}

/// Work-signal bus for distributing wakeups to workers.
///
/// # Core invariant
///
/// Delivery of a `WorkAvailable` signal is never a claim. The authoritative store
/// (Postgres in Kagzi today) is the only gateway to execution via an atomic lease/claim.
///
/// # Semantics
///
/// - Signals are **lossy** and **at-least-once** (duplicates are normal and safe).
/// - `publish()` must be best-effort; missed signals must not break correctness.
/// - Workers should ack/commit broker messages immediately after triggering a claim attempt;
///   execution must not be tied to broker visibility/ack deadlines.
///
/// # Future external backends (Kafka / NATS JetStream / RabbitMQ)
///
/// This crate deliberately keeps the API small so future backends can be implemented
/// without changing Kagzi correctness:
///
/// - Kafka: key records by `namespace:task_queue` (or structured key), consume via consumer groups.
/// - NATS JetStream: subject like `kagzi.work.<namespace>.<task_queue>` with queue groups.
/// - RabbitMQ: queue per `{namespace, task_queue}` or topic exchange with routing keys.
///
/// In all cases, the handler for a signal should call the server's `ClaimTask` (or equivalent)
/// to perform the DB claim and then execute only if the claim succeeds.
#[async_trait]
pub trait WorkSignalBus: Send + Sync + Clone {
    /// Publish a best-effort wakeup signal for the specified queue.
    async fn publish(&self, namespace: &str, task_queue: &str) -> Result<(), QueueError>;

    /// Subscribe to wakeups for the specified queue.
    ///
    /// Receivers may lag and miss signals; duplicates are normal.
    fn subscribe(&self, namespace: &str, task_queue: &str) -> broadcast::Receiver<WorkAvailable>;

    /// Start the backend listener loop (e.g. Postgres LISTEN/NOTIFY bridge).
    async fn start(&self, shutdown: CancellationToken) -> Result<(), QueueError>;
}
