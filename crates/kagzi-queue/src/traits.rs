use async_trait::async_trait;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::QueueError;

#[async_trait]
pub trait QueueNotifier: Send + Sync + Clone {
    async fn notify(&self, namespace: &str, task_queue: &str) -> Result<(), QueueError>;

    fn subscribe(&self, namespace: &str, task_queue: &str) -> broadcast::Receiver<()>;

    async fn start(&self, shutdown: CancellationToken) -> Result<(), QueueError>;
}
