use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use anyhow::anyhow;
use kagzi_proto::kagzi::worker_service_client::WorkerServiceClient;
use kagzi_proto::kagzi::{
    CompleteWorkflowRequest, DeregisterRequest, ErrorCode, FailWorkflowRequest, HeartbeatRequest,
    Payload as ProtoPayload, PollTaskRequest, RegisterRequest,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;
use tonic::Request;
use tonic::transport::Channel;
use tracing::{Instrument, error, info, info_span, warn};
use uuid::Uuid;

use crate::context::Context;
use crate::errors::{KagziError, WorkflowPaused, map_grpc_error};
use crate::propagation::inject_context;
use crate::retry::Retry;

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

type WorkflowFn = Box<
    dyn Fn(Context, serde_json::Value) -> BoxFuture<'static, anyhow::Result<serde_json::Value>>
        + Send
        + Sync,
>;

const DEFAULT_MAX_CONCURRENT_WORKFLOWS: usize = 100;

pub struct WorkerBuilder {
    addr: String,
    namespace_id: String,
    max_concurrent: usize,
    default_retry: Option<Retry>,
    hostname: Option<String>,
    version: Option<String>,
    labels: HashMap<String, String>,
    workflows: Vec<(String, Arc<WorkflowFn>)>,
}

impl WorkerBuilder {
    pub fn new(addr: &str) -> Self {
        Self {
            addr: addr.to_string(),
            namespace_id: "default".to_string(),
            max_concurrent: DEFAULT_MAX_CONCURRENT_WORKFLOWS,
            default_retry: None,
            hostname: None,
            version: None,
            labels: HashMap::new(),
            workflows: Vec::new(),
        }
    }

    pub fn namespace(mut self, ns: &str) -> Self {
        self.namespace_id = ns.to_string();
        self
    }

    pub fn max_concurrent(mut self, n: usize) -> Self {
        self.max_concurrent = n;
        self
    }

    /// Simple retry count with default exponential backoff
    pub fn retries(mut self, n: u32) -> Self {
        self.default_retry = Some(Retry::exponential(n));
        self
    }

    /// Full retry configuration
    pub fn retry(mut self, r: Retry) -> Self {
        self.default_retry = Some(r);
        self
    }

    pub fn hostname(mut self, h: &str) -> Self {
        self.hostname = Some(h.to_string());
        self
    }

    pub fn version(mut self, v: &str) -> Self {
        self.version = Some(v.to_string());
        self
    }

    pub fn label(mut self, key: &str, value: &str) -> Self {
        self.labels.insert(key.to_string(), value.to_string());
        self
    }

    pub fn workflows<I, F, Fut, In, Out>(mut self, workflows: I) -> Self
    where
        I: IntoIterator<Item = (&'static str, F)>,
        F: Fn(Context, In) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = anyhow::Result<Out>> + Send + 'static,
        In: DeserializeOwned + Send + 'static,
        Out: Serialize + Send + 'static,
    {
        for (name, handler) in workflows {
            let workflow_name = name.to_string();
            let wrapped =
                move |ctx: Context,
                      input_val: serde_json::Value|
                      -> BoxFuture<'static, anyhow::Result<serde_json::Value>> {
                    let workflow_name = workflow_name.clone();
                    match serde_json::from_value::<In>(input_val) {
                        Ok(input) => {
                            let fut = handler(ctx, input);
                            Box::pin(async move {
                                let output = fut.await?;
                                Ok(serde_json::to_value(output)?)
                            })
                        }
                        Err(e) => Box::pin(async move {
                            Err(anyhow!(
                                "Failed to deserialize workflow '{}' input: {}",
                                workflow_name,
                                e
                            ))
                        }),
                    }
                };
            self.workflows
                .push((name.to_string(), Arc::new(Box::new(wrapped))));
        }
        self
    }

    pub async fn build(self) -> anyhow::Result<Worker> {
        if self.workflows.is_empty() {
            anyhow::bail!("At least one workflow must be registered");
        }

        let client = WorkerServiceClient::connect(self.addr.clone()).await?;

        // Build workflow map and collect types
        let mut workflow_map = HashMap::new();
        let mut workflow_types = Vec::new();
        for (name, handler) in self.workflows {
            workflow_types.push(name.clone());
            workflow_map.insert(name, handler);
        }

        Ok(Worker {
            client,
            namespace_id: self.namespace_id,
            max_concurrent: self.max_concurrent,
            hostname: self.hostname,
            version: self.version,
            labels: self.labels,
            default_retry: self.default_retry,
            workflows: workflow_map,
            workflow_types,
            worker_id: None,
            heartbeat_interval: Duration::from_secs(10),
            semaphore: Arc::new(Semaphore::new(self.max_concurrent)),
            shutdown: CancellationToken::new(),
            consecutive_poll_failures: Arc::new(AtomicU32::new(0)),
        })
    }
}

pub struct Worker {
    pub(crate) client: WorkerServiceClient<Channel>,
    namespace_id: String,
    max_concurrent: usize,
    hostname: Option<String>,
    version: Option<String>,
    labels: HashMap<String, String>,
    default_retry: Option<Retry>,
    workflows: HashMap<String, Arc<WorkflowFn>>,
    workflow_types: Vec<String>,
    worker_id: Option<Uuid>,
    heartbeat_interval: Duration,
    semaphore: Arc<Semaphore>,
    shutdown: CancellationToken,
    consecutive_poll_failures: Arc<AtomicU32>,
}

impl Worker {
    #[allow(clippy::new_ret_no_self)]
    pub fn new(addr: &str) -> WorkerBuilder {
        WorkerBuilder::new(addr)
    }

    pub fn worker_id(&self) -> Option<Uuid> {
        self.worker_id
    }

    pub fn is_registered(&self) -> bool {
        self.worker_id.is_some()
    }

    pub fn active_count(&self) -> usize {
        self.max_concurrent - self.semaphore.available_permits()
    }

    pub fn shutdown(&self) {
        self.shutdown.cancel();
    }

    pub fn shutdown_token(&self) -> CancellationToken {
        self.shutdown.clone()
    }

    pub async fn run(&mut self) -> anyhow::Result<()> {
        if self.workflows.is_empty() {
            anyhow::bail!("No workflows registered. Call workflows() before run()");
        }

        // For each workflow type, use it as its own queue
        let mut all_queues = Vec::new();
        for workflow_type in &self.workflow_types {
            all_queues.push(workflow_type.clone());
        }

        // Use first workflow type as primary queue for registration
        let primary_queue = self
            .workflow_types
            .first()
            .ok_or_else(|| anyhow::anyhow!("No workflows registered"))?;

        let resp = self
            .client
            .register(RegisterRequest {
                namespace_id: self.namespace_id.clone(),
                task_queue: primary_queue.clone(),
                workflow_types: self.workflow_types.clone(),
                hostname: self.hostname.clone().unwrap_or_else(|| {
                    hostname::get()
                        .ok()
                        .and_then(|h| h.into_string().ok())
                        .unwrap_or_default()
                }),
                pid: std::process::id() as i32,
                version: self.version.clone().unwrap_or_default(),
                labels: self.labels.clone(),
                queue_concurrency_limit: None,
                workflow_type_concurrency: Vec::new(),
            })
            .await
            .map_err(map_grpc_error)?
            .into_inner();

        self.worker_id = Some(Uuid::parse_str(&resp.worker_id)?);
        self.heartbeat_interval = Duration::from_secs(resp.heartbeat_interval_secs as u64);

        info!(
            worker_id = %self.worker_id.unwrap(),
            task_queue = %primary_queue,
            workflows = ?self.workflow_types,
            "Worker registered"
        );

        let heartbeat_handle = self.spawn_heartbeat_task();

        let primary_queue_clone = primary_queue.clone();
        let shutdown = self.shutdown.clone();
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => {
                    info!("Worker shutdown signal received");
                    break;
                }
                _ = self.poll_and_execute(primary_queue_clone.clone()) => {}
            }
        }

        info!(active = self.active_count(), "Draining active workflows...");
        while self.active_count() > 0 {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        heartbeat_handle.abort();
        if let Some(id) = self.worker_id {
            let _ = self
                .client
                .deregister(DeregisterRequest {
                    worker_id: id.to_string(),
                    drain: false,
                })
                .await;
        }

        info!("Worker deregistered");
        Ok(())
    }

    fn spawn_heartbeat_task(&self) -> tokio::task::JoinHandle<()> {
        let mut client = self.client.clone();
        let worker_id = self.worker_id.unwrap();
        let interval = self.heartbeat_interval;
        let shutdown = self.shutdown.clone();

        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);

            loop {
                tokio::select! {
                    _ = shutdown.cancelled() => break,
                    _ = ticker.tick() => {
                        let resp = client.heartbeat(HeartbeatRequest {
                            worker_id: worker_id.to_string(),
                        }).await;

                        match resp {
                            Ok(r) => {
                                let inner = r.into_inner();
                                if inner.should_drain {
                                    info!("Server requested drain");
                                    shutdown.cancel();
                                }
                            }
                            Err(e) => {
                                // If NotFound or FailedPrecondition, the server thinks we're offline
                                // (e.g., due to missed heartbeats during network partition).
                                // Trigger shutdown to prevent double execution when server assigns
                                // our tasks to other workers.
                                if e.code() == tonic::Code::NotFound 
                                    || e.code() == tonic::Code::FailedPrecondition 
                                {
                                    error!(
                                        "Worker rejected by server (offline), triggering shutdown: {:?}", 
                                        e
                                    );
                                    shutdown.cancel();
                                } else {
                                    error!("Heartbeat failed: {:?}", e);
                                }
                            }
                        }
                    }
                }
            }
        })
    }

    async fn poll_and_execute(&mut self, task_queue: String) {
        let permit = match self.semaphore.clone().acquire_owned().await {
            Ok(p) => p,
            Err(_) => return,
        };

        // Add client-side timeout slightly longer than server hold time
        // to ensure responsive shutdown during long-polling
        let mut request = Request::new(PollTaskRequest {
            task_queue,
            worker_id: self.worker_id.unwrap().to_string(),
            namespace_id: self.namespace_id.clone(),
            workflow_types: self.workflow_types.clone(),
        });
        request.set_timeout(Duration::from_secs(65));

        let resp = self.client.poll_task(request).await;

        match resp {
            Ok(r) => {
                let task = r.into_inner();
                // Reset failure counter on successful poll
                self.consecutive_poll_failures.store(0, Ordering::Relaxed);
                if task.run_id.is_empty() {
                    drop(permit);
                    return;
                }

                if let Some(handler) = self.workflows.get(&task.workflow_type) {
                    let handler = handler.clone();
                    let client = self.client.clone();
                    let payload = task.input.unwrap_or(ProtoPayload {
                        data: Vec::new(),
                        metadata: HashMap::new(),
                    });
                    let input: serde_json::Value = match serde_json::from_slice(&payload.data) {
                        Ok(v) => v,
                        Err(e) => {
                            warn!(
                                run_id = %task.run_id,
                                error = %e,
                                payload_len = payload.data.len(),
                                "Failed to deserialize task input, using null"
                            );
                            serde_json::Value::Null
                        }
                    };
                    let run_id = task.run_id.clone();
                    let default_retry = self.default_retry.clone();

                    tokio::spawn(async move {
                        let _permit = permit;
                        execute_workflow(client, handler, run_id, input, default_retry).await;
                    });
                } else {
                    error!("No handler for workflow type: {}", task.workflow_type);
                    drop(permit);
                }
            }
            Err(e) => {
                drop(permit);
                if e.code() != tonic::Code::DeadlineExceeded {
                    error!("Poll failed: {:?}", e);
                    // Exponential backoff on poll failures to prevent tight error loops
                    let failures = self
                        .consecutive_poll_failures
                        .fetch_add(1, Ordering::Relaxed)
                        + 1;
                    let backoff_ms = std::cmp::min(100 * 2u64.pow(failures.min(10)), 30_000);
                    tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
                }
            }
        }
    }
}

async fn execute_workflow(
    mut client: WorkerServiceClient<Channel>,
    handler: Arc<WorkflowFn>,
    run_id: String,
    input: serde_json::Value,
    default_retry: Option<Retry>,
) {
    // Create a span for the entire workflow execution
    let span = info_span!(
        "workflow_execution",
        run_id = %run_id,
        otel.kind = "client",
    );

    async {
        let ctx = Context {
            client: client.clone(),
            run_id: run_id.clone(),
            default_retry,
        };

        // Execute workflow in a separate task so panics are captured as JoinError
        // instead of crashing the runtime. JoinError is converted into a failure.
        let task = tokio::spawn(async move { handler(ctx, input).await }.in_current_span());

        let result = match task.await {
            Ok(r) => r,
            Err(join_err) => {
                error!(
                    run_id = %run_id,
                    error = %join_err,
                    "Workflow panicked"
                );
                Err(anyhow::anyhow!(format!("workflow panicked: {join_err}")))
            }
        };

        match result {
            Ok(output) => {
                let data = match serde_json::to_vec(&output) {
                    Ok(bytes) => bytes,
                    Err(e) => {
                        error!(
                            run_id = %run_id,
                            error = %e,
                            "Failed to serialize workflow output"
                        );
                        return; // Let workflow retry or fail explicitly
                    }
                };

                let mut complete_request = Request::new(CompleteWorkflowRequest {
                    run_id,
                    output: Some(ProtoPayload {
                        data,
                        metadata: HashMap::new(),
                    }),
                });
                inject_context(complete_request.metadata_mut());

                let _ = client.complete_workflow(complete_request).await;
            }
            Err(e) => {
                if e.downcast_ref::<WorkflowPaused>().is_some() {
                    info!(
                        run_id = %run_id,
                        "Workflow paused (sleeping)"
                    );
                    return;
                }

                error!(
                    run_id = %run_id,
                    error = %e,
                    "Workflow failed"
                );

                let kagzi_err = e
                    .downcast_ref::<KagziError>()
                    .cloned()
                    .unwrap_or_else(|| KagziError::new(ErrorCode::Internal, e.to_string()));

                let mut fail_request = Request::new(FailWorkflowRequest {
                    run_id,
                    error: Some(kagzi_err.to_detail()),
                });
                inject_context(fail_request.metadata_mut());

                let _ = client.fail_workflow(fail_request).await;
            }
        }
    }
    .instrument(span)
    .await
}
