use kagzi_store::{PgStore, WorkflowRepository};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use tracing::{error, info};
use uuid::Uuid;

const WORKFLOW_LOCK_DURATION_SECS: i64 = 30;

type PendingRequests = HashMap<(String, String), Vec<WorkRequest>>;

#[derive(Debug, Clone)]
pub struct WorkItem {
    pub run_id: Uuid,
    pub task_queue: String,
    pub namespace_id: String,
    pub workflow_type: String,
    pub input: Vec<u8>,
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
    pending_requests: Arc<Mutex<PendingRequests>>,
    shutdown: CancellationToken,
}

impl WorkDistributor {
    fn new(store: PgStore, shutdown: CancellationToken) -> (Arc<Self>, mpsc::Sender<WorkRequest>) {
        let (request_tx, request_rx) = mpsc::channel::<WorkRequest>(1000);

        let distributor = Arc::new(Self {
            store,
            request_tx: request_tx.clone(),
            pending_requests: Arc::new(Mutex::new(HashMap::new())),
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

        loop {
            tokio::select! {
                _ = self.shutdown.cancelled() => {
                    info!("Work distributor shutting down");
                    self.drain_pending(None).await;
                    break;
                }
                maybe_request = request_rx.recv() => {
                    match maybe_request {
                        Some(request) => {
                            let key = (request.namespace_id.clone(), request.task_queue.clone());
                            {
                                let mut pending = self.pending_requests.lock().await;
                                pending.entry(key.clone()).or_default().push(request);
                            }
                            Arc::clone(&self).process_queue(key).await;
                        }
                        None => {
                            info!("Work distributor channel closed");
                            self.drain_pending(None).await;
                            break;
                        }
                    }
                }
            }
        }
    }

    #[allow(clippy::manual_async_fn)]
    fn process_queue(
        self: Arc<Self>,
        key: (String, String),
    ) -> impl std::future::Future<Output = ()> + Send {
        async move {
            let requests = {
                let mut pending = self.pending_requests.lock().await;
                let entry = pending.entry(key.clone()).or_default();
                if entry.is_empty() {
                    return;
                }
                std::mem::take(entry)
            };

            let mut remaining = Vec::new();

            for request in requests {
                match self
                    .store
                    .workflows()
                    .claim_workflow_batch(
                        &key.1,
                        &key.0,
                        &request.worker_id,
                        &request.supported_workflow_types,
                        1,
                        WORKFLOW_LOCK_DURATION_SECS,
                    )
                    .await
                {
                    Ok(mut claimed) => {
                        if let Some(claimed_workflow) = claimed.pop() {
                            let work_item = WorkItem {
                                run_id: claimed_workflow.run_id,
                                task_queue: key.1.clone(),
                                namespace_id: key.0.clone(),
                                workflow_type: claimed_workflow.workflow_type,
                                input: claimed_workflow.input,
                                locked_by: request.worker_id.clone(),
                            };
                            let _ = request.response_tx.send(Some(work_item));
                        } else {
                            remaining.push(request);
                        }
                    }
                    Err(e) => {
                        error!(
                            task_queue = %key.1,
                            namespace_id = %key.0,
                            error = ?e,
                            "Failed to claim workflow batch"
                        );
                        remaining.push(request);
                    }
                }
            }

            if !remaining.is_empty() {
                {
                    let mut pending = self.pending_requests.lock().await;
                    pending.entry(key.clone()).or_default().extend(remaining);
                }

                let this = Arc::clone(&self);
                tokio::spawn(this.process_queue(key));
            }
        }
    }

    async fn drain_pending(&self, response: Option<WorkItem>) {
        let mut pending = self.pending_requests.lock().await;
        for (_, waiters) in pending.drain() {
            for waiter in waiters {
                let _ = waiter.response_tx.send(response.clone());
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

#[allow(dead_code)]
fn _assert_send_sync() {
    fn assert_send<T: Send>() {}
    fn assert_sync<T: Sync>() {}
    assert_send::<Arc<WorkDistributor>>();
    assert_sync::<Arc<WorkDistributor>>();
    assert_send::<WorkRequest>();
    assert_send::<WorkItem>();
}
