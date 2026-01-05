//! Worker implementation for executing Kagzi workflows.
//!
//! # Architecture Overview
//!
//! The worker polls the Kagzi server for workflow tasks, executes them using
//! registered handler functions, and reports results back to the server.
//!
//! # Type Erasure with BoxFuture
//!
//! Workflow handlers have different input/output types, but we need to store
//! them uniformly in a `HashMap`. We use `BoxFuture` for type erasure:
//!
//! ```rust
//! type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
//! ```
//!
//! This allows us to:
//! 1. Store handlers with different signatures in the same map
//! 2. Pass them across async boundaries
//! 3. Execute them dynamically based on workflow type
//!
//! The `'static` lifetime is required because handlers are stored in the worker
//! and can be called at any time in the future.
//!
//! # Why Arc?
//!
//! - `Arc<Semaphore>`: Shared across concurrent task executions to limit parallelism
//! - `Arc<WorkflowFn>`: Handlers are cloned into each spawned task
//! - `Arc<AtomicU32>`: Poll failure counter shared between main loop and tasks

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use backoff::ExponentialBackoff;
use backoff::backoff::Backoff;
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
use tower::ServiceBuilder;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::BoxFuture;
use crate::context::Context;
use crate::errors::{KagziError, WorkflowPaused};
use crate::propagation::inject_context;
use crate::retry::Retry;

/// Default maximum number of concurrent workflow executions
const DEFAULT_MAX_CONCURRENT_WORKFLOWS: usize = 100;

/// Default heartbeat interval in seconds
const DEFAULT_HEARTBEAT_INTERVAL_SECS: u64 = 10;

/// Long poll timeout for task polling (slightly longer than server hold time)
const POLL_TIMEOUT_SECS: u64 = 65;

/// Workflow handler function type.
///
/// Wraps user-provided workflow functions with type erasure so they can be
/// stored and executed dynamically. The Arc allows sharing across concurrent
/// task executions.
///
/// # Type Parameters
/// - Input is deserialized from `serde_json::Value`
/// - Output is serialized back to `serde_json::Value`
type WorkflowFn = Box<
    dyn Fn(Context, serde_json::Value) -> BoxFuture<'static, anyhow::Result<serde_json::Value>>
        + Send
        + Sync,
>;

/// Builder for configuring and constructing a Worker
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
    pub fn new(addr: impl Into<String>) -> Self {
        Self {
            addr: addr.into(),
            namespace_id: "default".to_string(),
            max_concurrent: DEFAULT_MAX_CONCURRENT_WORKFLOWS,
            default_retry: None,
            hostname: None,
            version: None,
            labels: HashMap::new(),
            workflows: Vec::new(),
        }
    }

    pub fn namespace(mut self, ns: impl Into<String>) -> Self {
        self.namespace_id = ns.into();
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

    pub fn hostname(mut self, h: impl Into<String>) -> Self {
        self.hostname = Some(h.into());
        self
    }

    pub fn version(mut self, v: impl Into<String>) -> Self {
        self.version = Some(v.into());
        self
    }

    pub fn label(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.labels.insert(key.into(), value.into());
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
                            Err(anyhow::anyhow!(
                                "Failed to deserialize workflow '{}' input: {e}",
                                workflow_name,
                            ))
                        }),
                    }
                };
            self.workflows
                .push((name.to_string(), Arc::new(Box::new(wrapped))));
        }
        self
    }

    /// Validate the worker configuration before building
    ///
    /// # Errors
    /// Returns `anyhow::Error` if the configuration is invalid
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.max_concurrent == 0 {
            anyhow::bail!("max_concurrent must be greater than 0");
        }

        if let Some(ref retry) = self.default_retry {
            retry
                .validate()
                .map_err(|e| anyhow::anyhow!("Invalid retry configuration: {e}"))?;
        }

        for (name, _) in &self.workflows {
            if name.is_empty() {
                anyhow::bail!("Workflow names cannot be empty");
            }
        }

        Ok(())
    }

    /// Build the worker and connect to the server
    ///
    /// # Errors
    /// Returns an error if:
    /// - No workflows are registered
    /// - Configuration is invalid
    /// - Connection to the server fails
    #[tracing::instrument(skip(self))]
    pub async fn build(self) -> anyhow::Result<Worker> {
        // Validate configuration first
        self.validate()?;

        if self.workflows.is_empty() {
            anyhow::bail!("At least one workflow must be registered");
        }

        // Create channel with tower middleware for timeout
        let channel = Channel::from_shared(self.addr.clone())
            .map_err(|e| anyhow::anyhow!("Invalid server address: {e}"))?
            .connect()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to connect to worker service: {e}"))?;

        // Wrap channel with tower timeout layer
        let channel = ServiceBuilder::new()
            .timeout(Duration::from_secs(POLL_TIMEOUT_SECS))
            .service(channel);

        let client = WorkerServiceClient::new(channel);

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
            heartbeat_interval: Duration::from_secs(DEFAULT_HEARTBEAT_INTERVAL_SECS),
            semaphore: Arc::new(Semaphore::new(self.max_concurrent)),
            shutdown: CancellationToken::new(),
            consecutive_poll_failures: Arc::new(AtomicU32::new(0)),
        })
    }
}

pub struct Worker {
    pub(crate) client: WorkerServiceClient<tower::timeout::Timeout<Channel>>,
    namespace_id: String,
    max_concurrent: usize,
    hostname: Option<String>,
    version: Option<String>,
    labels: HashMap<String, String>,
    default_retry: Option<Retry>,
    /// Workflow handlers by type name.
    /// Arc is used to clone handlers into each spawned task for concurrent execution.
    workflows: HashMap<String, Arc<WorkflowFn>>,
    workflow_types: Vec<String>,
    worker_id: Option<Uuid>,
    heartbeat_interval: Duration,
    /// Semaphore limits concurrent workflow executions.
    /// Arc allows cloning permits into spawned tasks.
    semaphore: Arc<Semaphore>,
    shutdown: CancellationToken,
    /// Counter for exponential backoff on poll failures.
    /// Arc allows atomic updates from both main loop and spawned tasks.
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

    /// Run the worker main loop
    ///
    /// This method will:
    /// 1. Register the worker with the server
    /// 2. Start a heartbeat task
    /// 3. Poll for tasks and execute them
    /// 4. Handle graceful shutdown
    #[tracing::instrument(skip(self))]
    pub async fn run(&mut self) -> anyhow::Result<()> {
        if self.workflows.is_empty() {
            anyhow::bail!("No workflows registered. Call workflows() before run()");
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
            .map_err(|e| anyhow::anyhow!(KagziError::from(e)))?
            .into_inner();

        self.worker_id = Some(Uuid::parse_str(&resp.worker_id)?);
        self.heartbeat_interval = Duration::from_secs(resp.heartbeat_interval_secs as u64);

        info!(
            worker_id = %self.worker_id.map(|id| id.to_string()).unwrap_or_else(|| "unregistered".to_string()),
            task_queue = %primary_queue,
            workflow_count = %self.workflow_types.len(),
            "Worker registered"
        );

        let heartbeat_handle = match self.spawn_heartbeat_task() {
            Some(handle) => handle,
            None => {
                warn!("Cannot spawn heartbeat task: worker not registered");
                return Ok(());
            }
        };

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

        info!(
            active_count = self.active_count(),
            "Draining active workflows..."
        );
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

    /// Spawn a background task to send periodic heartbeats to the server
    fn spawn_heartbeat_task(&self) -> Option<tokio::task::JoinHandle<()>> {
        let worker_id = self.worker_id?;
        let mut client = self.client.clone();
        let interval = self.heartbeat_interval;
        let shutdown = self.shutdown.clone();

        Some(tokio::spawn(async move {
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
                                        error = %e,
                                        "Worker rejected by server (offline), triggering shutdown"
                                    );
                                    shutdown.cancel();
                                } else {
                                    error!(error = %e, "Heartbeat failed");
                                }
                            }
                        }
                    }
                }
            }
        }))
    }

    /// Poll for a task from the server and execute it
    #[tracing::instrument(skip(self))]
    async fn poll_and_execute(&mut self, task_queue: String) {
        let permit = match self.semaphore.clone().acquire_owned().await {
            Ok(p) => p,
            Err(_) => return,
        };

        // Get worker_id, return early if not registered
        let worker_id = match &self.worker_id {
            Some(id) => id,
            None => {
                warn!("Worker not registered, skipping poll");
                tokio::time::sleep(Duration::from_secs(1)).await;
                return;
            }
        };

        // Timeout is handled by tower middleware at the channel level
        let request = Request::new(PollTaskRequest {
            task_queue,
            worker_id: worker_id.to_string(),
            namespace_id: self.namespace_id.clone(),
            workflow_types: self.workflow_types.clone(),
        });

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
                    error!(workflow_type = %task.workflow_type, "No handler for workflow type");
                    drop(permit);
                }
            }
            Err(e) => {
                drop(permit);
                if e.code() != tonic::Code::DeadlineExceeded {
                    error!(error = %e, "Poll failed");
                    // Exponential backoff on poll failures to prevent tight error loops
                    let _failures = self
                        .consecutive_poll_failures
                        .fetch_add(1, Ordering::Relaxed)
                        + 1;

                    // Use backoff crate for exponential backoff
                    let mut backoff = ExponentialBackoff {
                        initial_interval: Duration::from_millis(100),
                        max_interval: Duration::from_secs(30),
                        multiplier: 2.0,
                        max_elapsed_time: None, // No limit
                        ..Default::default()
                    };

                    // Calculate backoff duration based on failure count
                    let backoff_duration =
                        backoff.next_backoff().unwrap_or(Duration::from_secs(30));
                    tokio::time::sleep(backoff_duration).await;
                }
            }
        }
    }
}

/// Execute a workflow task and report the result to the server
#[tracing::instrument(
    name = "workflow_execution",
    skip(client, handler),
    fields(run_id = %run_id, otel.kind = "client")
)]
async fn execute_workflow(
    mut client: WorkerServiceClient<tower::timeout::Timeout<Channel>>,
    handler: Arc<WorkflowFn>,
    run_id: String,
    input: serde_json::Value,
    default_retry: Option<Retry>,
) {
    let ctx = Context {
        client: client.clone(),
        run_id: run_id.clone(),
        default_retry,
    };

    // Execute workflow in a separate task so panics are captured as JoinError
    // instead of crashing the runtime. JoinError is converted into a failure.
    let task = tokio::spawn(async move { handler(ctx, input).await });

    let result = match task.await {
        Ok(r) => r,
        Err(join_err) => {
            error!(error = %join_err, "Workflow panicked");
            Err(anyhow::anyhow!("workflow panicked: {join_err}"))
        }
    };

    match result {
        Ok(output) => {
            let data = match serde_json::to_vec(&output) {
                Ok(bytes) => bytes,
                Err(e) => {
                    error!(error = %e, "Failed to serialize workflow output");
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
                info!("Workflow paused (sleeping)");
                return;
            }

            error!(error = %e, "Workflow failed");

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
