use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::Duration;

use anyhow::anyhow;
use kagzi_proto::kagzi::worker_service_client::WorkerServiceClient;
use kagzi_proto::kagzi::{
    CompleteWorkflowRequest, DeregisterRequest, ErrorCode, FailWorkflowRequest, HeartbeatRequest,
    Payload as ProtoPayload, PollTaskRequest, RegisterRequest,
    WorkflowTypeConcurrency as ProtoWorkflowTypeConcurrency,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;
use tonic::Request;
use tonic::transport::Channel;
use tracing::{error, info, instrument};
use uuid::Uuid;

use crate::BoxFuture;
use crate::errors::{KagziError, WorkflowPaused, map_grpc_error};
use crate::retry::RetryPolicy;
use crate::tracing_utils::{
    add_tracing_metadata, get_or_generate_correlation_id, get_or_generate_trace_id,
};
use crate::workflow_context::WorkflowContext;

type WorkflowFn = Box<
    dyn Fn(
            WorkflowContext,
            serde_json::Value,
        ) -> BoxFuture<'static, anyhow::Result<serde_json::Value>>
        + Send
        + Sync,
>;

const DEFAULT_MAX_CONCURRENT_WORKFLOWS: usize = 100;

pub struct WorkerBuilder {
    addr: String,
    task_queue: String,
    namespace_id: String,
    max_concurrent: usize,
    hostname: Option<String>,
    version: Option<String>,
    labels: HashMap<String, String>,
    queue_concurrency_limit: Option<i32>,
    workflow_type_concurrency: HashMap<String, i32>,
    default_step_retry: Option<RetryPolicy>,
}

impl WorkerBuilder {
    pub fn new(addr: &str, task_queue: &str) -> Self {
        Self {
            addr: addr.to_string(),
            task_queue: task_queue.to_string(),
            namespace_id: "default".to_string(),
            max_concurrent: DEFAULT_MAX_CONCURRENT_WORKFLOWS,
            hostname: None,
            version: None,
            labels: HashMap::new(),
            queue_concurrency_limit: None,
            workflow_type_concurrency: HashMap::new(),
            default_step_retry: None,
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

    pub fn queue_concurrency_limit(mut self, limit: i32) -> Self {
        if limit > 0 {
            self.queue_concurrency_limit = Some(limit);
        }
        self
    }

    pub fn workflow_type_concurrency(mut self, workflow_type: &str, limit: i32) -> Self {
        if limit > 0 {
            self.workflow_type_concurrency
                .insert(workflow_type.to_string(), limit);
        }
        self
    }

    pub fn default_step_retry(mut self, policy: RetryPolicy) -> Self {
        self.default_step_retry = Some(policy);
        self
    }

    pub async fn build(self) -> anyhow::Result<Worker> {
        let client = WorkerServiceClient::connect(self.addr.clone()).await?;

        Ok(Worker {
            client,
            task_queue: self.task_queue,
            namespace_id: self.namespace_id,
            max_concurrent: self.max_concurrent,
            hostname: self.hostname,
            version: self.version,
            labels: self.labels,
            queue_concurrency_limit: self.queue_concurrency_limit,
            workflow_type_concurrency: self.workflow_type_concurrency,
            default_step_retry: self.default_step_retry,
            workflows: HashMap::new(),
            workflow_types: Vec::new(),
            worker_id: None,
            heartbeat_interval: Duration::from_secs(10),
            semaphore: Arc::new(Semaphore::new(self.max_concurrent)),
            shutdown: CancellationToken::new(),
            completed_counter: Arc::new(AtomicI32::new(0)),
            failed_counter: Arc::new(AtomicI32::new(0)),
        })
    }
}

pub struct Worker {
    pub(crate) client: WorkerServiceClient<Channel>,
    task_queue: String,
    namespace_id: String,
    max_concurrent: usize,
    hostname: Option<String>,
    version: Option<String>,
    labels: HashMap<String, String>,
    queue_concurrency_limit: Option<i32>,
    workflow_type_concurrency: HashMap<String, i32>,
    default_step_retry: Option<RetryPolicy>,
    workflows: HashMap<String, Arc<WorkflowFn>>,
    workflow_types: Vec<String>,
    worker_id: Option<Uuid>,
    heartbeat_interval: Duration,
    semaphore: Arc<Semaphore>,
    shutdown: CancellationToken,
    completed_counter: Arc<AtomicI32>,
    failed_counter: Arc<AtomicI32>,
}

impl Worker {
    pub fn builder(addr: &str, task_queue: &str) -> WorkerBuilder {
        WorkerBuilder::new(addr, task_queue)
    }

    pub async fn new(addr: &str, task_queue: &str) -> anyhow::Result<Self> {
        Self::builder(addr, task_queue).build().await
    }

    pub fn register<F, Fut, I, O>(&mut self, name: &str, func: F)
    where
        F: Fn(WorkflowContext, I) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = anyhow::Result<O>> + Send + 'static,
        I: DeserializeOwned + Send + 'static,
        O: Serialize + Send + 'static,
    {
        let workflow_name = name.to_string();
        let wrapped = move |ctx: WorkflowContext,
                            input_val: serde_json::Value|
              -> BoxFuture<'static, anyhow::Result<serde_json::Value>> {
            let workflow_name = workflow_name.clone();
            match serde_json::from_value::<I>(input_val) {
                Ok(input) => {
                    let fut = func(ctx, input);
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
            .insert(name.to_string(), Arc::new(Box::new(wrapped)));
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

    #[instrument(skip(self), fields(
        correlation_id = %get_or_generate_correlation_id(),
        trace_id = %get_or_generate_trace_id(),
        task_queue = %self.task_queue,
        max_concurrent = %self.max_concurrent
    ))]
    pub async fn run(&mut self) -> anyhow::Result<()> {
        if self.workflows.is_empty() {
            anyhow::bail!("No workflows registered. Call register() before run()");
        }

        let workflow_types: Vec<String> = self.workflows.keys().cloned().collect();
        self.workflow_types = workflow_types.clone();

        let resp = self
            .client
            .register(RegisterRequest {
                namespace_id: self.namespace_id.clone(),
                task_queue: self.task_queue.clone(),
                workflow_types,
                hostname: self.hostname.clone().unwrap_or_else(|| {
                    hostname::get()
                        .ok()
                        .and_then(|h| h.into_string().ok())
                        .unwrap_or_default()
                }),
                pid: std::process::id() as i32,
                version: self.version.clone().unwrap_or_default(),
                max_concurrent: self.max_concurrent as i32,
                labels: self.labels.clone(),
                queue_concurrency_limit: self.queue_concurrency_limit,
                workflow_type_concurrency: self
                    .workflow_type_concurrency
                    .iter()
                    .map(|(workflow_type, max)| ProtoWorkflowTypeConcurrency {
                        workflow_type: workflow_type.clone(),
                        max_concurrent: *max,
                    })
                    .collect(),
            })
            .await
            .map_err(map_grpc_error)?
            .into_inner();

        self.worker_id = Some(Uuid::parse_str(&resp.worker_id)?);
        self.heartbeat_interval = Duration::from_secs(resp.heartbeat_interval_secs as u64);

        info!(
            worker_id = %self.worker_id.unwrap(),
            task_queue = %self.task_queue,
            workflows = ?self.workflows.keys().collect::<Vec<_>>(),
            "Worker registered"
        );

        let heartbeat_handle = self.spawn_heartbeat_task();

        let shutdown = self.shutdown.clone();
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => {
                    info!("Worker shutdown signal received");
                    break;
                }
                _ = self.poll_and_execute() => {}
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
        let semaphore = self.semaphore.clone();
        let max = self.max_concurrent;
        let interval = self.heartbeat_interval;
        let shutdown = self.shutdown.clone();
        let completed_counter = self.completed_counter.clone();
        let failed_counter = self.failed_counter.clone();

        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);

            loop {
                tokio::select! {
                    _ = shutdown.cancelled() => break,
                    _ = ticker.tick() => {
                        let active = (max - semaphore.available_permits()) as i32;
                        let completed = completed_counter.swap(0, Ordering::Relaxed);
                        let failed = failed_counter.swap(0, Ordering::Relaxed);

                        let resp = client.heartbeat(HeartbeatRequest {
                            worker_id: worker_id.to_string(),
                            active_count: active,
                            completed_delta: completed,
                            failed_delta: failed,
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
                                error!("Heartbeat failed: {:?}", e);
                            }
                        }
                    }
                }
            }
        })
    }

    async fn poll_and_execute(&mut self) {
        let permit = match self.semaphore.clone().acquire_owned().await {
            Ok(p) => p,
            Err(_) => return,
        };

        let resp = self
            .client
            .poll_task(PollTaskRequest {
                task_queue: self.task_queue.clone(),
                worker_id: self.worker_id.unwrap().to_string(),
                namespace_id: self.namespace_id.clone(),
                workflow_types: self.workflow_types.clone(),
            })
            .await;

        match resp {
            Ok(r) => {
                let task = r.into_inner();
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
                            tracing::warn!(
                                run_id = %task.run_id,
                                error = %e,
                                payload_len = payload.data.len(),
                                "Failed to deserialize task input, using null"
                            );
                            serde_json::Value::Null
                        }
                    };
                    let run_id = task.run_id.clone();
                    let default_step_retry = self.default_step_retry.clone();
                    let completed_counter = self.completed_counter.clone();
                    let failed_counter = self.failed_counter.clone();

                    tokio::spawn(async move {
                        let _permit = permit;
                        let correlation_id = uuid::Uuid::now_v7().to_string();
                        let trace_id = uuid::Uuid::now_v7().to_string();

                        crate::tracing_utils::with_tracing_context(
                            Some(correlation_id.clone()),
                            Some(trace_id.clone()),
                            execute_workflow(
                                client,
                                handler,
                                run_id,
                                input,
                                correlation_id,
                                trace_id,
                                default_step_retry,
                                completed_counter,
                                failed_counter,
                            ),
                        )
                        .await;
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
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn execute_workflow(
    mut client: WorkerServiceClient<Channel>,
    handler: Arc<WorkflowFn>,
    run_id: String,
    input: serde_json::Value,
    correlation_id: String,
    trace_id: String,
    default_step_retry: Option<RetryPolicy>,
    completed_counter: Arc<AtomicI32>,
    failed_counter: Arc<AtomicI32>,
) {
    let ctx = WorkflowContext {
        client: client.clone(),
        run_id: run_id.clone(),
        sleep_counter: 0,
        default_step_retry,
    };

    let result = handler(ctx, input).await;

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

            let complete_request = add_tracing_metadata(Request::new(CompleteWorkflowRequest {
                run_id,
                output: Some(ProtoPayload {
                    data,
                    metadata: HashMap::new(),
                }),
            }));

            let _ = client.complete_workflow(complete_request).await;
            completed_counter.fetch_add(1, Ordering::Relaxed);
        }
        Err(e) => {
            if e.downcast_ref::<WorkflowPaused>().is_some() {
                info!(
                    correlation_id = correlation_id,
                    trace_id = trace_id,
                    run_id = %run_id,
                    "Workflow paused (sleeping)"
                );
                return;
            }

            error!(
                correlation_id = correlation_id,
                trace_id = trace_id,
                run_id = %run_id,
                error = %e,
                "Workflow failed"
            );

            let kagzi_err = e
                .downcast_ref::<KagziError>()
                .cloned()
                .unwrap_or_else(|| KagziError::new(ErrorCode::Internal, e.to_string()));

            let fail_request = add_tracing_metadata(Request::new(FailWorkflowRequest {
                run_id,
                error: Some(kagzi_err.to_detail()),
            }));

            let _ = client.fail_workflow(fail_request).await;
            failed_counter.fetch_add(1, Ordering::Relaxed);
        }
    }
}
