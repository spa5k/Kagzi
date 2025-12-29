//! Core trait for queue notification backends.

use async_trait::async_trait;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::QueueError;

/// Trait for queue notification backends.
///
/// Implementations signal when work is available on a queue and allow
/// consumers to subscribe for notifications.
///
/// ### Lifecycle
/// 1. Initialize the implementation (e.g., `PostgresNotifier::new()`).
/// 2. Call `start()` exactly once in a background task to process incoming signals.
/// 3. Call `notify()` when new work is produced (added to a table/external queue).
/// 4. Call `subscribe()` on the consumer side (e.g., in a long-polling loop) to wait for notifications.
///
/// ### Guarantees
/// This is a "best-effort" signaling mechanism. While implementations strive for low latency,
/// notifications can be lost (especially during reconnection). Consumers MUST implement
/// a fallback polling mechanism with an appropriate timeout to ensure eventual consistency.
#[async_trait]
pub trait QueueNotifier: Send + Sync + Clone {
    /// Signal that work is available on the specified queue.
    async fn notify(&self, namespace: &str, task_queue: &str) -> Result<(), QueueError>;

    /// Subscribe to notifications for a queue.
    /// Returns a receiver that will receive `()` when work may be available.
    fn subscribe(&self, namespace: &str, task_queue: &str) -> broadcast::Receiver<()>;

    /// Start the background listener task.
    /// Call once at server startup.
    async fn start(&self, shutdown: CancellationToken) -> Result<(), QueueError>;
}
