use std::collections::HashMap;
use std::future::{Future, IntoFuture};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use kagzi_proto::kagzi::workflow_service_client::WorkflowServiceClient;
use kagzi_proto::kagzi::{
    BeginStepRequest, CompleteStepRequest, CompleteWorkflowRequest, FailStepRequest,
    FailWorkflowRequest, PollActivityRequest, RecordHeartbeatRequest, ScheduleSleepRequest,
    StartWorkflowRequest,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tonic::Request;
use tonic::transport::Channel;
use tracing::{error, info, instrument};
use tracing_utils::{
    add_tracing_metadata, get_or_generate_correlation_id, get_or_generate_trace_id,
};

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

/// Error that skips all retries
#[derive(Debug)]
pub struct NonRetryableError(pub String);

impl std::fmt::Display for NonRetryableError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl std::error::Error for NonRetryableError {}

/// Error with custom retry delay
#[derive(Debug)]
pub struct RetryAfterError {
    pub message: String,
    pub retry_after: Duration,
}

impl RetryAfterError {
    pub fn new(message: impl Into<String>, retry_after: Duration) -> Self {
        Self {
            message: message.into(),
            retry_after,
        }
    }
}

impl std::fmt::Display for RetryAfterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}
impl std::error::Error for RetryAfterError {}

pub struct WorkflowContext {
    client: WorkflowServiceClient<Channel>,
    run_id: String,
    sleep_counter: u32,
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
            retry_policy: None,
        }));

        let begin_resp = self.client.begin_step(begin_request).await?.into_inner();

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

                self.client.complete_step(complete_request).await?;
                Ok(val)
            }
            Err(e) => {
                error!(error = %e, "Step {} failed", step_id);

                let non_retryable = e.downcast_ref::<NonRetryableError>().is_some();
                let retry_after_ms = e
                    .downcast_ref::<RetryAfterError>()
                    .map(|e| e.retry_after.as_millis() as i64)
                    .unwrap_or(0);

                let fail_request = add_tracing_metadata(Request::new(FailStepRequest {
                    run_id: self.run_id.clone(),
                    step_id: step_id.to_string(),
                    error: e.to_string(),
                    non_retryable,
                    retry_after_ms,
                }));

                self.client.fail_step(fail_request).await?;
                Err(e)
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
        let input_bytes = serde_json::to_vec(input)?;
        let begin_request = add_tracing_metadata(Request::new(BeginStepRequest {
            run_id: self.run_id.clone(),
            step_id: step_id.to_string(),
            input: input_bytes,
            retry_policy: None,
        }));

        let begin_resp = self.client.begin_step(begin_request).await?.into_inner();

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

                self.client.complete_step(complete_request).await?;
                Ok(val)
            }
            Err(e) => {
                error!(error = %e, "Step {} failed", step_id);

                let non_retryable = e.downcast_ref::<NonRetryableError>().is_some();
                let retry_after_ms = e
                    .downcast_ref::<RetryAfterError>()
                    .map(|e| e.retry_after.as_millis() as i64)
                    .unwrap_or(0);

                let fail_request = add_tracing_metadata(Request::new(FailStepRequest {
                    run_id: self.run_id.clone(),
                    step_id: step_id.to_string(),
                    error: e.to_string(),
                    non_retryable,
                    retry_after_ms,
                }));

                self.client.fail_step(fail_request).await?;
                Err(e)
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

        let begin_resp = self.client.begin_step(begin_request).await?.into_inner();

        if !begin_resp.should_execute {
            return Ok(());
        }

        let sleep_request = add_tracing_metadata(Request::new(ScheduleSleepRequest {
            run_id: self.run_id.clone(),
            duration_seconds: duration.as_secs(),
        }));

        self.client.schedule_sleep(sleep_request).await?;

        let complete_request = add_tracing_metadata(Request::new(CompleteStepRequest {
            run_id: self.run_id.clone(),
            step_id,
            output: serde_json::to_vec(&())?,
        }));
        self.client.complete_step(complete_request).await?;

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

pub struct Worker {
    client: WorkflowServiceClient<Channel>,
    task_queue: String,
    workflows: HashMap<String, Arc<WorkflowFn>>,
    worker_id: String,
}

impl Worker {
    #[instrument(fields(
        correlation_id = %get_or_generate_correlation_id(),
        trace_id = %get_or_generate_trace_id(),
        task_queue = %task_queue
    ))]
    pub async fn new(addr: &str, task_queue: &str) -> anyhow::Result<Self> {
        let client = WorkflowServiceClient::connect(addr.to_string()).await?;
        let worker_id = uuid::Uuid::new_v4().to_string();

        Ok(Self {
            client,
            task_queue: task_queue.to_string(),
            workflows: HashMap::new(),
            worker_id,
        })
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

    #[instrument(skip(self), fields(
        correlation_id = %get_or_generate_correlation_id(),
        trace_id = %get_or_generate_trace_id(),
        worker_id = %self.worker_id,
        task_queue = %self.task_queue
    ))]
    pub async fn run(&mut self) -> anyhow::Result<()> {
        info!(
            "Worker {} started on queue {}",
            self.worker_id, self.task_queue
        );
        loop {
            let poll_request = add_tracing_metadata(Request::new(PollActivityRequest {
                task_queue: self.task_queue.clone(),
                worker_id: self.worker_id.clone(),
                namespace_id: "default".to_string(),
            }));

            let resp = self.client.poll_activity(poll_request).await;

            match resp {
                Ok(r) => {
                    let task = r.into_inner();

                    // Empty response means no work available (timeout)
                    if task.run_id.is_empty() {
                        continue;
                    }

                    info!(
                        run_id = %task.run_id,
                        workflow_type = %task.workflow_type,
                        "Received task"
                    );

                    if let Some(handler) = self.workflows.get(&task.workflow_type) {
                        let handler = handler.clone();
                        let client = self.client.clone();
                        let run_id = task.run_id.clone();
                        let worker_id = self.worker_id.clone();
                        let input: serde_json::Value = serde_json::from_slice(&task.workflow_input)
                            .unwrap_or(serde_json::Value::Null);

                        tokio::spawn(async move {
                            // Generate tracing IDs for this workflow execution
                            let correlation_id = uuid::Uuid::new_v4().to_string();
                            let trace_id = uuid::Uuid::new_v4().to_string();

                            // Execute workflow with tracing context
                            tracing_utils::with_tracing_context(
                                Some(correlation_id.clone()),
                                Some(trace_id.clone()),
                                execute_workflow(
                                    client,
                                    handler,
                                    run_id,
                                    worker_id,
                                    input,
                                    correlation_id,
                                    trace_id,
                                ),
                            )
                            .await;
                        });
                    } else {
                        error!("No handler for workflow type: {}", task.workflow_type);
                    }
                }
                Err(status) => {
                    if status.code() != tonic::Code::DeadlineExceeded
                        && status.code() != tonic::Code::NotFound
                    {
                        error!("Poll failed: {:?}", status);
                    }
                }
            }
        }
    }
}

/// Execute a workflow with proper tracing context and heartbeat management
async fn execute_workflow(
    mut client: WorkflowServiceClient<Channel>,
    handler: Arc<WorkflowFn>,
    run_id: String,
    worker_id: String,
    input: serde_json::Value,
    correlation_id: String,
    trace_id: String,
) {
    // Start background heartbeat task
    let heartbeat_handle = {
        let mut heartbeat_client = client.clone();
        let heartbeat_run_id = run_id.clone();
        let heartbeat_worker_id = worker_id.clone();

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(10));
            loop {
                interval.tick().await;
                let request = add_tracing_metadata(Request::new(RecordHeartbeatRequest {
                    run_id: heartbeat_run_id.clone(),
                    worker_id: heartbeat_worker_id.clone(),
                }));
                if let Err(e) = heartbeat_client.record_heartbeat(request).await {
                    // Log but don't fail - heartbeat errors are non-fatal
                    // The workflow might have been completed/failed already
                    tracing::debug!(
                        run_id = %heartbeat_run_id,
                        error = %e,
                        "Heartbeat failed (workflow may have completed)"
                    );
                    break;
                }
            }
        })
    };

    let ctx = WorkflowContext {
        client: client.clone(),
        run_id: run_id.clone(),
        sleep_counter: 0,
    };

    let result = handler(ctx, input).await;

    // Stop heartbeat task
    heartbeat_handle.abort();

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

            let fail_request = add_tracing_metadata(Request::new(FailWorkflowRequest {
                run_id,
                error: e.to_string(),
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
            .await?;

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
