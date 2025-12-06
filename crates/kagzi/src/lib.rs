use std::collections::HashMap;
use std::future::{Future, IntoFuture};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use kagzi_proto::kagzi::workflow_service_client::WorkflowServiceClient;
use kagzi_proto::kagzi::{
    BeginStepRequest, CompleteStepRequest, CompleteWorkflowRequest, CreateScheduleRequest,
    DeleteScheduleRequest, DeregisterWorkerRequest, ErrorCode, ErrorDetail, FailStepRequest,
    FailWorkflowRequest, GetScheduleRequest, ListSchedulesRequest, PollActivityRequest,
    RegisterWorkerRequest, Schedule as ProtoSchedule, ScheduleSleepRequest, StartWorkflowRequest,
    UpdateScheduleRequest, WorkerHeartbeatRequest,
    WorkflowTypeConcurrency as ProtoWorkflowTypeConcurrency,
};
use prost::Message;
use prost_types::Timestamp;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;
use tonic::transport::Channel;
use tonic::{Code, Request, Status};
use tracing::{error, info, instrument};
use tracing_utils::{
    add_tracing_metadata, get_or_generate_correlation_id, get_or_generate_trace_id,
};
use uuid::Uuid;

pub mod tracing_utils;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Error indicating the workflow should pause (e.g., for sleep)
#[derive(Debug)]
pub struct WorkflowPaused;

impl std::fmt::Display for WorkflowPaused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Workflow paused")
    }
}

impl std::error::Error for WorkflowPaused {}

#[derive(Debug, Clone)]
pub struct KagziError {
    pub code: ErrorCode,
    pub message: String,
    pub non_retryable: bool,
    pub retry_after: Option<Duration>,
}

impl KagziError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            non_retryable: matches!(
                code,
                ErrorCode::InvalidArgument
                    | ErrorCode::InvalidState
                    | ErrorCode::PreconditionFailed
                    | ErrorCode::Conflict
                    | ErrorCode::AlreadyCompleted
                    | ErrorCode::Unauthorized
            ),
            retry_after: None,
        }
    }

    pub fn non_retryable(message: impl Into<String>) -> Self {
        Self {
            code: ErrorCode::PreconditionFailed,
            message: message.into(),
            non_retryable: true,
            retry_after: None,
        }
    }

    pub fn retry_after(message: impl Into<String>, retry_after: Duration) -> Self {
        Self {
            code: ErrorCode::Unavailable,
            message: message.into(),
            non_retryable: false,
            retry_after: Some(retry_after),
        }
    }

    pub fn to_detail(&self) -> ErrorDetail {
        ErrorDetail {
            code: self.code as i32,
            message: self.message.clone(),
            non_retryable: self.non_retryable,
            retry_after_ms: self.retry_after.map(|d| d.as_millis() as i64).unwrap_or(0),
            subject: String::new(),
            subject_id: String::new(),
        }
    }
}

impl std::fmt::Display for KagziError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({:?})", self.message, self.code)
    }
}

impl std::error::Error for KagziError {}

fn error_code_from_status(code: Code) -> ErrorCode {
    match code {
        Code::NotFound => ErrorCode::NotFound,
        Code::InvalidArgument => ErrorCode::InvalidArgument,
        Code::FailedPrecondition => ErrorCode::PreconditionFailed,
        Code::Aborted => ErrorCode::Conflict,
        Code::PermissionDenied => ErrorCode::Unauthorized,
        Code::Unavailable => ErrorCode::Unavailable,
        Code::DeadlineExceeded | Code::Cancelled | Code::ResourceExhausted => {
            ErrorCode::Unavailable
        }
        _ => ErrorCode::Internal,
    }
}

impl From<Status> for KagziError {
    fn from(status: Status) -> Self {
        if let Ok(detail) = ErrorDetail::decode(status.details()) {
            return Self {
                code: ErrorCode::try_from(detail.code).unwrap_or(ErrorCode::Internal),
                message: if detail.message.is_empty() {
                    status.message().to_string()
                } else {
                    detail.message
                },
                non_retryable: detail.non_retryable,
                retry_after: if detail.retry_after_ms > 0 {
                    Some(Duration::from_millis(detail.retry_after_ms as u64))
                } else {
                    None
                },
            };
        }

        Self {
            code: error_code_from_status(status.code()),
            message: status.message().to_string(),
            non_retryable: matches!(
                status.code(),
                Code::InvalidArgument | Code::FailedPrecondition | Code::PermissionDenied
            ),
            retry_after: None,
        }
    }
}

fn map_grpc_error(status: Status) -> anyhow::Error {
    anyhow::Error::new(KagziError::from(status))
}

#[derive(Clone, Default)]
pub struct RetryPolicy {
    pub maximum_attempts: Option<i32>,
    pub initial_interval: Option<Duration>,
    pub backoff_coefficient: Option<f64>,
    pub maximum_interval: Option<Duration>,
    pub non_retryable_errors: Vec<String>,
}

impl From<RetryPolicy> for kagzi_proto::kagzi::RetryPolicy {
    fn from(p: RetryPolicy) -> Self {
        Self {
            maximum_attempts: p.maximum_attempts.unwrap_or(0),
            initial_interval_ms: p
                .initial_interval
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0),
            backoff_coefficient: p.backoff_coefficient.unwrap_or(0.0),
            maximum_interval_ms: p
                .maximum_interval
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0),
            non_retryable_errors: p.non_retryable_errors,
        }
    }
}

pub struct WorkflowContext {
    client: WorkflowServiceClient<Channel>,
    run_id: String,
    sleep_counter: u32,
    default_step_retry: Option<RetryPolicy>,
}

impl WorkflowContext {
    #[instrument(skip(self, fut), fields(
        correlation_id = %get_or_generate_correlation_id(),
        trace_id = %get_or_generate_trace_id(),
        run_id = %self.run_id,
        step_id = %step_id
    ))]
    pub async fn run<R, Fut>(&mut self, step_id: &str, fut: Fut) -> anyhow::Result<R>
    where
        R: Serialize + DeserializeOwned + Send + 'static,
        Fut: Future<Output = anyhow::Result<R>> + Send,
    {
        let begin_request = add_tracing_metadata(Request::new(BeginStepRequest {
            run_id: self.run_id.clone(),
            step_id: step_id.to_string(),
            input: vec![],
            retry_policy: self.default_step_retry.clone().map(Into::into),
        }));

        let begin_resp = self
            .client
            .begin_step(begin_request)
            .await
            .map_err(map_grpc_error)?
            .into_inner();

        if !begin_resp.should_execute {
            let result: R = serde_json::from_slice(&begin_resp.cached_result)?;
            return Ok(result);
        }

        let result = fut.await;

        match result {
            Ok(val) => {
                let output_bytes = serde_json::to_vec(&val)?;
                let complete_request = add_tracing_metadata(Request::new(CompleteStepRequest {
                    run_id: self.run_id.clone(),
                    step_id: step_id.to_string(),
                    output: output_bytes,
                }));

                self.client
                    .complete_step(complete_request)
                    .await
                    .map_err(map_grpc_error)?;
                Ok(val)
            }
            Err(e) => {
                error!(error = %e, "Step {} failed", step_id);

                let kagzi_err = e
                    .downcast_ref::<KagziError>()
                    .cloned()
                    .unwrap_or_else(|| KagziError::new(ErrorCode::Internal, e.to_string()));

                let fail_request = add_tracing_metadata(Request::new(FailStepRequest {
                    run_id: self.run_id.clone(),
                    step_id: step_id.to_string(),
                    error: Some(kagzi_err.to_detail()),
                }));

                self.client
                    .fail_step(fail_request)
                    .await
                    .map_err(map_grpc_error)?;
                Err(anyhow::Error::new(kagzi_err))
            }
        }
    }

    #[instrument(skip(self, input, fut), fields(
        correlation_id = %get_or_generate_correlation_id(),
        trace_id = %get_or_generate_trace_id(),
        run_id = %self.run_id,
        step_id = %step_id
    ))]
    pub async fn run_with_input<I, R, Fut>(
        &mut self,
        step_id: &str,
        input: &I,
        fut: Fut,
    ) -> anyhow::Result<R>
    where
        I: Serialize + Send + 'static,
        R: Serialize + DeserializeOwned + Send + 'static,
        Fut: Future<Output = anyhow::Result<R>> + Send,
    {
        self.run_with_input_with_retry(step_id, input, None, fut)
            .await
    }

    pub async fn run_with_input_with_retry<I, R, Fut>(
        &mut self,
        step_id: &str,
        input: &I,
        retry_policy: Option<RetryPolicy>,
        fut: Fut,
    ) -> anyhow::Result<R>
    where
        I: Serialize + Send + 'static,
        R: Serialize + DeserializeOwned + Send + 'static,
        Fut: Future<Output = anyhow::Result<R>> + Send,
    {
        let effective_retry = retry_policy.or_else(|| self.default_step_retry.clone());

        let input_bytes = serde_json::to_vec(input)?;
        let begin_request = add_tracing_metadata(Request::new(BeginStepRequest {
            run_id: self.run_id.clone(),
            step_id: step_id.to_string(),
            input: input_bytes,
            retry_policy: effective_retry.clone().map(Into::into),
        }));

        let begin_resp = self
            .client
            .begin_step(begin_request)
            .await
            .map_err(map_grpc_error)?
            .into_inner();

        if !begin_resp.should_execute {
            let result: R = serde_json::from_slice(&begin_resp.cached_result)?;
            return Ok(result);
        }

        let result = fut.await;

        match result {
            Ok(val) => {
                let output_bytes = serde_json::to_vec(&val)?;
                let complete_request = add_tracing_metadata(Request::new(CompleteStepRequest {
                    run_id: self.run_id.clone(),
                    step_id: step_id.to_string(),
                    output: output_bytes,
                }));

                self.client
                    .complete_step(complete_request)
                    .await
                    .map_err(map_grpc_error)?;
                Ok(val)
            }
            Err(e) => {
                error!(error = %e, "Step {} failed", step_id);

                let kagzi_err = e
                    .downcast_ref::<KagziError>()
                    .cloned()
                    .unwrap_or_else(|| KagziError::new(ErrorCode::Internal, e.to_string()));

                let fail_request = add_tracing_metadata(Request::new(FailStepRequest {
                    run_id: self.run_id.clone(),
                    step_id: step_id.to_string(),
                    error: Some(kagzi_err.to_detail()),
                }));

                self.client
                    .fail_step(fail_request)
                    .await
                    .map_err(map_grpc_error)?;
                Err(anyhow::Error::new(kagzi_err))
            }
        }
    }

    #[instrument(skip(self), fields(
        correlation_id = %get_or_generate_correlation_id(),
        trace_id = %get_or_generate_trace_id(),
        run_id = %self.run_id,
        duration_seconds = duration.as_secs()
    ))]
    pub async fn sleep(&mut self, duration: Duration) -> anyhow::Result<()> {
        let step_id = format!("__sleep_{}", self.sleep_counter);
        self.sleep_counter += 1;

        let begin_request = add_tracing_metadata(Request::new(BeginStepRequest {
            run_id: self.run_id.clone(),
            step_id: step_id.clone(),
            input: vec![],
            retry_policy: None,
        }));

        let begin_resp = self
            .client
            .begin_step(begin_request)
            .await
            .map_err(map_grpc_error)?
            .into_inner();

        if !begin_resp.should_execute {
            return Ok(());
        }

        let sleep_request = add_tracing_metadata(Request::new(ScheduleSleepRequest {
            run_id: self.run_id.clone(),
            duration_seconds: duration.as_secs(),
        }));

        self.client
            .schedule_sleep(sleep_request)
            .await
            .map_err(map_grpc_error)?;

        let complete_request = add_tracing_metadata(Request::new(CompleteStepRequest {
            run_id: self.run_id.clone(),
            step_id,
            output: serde_json::to_vec(&())?,
        }));
        self.client
            .complete_step(complete_request)
            .await
            .map_err(map_grpc_error)?;

        Err(WorkflowPaused.into())
    }
}

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
        let client = WorkflowServiceClient::connect(self.addr.clone()).await?;

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
        })
    }
}

pub struct Worker {
    client: WorkflowServiceClient<Channel>,
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
        let wrapped = move |ctx: WorkflowContext,
                            input_val: serde_json::Value|
              -> BoxFuture<'static, anyhow::Result<serde_json::Value>> {
            let input: I = serde_json::from_value(input_val).unwrap();
            let fut = func(ctx, input);
            Box::pin(async move {
                let output = fut.await?;
                Ok(serde_json::to_value(output)?)
            })
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
            .register_worker(RegisterWorkerRequest {
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
                queue_concurrency_limit: self.queue_concurrency_limit.unwrap_or(0),
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
                .deregister_worker(DeregisterWorkerRequest {
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

        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            let mut completed: i32 = 0;
            let mut failed: i32 = 0;

            loop {
                tokio::select! {
                    _ = shutdown.cancelled() => break,
                    _ = ticker.tick() => {
                        let active = (max - semaphore.available_permits()) as i32;

                        let resp = client.worker_heartbeat(WorkerHeartbeatRequest {
                            worker_id: worker_id.to_string(),
                            active_count: active,
                            completed_delta: completed,
                            failed_delta: failed,
                        }).await;

                        completed = 0;
                        failed = 0;

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
            .poll_activity(PollActivityRequest {
                task_queue: self.task_queue.clone(),
                worker_id: self.worker_id.unwrap().to_string(),
                namespace_id: self.namespace_id.clone(),
                supported_workflow_types: self.workflow_types.clone(),
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
                    let input: serde_json::Value = serde_json::from_slice(&task.workflow_input)
                        .unwrap_or(serde_json::Value::Null);
                    let run_id = task.run_id.clone();
                    let default_step_retry = self.default_step_retry.clone();

                    tokio::spawn(async move {
                        let _permit = permit;
                        let correlation_id = uuid::Uuid::new_v4().to_string();
                        let trace_id = uuid::Uuid::new_v4().to_string();

                        tracing_utils::with_tracing_context(
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

/// Execute a workflow with proper tracing context
async fn execute_workflow(
    mut client: WorkflowServiceClient<Channel>,
    handler: Arc<WorkflowFn>,
    run_id: String,
    input: serde_json::Value,
    correlation_id: String,
    trace_id: String,
    default_step_retry: Option<RetryPolicy>,
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
            let complete_request = add_tracing_metadata(Request::new(CompleteWorkflowRequest {
                run_id,
                output: serde_json::to_vec(&output).unwrap(),
            }));

            let _ = client.complete_workflow(complete_request).await;
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
        }
    }
}

pub struct Client {
    client: WorkflowServiceClient<Channel>,
}

impl Client {
    pub async fn connect(addr: &str) -> anyhow::Result<Self> {
        let client = WorkflowServiceClient::connect(addr.to_string()).await?;
        Ok(Self { client })
    }

    pub fn workflow<I: Serialize>(
        &mut self,
        workflow_type: &str,
        task_queue: &str,
        input: I,
    ) -> WorkflowBuilder<'_, I> {
        WorkflowBuilder::new(self, workflow_type, task_queue, input)
    }

    pub fn schedule(
        &mut self,
        workflow_type: &str,
        task_queue: &str,
        cron_expr: &str,
    ) -> ScheduleBuilder<'_> {
        ScheduleBuilder::new(self, workflow_type, task_queue, cron_expr)
    }

    pub async fn get_schedule(
        &mut self,
        schedule_id: &str,
        namespace_id: &str,
    ) -> anyhow::Result<ProtoSchedule> {
        let resp = self
            .client
            .get_schedule(GetScheduleRequest {
                schedule_id: schedule_id.to_string(),
                namespace_id: namespace_id.to_string(),
            })
            .await
            .map_err(map_grpc_error)?
            .into_inner();

        resp.schedule
            .ok_or_else(|| anyhow::anyhow!("Missing schedule in response"))
    }

    pub async fn list_schedules(
        &mut self,
        namespace_id: &str,
        task_queue: Option<&str>,
        limit: Option<i32>,
    ) -> anyhow::Result<Vec<ProtoSchedule>> {
        let resp = self
            .client
            .list_schedules(ListSchedulesRequest {
                namespace_id: namespace_id.to_string(),
                task_queue: task_queue.unwrap_or_default().to_string(),
                limit: limit.unwrap_or(0),
            })
            .await
            .map_err(map_grpc_error)?
            .into_inner();

        Ok(resp.schedules)
    }

    pub async fn update_schedule(
        &mut self,
        schedule_id: &str,
        namespace_id: &str,
        update: ScheduleUpdate,
    ) -> anyhow::Result<ProtoSchedule> {
        let req = UpdateScheduleRequest {
            schedule_id: schedule_id.to_string(),
            namespace_id: namespace_id.to_string(),
            task_queue: update.task_queue,
            workflow_type: update.workflow_type,
            cron_expr: update.cron_expr,
            input: update.input.map(|v| serde_json::to_vec(&v)).transpose()?,
            context: update.context.map(|v| serde_json::to_vec(&v)).transpose()?,
            enabled: update.enabled,
            max_catchup: update.max_catchup,
            next_fire_at: update.next_fire_at.map(|dt| Timestamp {
                seconds: dt.timestamp(),
                nanos: dt.timestamp_subsec_nanos() as i32,
            }),
            version: update.version,
        };

        let resp = self
            .client
            .update_schedule(req)
            .await
            .map_err(map_grpc_error)?
            .into_inner();

        resp.schedule
            .ok_or_else(|| anyhow::anyhow!("Missing schedule in response"))
    }

    pub async fn delete_schedule(
        &mut self,
        schedule_id: &str,
        namespace_id: &str,
    ) -> anyhow::Result<()> {
        self.client
            .delete_schedule(DeleteScheduleRequest {
                schedule_id: schedule_id.to_string(),
                namespace_id: namespace_id.to_string(),
            })
            .await
            .map_err(map_grpc_error)?;

        Ok(())
    }
}

pub struct WorkflowBuilder<'a, I> {
    client: &'a mut Client,
    workflow_type: String,
    task_queue: String,
    input: I,
    workflow_id: Option<String>,
    idempotency_key: Option<String>,
    context: Option<serde_json::Value>,
    deadline_at: Option<chrono::DateTime<chrono::Utc>>,
    version: Option<String>,
    retry_policy: Option<RetryPolicy>,
}

impl<'a, I: Serialize> WorkflowBuilder<'a, I> {
    fn new(client: &'a mut Client, workflow_type: &str, task_queue: &str, input: I) -> Self {
        Self {
            client,
            workflow_type: workflow_type.to_string(),
            task_queue: task_queue.to_string(),
            input,
            workflow_id: None,
            idempotency_key: None,
            context: None,
            deadline_at: None,
            version: None,
            retry_policy: None,
        }
    }

    pub fn id(mut self, workflow_id: impl Into<String>) -> Self {
        self.workflow_id = Some(workflow_id.into());
        self
    }

    pub fn idempotent(mut self, key: impl Into<String>) -> Self {
        self.idempotency_key = Some(key.into());
        self
    }

    pub fn context(mut self, ctx: serde_json::Value) -> Self {
        self.context = Some(ctx);
        self
    }

    pub fn deadline(mut self, deadline: chrono::DateTime<chrono::Utc>) -> Self {
        self.deadline_at = Some(deadline);
        self
    }

    pub fn version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }

    pub fn retry_policy(mut self, policy: RetryPolicy) -> Self {
        self.retry_policy = Some(policy);
        self
    }

    pub fn retries(mut self, max_attempts: i32) -> Self {
        self.retry_policy
            .get_or_insert_with(Default::default)
            .maximum_attempts = Some(max_attempts);
        self
    }

    async fn execute(self) -> anyhow::Result<String> {
        let input_bytes = serde_json::to_vec(&self.input)?;
        let context_bytes = self
            .context
            .map(|c| serde_json::to_vec(&c))
            .transpose()?
            .unwrap_or_default();

        let resp = self
            .client
            .client
            .start_workflow(StartWorkflowRequest {
                workflow_id: self
                    .workflow_id
                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                task_queue: self.task_queue,
                workflow_type: self.workflow_type,
                input: input_bytes,
                namespace_id: "default".to_string(),
                idempotency_key: self.idempotency_key.unwrap_or_default(),
                context: context_bytes,
                deadline_at: self.deadline_at.map(|dt| prost_types::Timestamp {
                    seconds: dt.timestamp(),
                    nanos: dt.timestamp_subsec_nanos() as i32,
                }),
                version: self.version.unwrap_or_default(),
                retry_policy: self.retry_policy.map(Into::into),
            })
            .await
            .map_err(map_grpc_error)?;

        Ok(resp.into_inner().run_id)
    }
}

impl<'a, I: Serialize + Send + 'a> IntoFuture for WorkflowBuilder<'a, I> {
    type Output = anyhow::Result<String>;
    type IntoFuture = Pin<Box<dyn Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(self.execute())
    }
}

pub struct ScheduleBuilder<'a> {
    client: &'a mut Client,
    namespace_id: String,
    task_queue: String,
    workflow_type: String,
    cron_expr: String,
    input: serde_json::Value,
    context: Option<serde_json::Value>,
    enabled: Option<bool>,
    max_catchup: Option<i32>,
    version: Option<String>,
}

impl<'a> ScheduleBuilder<'a> {
    fn new(client: &'a mut Client, workflow_type: &str, task_queue: &str, cron_expr: &str) -> Self {
        Self {
            client,
            namespace_id: "default".to_string(),
            task_queue: task_queue.to_string(),
            workflow_type: workflow_type.to_string(),
            cron_expr: cron_expr.to_string(),
            input: serde_json::json!(null),
            context: None,
            enabled: None,
            max_catchup: None,
            version: None,
        }
    }

    pub fn namespace(mut self, namespace_id: impl Into<String>) -> Self {
        self.namespace_id = namespace_id.into();
        self
    }

    pub fn input(mut self, input: serde_json::Value) -> Self {
        self.input = input;
        self
    }

    pub fn context(mut self, context: serde_json::Value) -> Self {
        self.context = Some(context);
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = Some(enabled);
        self
    }

    pub fn max_catchup(mut self, max_catchup: i32) -> Self {
        self.max_catchup = Some(max_catchup);
        self
    }

    pub fn version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }

    pub async fn create(self) -> anyhow::Result<ProtoSchedule> {
        let input_bytes = serde_json::to_vec(&self.input)?;
        let context_bytes = self
            .context
            .map(|c| serde_json::to_vec(&c))
            .transpose()?
            .unwrap_or_default();

        let resp = self
            .client
            .client
            .create_schedule(CreateScheduleRequest {
                namespace_id: self.namespace_id,
                task_queue: self.task_queue,
                workflow_type: self.workflow_type,
                cron_expr: self.cron_expr,
                input: input_bytes,
                context: context_bytes,
                enabled: self.enabled,
                max_catchup: self.max_catchup.unwrap_or(0),
                version: self.version.unwrap_or_default(),
            })
            .await
            .map_err(map_grpc_error)?
            .into_inner();

        resp.schedule
            .ok_or_else(|| anyhow::anyhow!("Missing schedule in response"))
    }
}

#[derive(Default)]
pub struct ScheduleUpdate {
    task_queue: Option<String>,
    workflow_type: Option<String>,
    cron_expr: Option<String>,
    input: Option<serde_json::Value>,
    context: Option<serde_json::Value>,
    enabled: Option<bool>,
    max_catchup: Option<i32>,
    next_fire_at: Option<chrono::DateTime<chrono::Utc>>,
    version: Option<String>,
}

impl ScheduleUpdate {
    pub fn task_queue(mut self, value: impl Into<String>) -> Self {
        self.task_queue = Some(value.into());
        self
    }

    pub fn workflow_type(mut self, value: impl Into<String>) -> Self {
        self.workflow_type = Some(value.into());
        self
    }

    pub fn cron_expr(mut self, value: impl Into<String>) -> Self {
        self.cron_expr = Some(value.into());
        self
    }

    pub fn input(mut self, value: serde_json::Value) -> Self {
        self.input = Some(value);
        self
    }

    pub fn context(mut self, value: serde_json::Value) -> Self {
        self.context = Some(value);
        self
    }

    pub fn enabled(mut self, value: bool) -> Self {
        self.enabled = Some(value);
        self
    }

    pub fn max_catchup(mut self, value: i32) -> Self {
        self.max_catchup = Some(value);
        self
    }

    pub fn next_fire_at(mut self, value: chrono::DateTime<chrono::Utc>) -> Self {
        self.next_fire_at = Some(value);
        self
    }

    pub fn version(mut self, value: impl Into<String>) -> Self {
        self.version = Some(value.into());
        self
    }
}
