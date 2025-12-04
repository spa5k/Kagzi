use dashmap::DashMap;
use kagzi_store::{PgStore, WorkflowRepository};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct WorkItem {
    pub run_id: Uuid,
    pub task_queue: String,
    pub namespace_id: String,
    pub workflow_type: String,
    pub input: serde_json::Value,
    pub locked_by: String,
}

struct WorkRequest {
    task_queue: String,
    namespace_id: String,
    worker_id: String,
    response_tx: oneshot::Sender<Option<WorkItem>>,
}

pub struct WorkDistributor {
    store: PgStore,
    request_tx: mpsc::Sender<WorkRequest>,
    pending_requests: Arc<DashMap<(String, String), Vec<WorkRequest>>>,
    batch_size: i32,
    poll_interval: Duration,
    shutdown: CancellationToken,
}

impl WorkDistributor {
    fn new(store: PgStore, shutdown: CancellationToken) -> (Arc<Self>, mpsc::Sender<WorkRequest>) {
        let (request_tx, request_rx) = mpsc::channel::<WorkRequest>(1000);

        let distributor = Arc::new(Self {
            store,
            request_tx: request_tx.clone(),
            pending_requests: Arc::new(DashMap::new()),
            batch_size: 50,
            poll_interval: Duration::from_millis(100),
            shutdown: shutdown.clone(),
        });

        let distributor_clone = distributor.clone();
        tokio::spawn(async move {
            distributor_clone.run_distribution_loop(request_rx).await;
        });

        (distributor, request_tx)
    }

    async fn run_distribution_loop(self: Arc<Self>, mut request_rx: mpsc::Receiver<WorkRequest>) {
        info!("Work distributor started");

        let mut interval = tokio::time::interval(self.poll_interval);

        loop {
            tokio::select! {
                _ = self.shutdown.cancelled() => {
                    info!("Work distributor shutting down");
                    break;
                }
                Some(request) = request_rx.recv() => {
                    let key = (request.namespace_id.clone(), request.task_queue.clone());
                    self.pending_requests
                        .entry(key)
                        .or_default()
                        .push(request);
                }
                _ = interval.tick() => {
                    self.distribute_work().await;
                }
            }
        }
    }

    async fn distribute_work(&self) {
        let queues_with_waiters: Vec<(String, String)> = self
            .pending_requests
            .iter()
            .filter(|entry| !entry.value().is_empty())
            .map(|entry| entry.key().clone())
            .collect();

        if queues_with_waiters.is_empty() {
            return;
        }

        for (namespace_id, task_queue) in queues_with_waiters {
            let waiter_count = self
                .pending_requests
                .get(&(namespace_id.clone(), task_queue.clone()))
                .map(|v| v.len())
                .unwrap_or(0);

            if waiter_count == 0 {
                continue;
            }

            let batch_size = (waiter_count as i32).min(self.batch_size);

            match self
                .store
                .workflows()
                .claim_batch(&task_queue, &namespace_id, batch_size)
                .await
            {
                Ok(claimed_items) => {
                    if !claimed_items.is_empty() {
                        debug!(
                            task_queue = %task_queue,
                            namespace_id = %namespace_id,
                            count = claimed_items.len(),
                            "Claimed batch"
                        );
                    }

                    for item in claimed_items {
                        if let Some(mut waiters) = self
                            .pending_requests
                            .get_mut(&(namespace_id.clone(), task_queue.clone()))
                            && let Some(request) = waiters.pop()
                        {
                            let batch_worker_id = item.locked_by.clone().unwrap_or_default();
                            if let Err(e) = self
                                .store
                                .workflows()
                                .transfer_lock(item.run_id, &batch_worker_id, &request.worker_id)
                                .await
                            {
                                warn!(run_id = %item.run_id, error = %e, "Failed to transfer lock");
                                continue;
                            }

                            let work_item = WorkItem {
                                run_id: item.run_id,
                                task_queue: task_queue.clone(),
                                namespace_id: namespace_id.clone(),
                                workflow_type: item.workflow_type,
                                input: item.input,
                                locked_by: request.worker_id.clone(),
                            };

                            let _ = request.response_tx.send(Some(work_item));
                        }
                    }
                }
                Err(e) => {
                    error!(task_queue = %task_queue, namespace_id = %namespace_id, error = %e, "Failed to claim batch");
                }
            }
        }
    }

    pub async fn wait_for_work(
        &self,
        task_queue: &str,
        namespace_id: &str,
        worker_id: &str,
        timeout: Duration,
    ) -> Option<WorkItem> {
        let (response_tx, response_rx) = oneshot::channel();

        let request = WorkRequest {
            task_queue: task_queue.to_string(),
            namespace_id: namespace_id.to_string(),
            worker_id: worker_id.to_string(),
            response_tx,
        };

        if self.request_tx.send(request).await.is_err() {
            error!("Work distributor channel closed");
            return None;
        }

        match tokio::time::timeout(timeout, response_rx).await {
            Ok(Ok(work)) => work,
            _ => None,
        }
    }
}

#[derive(Clone)]
pub struct WorkDistributorHandle {
    inner: Arc<WorkDistributor>,
    shutdown: CancellationToken,
}

impl WorkDistributorHandle {
    pub fn new(store: PgStore) -> Self {
        let shutdown = CancellationToken::new();
        let (distributor, _) = WorkDistributor::new(store, shutdown.clone());
        Self {
            inner: distributor,
            shutdown,
        }
    }

    pub async fn wait_for_work(
        &self,
        task_queue: &str,
        namespace_id: &str,
        worker_id: &str,
        timeout: Duration,
    ) -> Option<WorkItem> {
        self.inner
            .wait_for_work(task_queue, namespace_id, worker_id, timeout)
            .await
    }

    pub fn shutdown(&self) {
        self.shutdown.cancel();
    }
}
