use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use dashmap::DashMap;
use tokio::sync::broadcast;

use crate::bus::WorkAvailable;

#[derive(Debug)]
struct ChannelEntry {
    tx: broadcast::Sender<WorkAvailable>,
    started: AtomicBool,
}

#[derive(Clone)]
pub(crate) struct ChannelRegistry {
    entries: Arc<DashMap<String, Arc<ChannelEntry>>>,
    channel_capacity: usize,
}

impl ChannelRegistry {
    pub(crate) fn new(channel_capacity: usize) -> Self {
        Self {
            entries: Arc::new(DashMap::new()),
            channel_capacity,
        }
    }

    fn get_or_create(&self, key: &str) -> Arc<ChannelEntry> {
        self.entries
            .entry(key.to_string())
            .or_insert_with(|| {
                let (tx, _) = broadcast::channel(self.channel_capacity);
                Arc::new(ChannelEntry {
                    tx,
                    started: AtomicBool::new(false),
                })
            })
            .clone()
    }

    pub(crate) fn subscribe(&self, key: &str) -> broadcast::Receiver<WorkAvailable> {
        self.get_or_create(key).tx.subscribe()
    }

    pub(crate) fn subscribe_start_once(
        &self,
        key: &str,
    ) -> (
        broadcast::Sender<WorkAvailable>,
        broadcast::Receiver<WorkAvailable>,
        bool,
    ) {
        let entry = self.get_or_create(key);
        let tx = entry.tx.clone();
        let rx = tx.subscribe();
        let first = !entry.started.swap(true, Ordering::AcqRel);
        (tx, rx, first)
    }

    pub(crate) fn try_send_queue(&self, key: &str, namespace: &str, task_queue: &str) {
        if let Some(entry) = self.entries.get(key) {
            let _ = entry.tx.send(WorkAvailable {
                namespace: namespace.to_string(),
                task_queue: task_queue.to_string(),
            });
        }
    }

    pub(crate) fn remove(&self, key: &str) {
        self.entries.remove(key);
    }

    pub(crate) fn cleanup_stale(&self) -> usize {
        let mut removed = 0;
        self.entries.retain(|key, entry| {
            if entry.tx.receiver_count() == 0 {
                tracing::debug!(queue = %key, "Removing stale channel for queue");
                removed += 1;
                false
            } else {
                true
            }
        });
        removed
    }
}
