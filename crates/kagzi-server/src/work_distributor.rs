use dashmap::DashMap;
use kagzi_store::{PgStore, WorkflowRepository};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use tracing::{error, info};
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
    supported_workflow_types: Vec<String>,
    response_tx: oneshot::Sender<Option<WorkItem>>,
}

pub struct WorkDistributor {
    store: PgStore,
    request_tx: mpsc::Sender<WorkRequest>,
    pending_requests: Arc<DashMap<(String, String), Vec<WorkRequest>>>,
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
            if let Some(mut waiters) = self
                .pending_requests
                .get_mut(&(namespace_id.clone(), task_queue.clone()))
            {
                let mut backlog = Vec::new();

                while let Some(request) = waiters.pop() {
                    match self
                        .store
                        .workflows()
                        .claim_next_filtered(
                            &task_queue,
                            &namespace_id,
                            &request.worker_id,
                            &request.supported_workflow_types,
                        )
                        .await
                    {
                        Ok(Some(item)) => {
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
                        Ok(None) => {
                            backlog.push(request);
                        }
                        Err(e) => {
                            error!(task_queue = %task_queue, namespace_id = %namespace_id, error = %e, "Failed to claim work");
                            backlog.push(request);
                        }
                    }
                }

                // Put back any waiters that did not receive work
                waiters.extend(backlog.into_iter());
            }
        }
    }

    pub async fn wait_for_work(
        &self,
        task_queue: &str,
        namespace_id: &str,
        worker_id: &str,
        supported_workflow_types: &[String],
        timeout: Duration,
    ) -> Option<WorkItem> {
        // Fast path: try to claim immediately without waiting for the loop to tick.
        match self
            .store
            .workflows()
            .claim_next_filtered(
                task_queue,
                namespace_id,
                worker_id,
                supported_workflow_types,
            )
            .await
        {
            Ok(Some(item)) => {
                return Some(WorkItem {
                    run_id: item.run_id,
                    task_queue: task_queue.to_string(),
                    namespace_id: namespace_id.to_string(),
                    workflow_type: item.workflow_type,
                    input: item.input,
                    locked_by: worker_id.to_string(),
                });
            }
            Ok(None) => {
                // No work right now; fall through to long-poll. Capture a quick snapshot for observability.
                if let Ok(count) = sqlx::query_scalar!(
                    r#"
                    SELECT COUNT(*)::BIGINT
                    FROM kagzi.workflow_runs
                    WHERE task_queue = $1
                      AND namespace_id = $2
                      AND workflow_type = ANY($3)
                      AND (
                        (status = 'PENDING' AND (wake_up_at IS NULL OR wake_up_at <= NOW()))
                        OR (status = 'SLEEPING' AND wake_up_at <= NOW())
                      )
                    "#,
                    task_queue,
                    namespace_id,
                    supported_workflow_types
                )
                .fetch_one(self.store.pool())
                .await
                {
                    info!(
                        task_queue = task_queue,
                        namespace_id = namespace_id,
                        supported = ?supported_workflow_types,
                        pending = count,
                        "Immediate claim miss; falling back to long-poll"
                    );
                }
            }
            Err(e) => {
                error!(
                    task_queue = task_queue,
                    namespace_id = namespace_id,
                    error = %e,
                    "Immediate claim_next_filtered failed; falling back to long-poll"
                );
            }
        }

        let (response_tx, response_rx) = oneshot::channel();

        let request = WorkRequest {
            task_queue: task_queue.to_string(),
            namespace_id: namespace_id.to_string(),
            worker_id: worker_id.to_string(),
            supported_workflow_types: supported_workflow_types.to_vec(),
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
        supported_workflow_types: &[String],
        timeout: Duration,
    ) -> Option<WorkItem> {
        self.inner
            .wait_for_work(
                task_queue,
                namespace_id,
                worker_id,
                supported_workflow_types,
                timeout,
            )
            .await
    }

    pub fn shutdown(&self) {
        self.shutdown.cancel();
    }
}
