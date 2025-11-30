use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use kagzi_proto::kagzi::workflow_service_client::WorkflowServiceClient;
use kagzi_proto::kagzi::{
    BeginStepRequest, CompleteStepRequest, CompleteWorkflowRequest, FailStepRequest,
    FailWorkflowRequest, PollActivityRequest, ScheduleSleepRequest, StartWorkflowRequest,
};
use serde::de::DeserializeOwned;
use serde::Serialize;
use tonic::transport::Channel;
use tracing::{error, info};

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub struct WorkflowContext {
    client: WorkflowServiceClient<Channel>,
    run_id: String,
}

impl WorkflowContext {
    pub async fn step<R, F, Fut>(&mut self, step_id: &str, func: F) -> anyhow::Result<R>
    where
        R: Serialize + DeserializeOwned + Send + 'static,
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = anyhow::Result<R>> + Send,
    {
        // 1. Check if step already ran
        let begin_resp = self
            .client
            .begin_step(BeginStepRequest {
                run_id: self.run_id.clone(),
                step_id: step_id.to_string(),
            })
            .await?
            .into_inner();

        if !begin_resp.should_execute {
            let result: R = serde_json::from_slice(&begin_resp.cached_result)?;
            return Ok(result);
        }

        // 2. Run the step
        let result = func().await;

        match result {
            Ok(val) => {
                let output_bytes = serde_json::to_vec(&val)?;
                self.client
                    .complete_step(CompleteStepRequest {
                        run_id: self.run_id.clone(),
                        step_id: step_id.to_string(),
                        output: output_bytes,
                    })
                    .await?;
                Ok(val)
            }
            Err(e) => {
                self.client
                    .fail_step(FailStepRequest {
                        run_id: self.run_id.clone(),
                        step_id: step_id.to_string(),
                        error: e.to_string(),
                    })
                    .await?;
                Err(e)
            }
        }
    }

    pub async fn sleep(&mut self, duration: Duration) -> anyhow::Result<()> {
        self.client
            .schedule_sleep(ScheduleSleepRequest {
                run_id: self.run_id.clone(),
                duration_seconds: duration.as_secs(),
            })
            .await?;

        // We return a special error to unwind the stack, or we could handle control flow differently.
        // For now, let's assume the worker loop handles the "Sleep" state by checking the DB,
        // but here we just return Ok and expect the user code to return.
        // Actually, to stop execution, we should probably return a specific error or panic?
        // Or better, the user code awaits this, and we block? No, we can't block.
        // We need to signal the runner to stop.
        // For this MVP, we'll just return Ok, but the server has already set status to SLEEPING.
        // If the workflow continues, it might try to execute more steps, which is fine,
        // but ideally it should stop.
        // Let's use a special error for now to interrupt flow if needed, or just let it finish current scope.
        Ok(())
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
    pub async fn new(addr: String, task_queue: String) -> anyhow::Result<Self> {
        let client = WorkflowServiceClient::connect(addr).await?;
        Ok(Self {
            client,
            task_queue,
            workflows: HashMap::new(),
            worker_id: uuid::Uuid::new_v4().to_string(),
        })
    }

    pub fn register_workflow<F, Fut, I, O>(&mut self, name: &str, func: F)
    where
        F: Fn(WorkflowContext, I) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = anyhow::Result<O>> + Send + 'static,
        I: DeserializeOwned + Send + 'static,
        O: Serialize + Send + 'static,
    {
        let wrapped = move |ctx: WorkflowContext,
                            input_val: serde_json::Value|
              -> BoxFuture<'static, anyhow::Result<serde_json::Value>> {
            let input: I = serde_json::from_value(input_val).unwrap(); // Handle error better
            let fut = func(ctx, input);
            Box::pin(async move {
                let output = fut.await?;
                Ok(serde_json::to_value(output)?)
            })
        };
        self.workflows
            .insert(name.to_string(), Arc::new(Box::new(wrapped)));
    }

    pub async fn run(&mut self) -> anyhow::Result<()> {
        info!(
            "Worker {} started on queue {}",
            self.worker_id, self.task_queue
        );
        loop {
            let resp = self
                .client
                .poll_activity(PollActivityRequest {
                    task_queue: self.task_queue.clone(),
                    worker_id: self.worker_id.clone(),
                })
                .await;

            match resp {
                Ok(r) => {
                    let task = r.into_inner();
                    info!("Received task: {}", task.run_id);

                    if let Some(handler) = self.workflows.get(&task.workflow_type) {
                        let handler = handler.clone();
                        let mut client = self.client.clone();
                        let run_id = task.run_id.clone();
                        let input: serde_json::Value = serde_json::from_slice(&task.workflow_input)
                            .unwrap_or(serde_json::Value::Null);

                        tokio::spawn(async move {
                            let ctx = WorkflowContext {
                                client: client.clone(),
                                run_id: run_id.clone(),
                            };

                            match handler(ctx, input).await {
                                Ok(output) => {
                                    let output_bytes = serde_json::to_vec(&output).unwrap();
                                    let _ = client
                                        .complete_workflow(CompleteWorkflowRequest {
                                            run_id,
                                            output: output_bytes,
                                        })
                                        .await;
                                }
                                Err(e) => {
                                    let _ = client
                                        .fail_workflow(FailWorkflowRequest {
                                            run_id,
                                            error: e.to_string(),
                                        })
                                        .await;
                                }
                            }
                        });
                    } else {
                        error!("No handler for workflow type: {}", task.workflow_type);
                    }
                }
                Err(status) => {
                    if status.code() != tonic::Code::DeadlineExceeded {
                        error!("Poll failed: {:?}", status);
                    }
                }
            }
        }
    }
}

// Client for starting workflows
pub struct Client {
    client: WorkflowServiceClient<Channel>,
}

impl Client {
    pub async fn new(addr: String) -> anyhow::Result<Self> {
        let client = WorkflowServiceClient::connect(addr).await?;
        Ok(Self { client })
    }

    pub async fn start_workflow(
        &mut self,
        workflow_id: String,
        task_queue: String,
        workflow_type: String,
        input: impl Serialize,
    ) -> anyhow::Result<String> {
        let input_bytes = serde_json::to_vec(&input)?;
        let resp = self
            .client
            .start_workflow(StartWorkflowRequest {
                workflow_id,
                task_queue,
                workflow_type,
                input: input_bytes,
            })
            .await?;
        Ok(resp.into_inner().run_id)
    }
}
