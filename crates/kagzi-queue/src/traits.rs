use async_trait::async_trait;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::QueueError;

/// Provides notification capabilities for workflow queue events.
///
/// This trait defines the interface for notifying workers when new work is available
/// on a queue, and for workers to subscribe to those notifications. Implementations
/// should provide both database-backed notifications (for cross-process communication)
/// and in-memory channels (for local worker notification).
///
/// # Purpose
///
/// The queue notification system enables event-driven work distribution, allowing
/// workers to be notified immediately when workflows are enqueued rather than
/// polling for work. This reduces latency and database load.
///
/// # Implementation Notes
///
/// - Implementations must be thread-safe (`Send + Sync`) and cloneable
/// - The `notify` method should be best-effort; failures should be logged but not fatal
/// - The `subscribe` method returns a broadcast receiver that workers can use to
///   wait for notifications
/// - The `start` method runs the listener loop and should handle reconnections gracefully
///
/// # Example
///
/// ```no_run
/// use kagzi_queue::{PostgresNotifier, QueueNotifier};
/// use tokio_util::sync::CancellationToken;
///
/// async fn example(pool: sqlx::PgPool) {
///     let queue = PostgresNotifier::new(pool, 300, 300);
///     let shutdown = CancellationToken::new();
///     
///     // Start the background listener
///     let queue_listener = queue.clone();
///     tokio::spawn(async move {
///         queue_listener.start(shutdown).await.ok();
///     });
///     
///     // Notify that work is available
///     queue.notify("default", "main-queue").await.ok();
///     
///     // Subscribe to notifications
///     let mut rx = queue.subscribe("default", "main-queue");
///     rx.recv().await.ok();
/// }
/// ```
#[async_trait]
pub trait QueueNotifier: Send + Sync + Clone {
    /// Sends a notification that work is available for the specified queue.
    ///
    /// This method notifies both external processes (via database mechanisms like
    /// PostgreSQL's LISTEN/NOTIFY) and local subscribers (via in-memory channels).
    ///
    /// # Arguments
    ///
    /// * `namespace` - The namespace identifier for the queue
    /// * `task_queue` - The name of the task queue where work is available
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` if the notification was sent successfully, or a `QueueError`
    /// if the notification failed. Implementations should log errors but may choose
    /// to treat notification failures as non-fatal.
    ///
    /// # Errors
    ///
    /// - `QueueError::Database` - Database connection or query failure
    /// - `QueueError::Other` - Other notification delivery failures
    async fn notify(&self, namespace: &str, task_queue: &str) -> Result<(), QueueError>;

    /// Subscribes to notifications for the specified queue.
    ///
    /// Returns a broadcast receiver that will receive a unit value `()` whenever
    /// work becomes available on the specified queue. Workers should poll for work
    /// after receiving a notification.
    ///
    /// # Arguments
    ///
    /// * `namespace` - The namespace identifier for the queue
    /// * `task_queue` - The name of the task queue to subscribe to
    ///
    /// # Returns
    ///
    /// A `broadcast::Receiver<()>` that receives notifications when work is available.
    /// The channel may be closed if all senders are dropped or the queue is cleaned up.
    ///
    /// # Notes
    ///
    /// - Receivers may miss notifications if they don't keep up with the send rate
    /// - Lagging receivers will receive a `RecvError::Lagged` error
    /// - The channel is created on first subscription and may be cleaned up if unused
    fn subscribe(&self, namespace: &str, task_queue: &str) -> broadcast::Receiver<()>;

    /// Starts the queue listener background task.
    ///
    /// This method runs a long-lived listener that receives notifications from the
    /// database and forwards them to local subscribers. It should handle reconnections
    /// gracefully and only exit when the shutdown token is cancelled.
    ///
    /// # Arguments
    ///
    /// * `shutdown` - A cancellation token to signal graceful shutdown
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` on clean shutdown, or a `QueueError` if the listener
    /// encounters a fatal error that cannot be recovered.
    ///
    /// # Errors
    ///
    /// - `QueueError::Database` - Failed to connect or reconnect to the database
    /// - `QueueError::Other` - Exceeded maximum reconnection attempts
    ///
    /// # Behavior
    ///
    /// - Should automatically reconnect on transient failures
    /// - Must respect the shutdown token and exit cleanly
    /// - Should periodically clean up stale channels to prevent memory leaks
    async fn start(&self, shutdown: CancellationToken) -> Result<(), QueueError>;
}
