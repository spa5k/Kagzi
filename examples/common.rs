use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use kagzi::{Kagzi, Retry, Worker};
use tokio::sync::Mutex;
use uuid::Uuid;

/// Connect a Kagzi client to the given server.
#[allow(dead_code)]
pub async fn connect_client(server: &str) -> Result<Kagzi> {
    Kagzi::connect(server).await
}

pub fn default_retry() -> Retry {
    Retry::exponential(3)
        .with_initial(Duration::from_millis(300))
        .with_max(Duration::from_secs(5))
}

/// Build a worker with sensible defaults (retry/backoff). Caller registers workflows.
#[allow(dead_code)]
pub async fn build_worker(server: &str, namespace: &str) -> Result<Worker> {
    Worker::new(server)
        .namespace(namespace)
        .retry(default_retry())
        .build()
        .await
}

/// A simple flaky helper that fails until a target attempt.
#[derive(Clone)]
#[allow(dead_code)]
pub struct FlakyStep {
    target: u32,
    counter: Arc<tokio::sync::Mutex<u32>>,
}

#[allow(dead_code)]
impl FlakyStep {
    pub fn succeed_after(attempts: u32) -> Self {
        Self {
            target: attempts.max(1),
            counter: Arc::new(tokio::sync::Mutex::new(0)),
        }
    }

    /// Returns Ok only on/after the configured attempt.
    pub async fn run(&self, label: &str) -> Result<String> {
        let mut guard = self.counter.lock().await;
        *guard += 1;
        let attempt = *guard;
        drop(guard);

        if attempt < self.target {
            anyhow::bail!("{label} failed on attempt {attempt}");
        }

        Ok(format!("{label} succeeded on attempt {attempt}"))
    }
}

/// In-memory blob store used by the data pipeline example.
#[derive(Clone, Default)]
#[allow(dead_code)]
pub struct InMemoryBlobStore {
    inner: Arc<Mutex<HashMap<String, Vec<u8>>>>,
}

#[allow(dead_code)]
impl InMemoryBlobStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn put(&self, bytes: Vec<u8>) -> String {
        let key = Uuid::now_v7().to_string();
        let mut map = self.inner.lock().await;
        map.insert(key.clone(), bytes);
        key
    }

    pub async fn get(&self, key: &str) -> Option<Vec<u8>> {
        let map = self.inner.lock().await;
        map.get(key).cloned()
    }
}
