use std::collections::HashMap;
use std::future::Future;
use std::time::Duration;

use kagzi_proto::kagzi::worker_service_client::WorkerServiceClient;
use kagzi_proto::kagzi::{
    BeginStepRequest, CompleteStepRequest, ErrorCode, FailStepRequest, Payload as ProtoPayload,
    SleepRequest, StepKind,
};
use prost_types::Duration as ProstDuration;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tonic::Request;
use tonic::transport::Channel;
use tracing::{error, warn};

use crate::errors::{KagziError, WorkflowPaused, map_grpc_error};
use crate::retry::RetryPolicy;

pub struct WorkflowContext {
    pub(crate) client: WorkerServiceClient<Channel>,
    pub(crate) run_id: String,
    pub(crate) sleep_counter: u32,
    pub(crate) default_step_retry: Option<RetryPolicy>,
}

impl WorkflowContext {
    pub async fn run<R, Fut>(&mut self, step_id: &str, fut: Fut) -> anyhow::Result<R>
    where
        R: Serialize + DeserializeOwned + Send + 'static,
        Fut: Future<Output = anyhow::Result<R>> + Send,
    {
        let begin_request = Request::new(BeginStepRequest {
            run_id: self.run_id.clone(),
            step_name: step_id.to_string(),
            kind: StepKind::Function as i32,
            input: Some(ProtoPayload {
                data: Vec::new(),
                metadata: HashMap::new(),
            }),
            retry_policy: self.default_step_retry.clone().map(Into::into),
        });

        let begin_resp = self
            .client
            .begin_step(begin_request)
            .await
            .map_err(map_grpc_error)?
            .into_inner();

        let step_id_resp = if begin_resp.step_id.is_empty() {
            step_id.to_string()
        } else {
            begin_resp.step_id.clone()
        };

        if !begin_resp.should_execute {
            let cached = begin_resp.cached_output.unwrap_or(ProtoPayload {
                data: Vec::new(),
                metadata: HashMap::new(),
            });
            let result: R = serde_json::from_slice(&cached.data)?;
            return Ok(result);
        }

        let result = fut.await;

        match result {
            Ok(val) => {
                let output_bytes = serde_json::to_vec(&val)?;

                let mut backoff = Duration::from_secs(1);
                loop {
                    let complete_request = Request::new(CompleteStepRequest {
                        run_id: self.run_id.clone(),
                        step_id: step_id_resp.clone(),
                        output: Some(ProtoPayload {
                            data: output_bytes.clone(),
                            metadata: HashMap::new(),
                        }),
                    });

                    match self.client.complete_step(complete_request).await {
                        Ok(_) => break,

                        Err(e) if e.code() == tonic::Code::NotFound => {
                            error!("Workflow deleted during execution: {}", e);
                            return Err(map_grpc_error(e));
                        }

                        Err(e) => {
                            warn!("CompleteStep failed, retrying in {:?}: {}", backoff, e);
                            tokio::time::sleep(backoff).await;
                            backoff = std::cmp::min(backoff * 2, Duration::from_secs(60));
                        }
                    }
                }
                Ok(val)
            }
            Err(e) => {
                error!(error = %e, "Step {} failed", step_id);

                let kagzi_err = e
                    .downcast_ref::<KagziError>()
                    .cloned()
                    .unwrap_or_else(|| KagziError::new(ErrorCode::Internal, e.to_string()));

                let fail_request = Request::new(FailStepRequest {
                    run_id: self.run_id.clone(),
                    step_id: step_id_resp,
                    error: Some(kagzi_err.to_detail()),
                });

                let fail_resp = self
                    .client
                    .fail_step(fail_request)
                    .await
                    .map_err(map_grpc_error)?
                    .into_inner();

                if fail_resp.scheduled_retry {
                    return Err(WorkflowPaused.into());
                }

                Err(anyhow::Error::new(kagzi_err))
            }
        }
    }

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
        let begin_request = Request::new(BeginStepRequest {
            run_id: self.run_id.clone(),
            step_name: step_id.to_string(),
            kind: StepKind::Function as i32,
            input: Some(ProtoPayload {
                data: input_bytes,
                metadata: HashMap::new(),
            }),
            retry_policy: effective_retry.clone().map(Into::into),
        });

        let begin_resp = self
            .client
            .begin_step(begin_request)
            .await
            .map_err(map_grpc_error)?
            .into_inner();

        let step_id_resp = if begin_resp.step_id.is_empty() {
            step_id.to_string()
        } else {
            begin_resp.step_id.clone()
        };

        if !begin_resp.should_execute {
            let cached = begin_resp.cached_output.unwrap_or(ProtoPayload {
                data: Vec::new(),
                metadata: HashMap::new(),
            });
            let result: R = serde_json::from_slice(&cached.data)?;
            return Ok(result);
        }

        let result = fut.await;

        match result {
            Ok(val) => {
                let output_bytes = serde_json::to_vec(&val)?;

                // Infinite retry for complete_step to prevent "zombie success" bug
                // If the user function succeeded, we MUST persist that success to the DB
                let mut backoff = Duration::from_secs(1);
                loop {
                    let complete_request = Request::new(CompleteStepRequest {
                        run_id: self.run_id.clone(),
                        step_id: step_id_resp.clone(),
                        output: Some(ProtoPayload {
                            data: output_bytes.clone(),
                            metadata: HashMap::new(),
                        }),
                    });

                    match self.client.complete_step(complete_request).await {
                        Ok(_) => break, // Success!

                        Err(e) if e.code() == tonic::Code::NotFound => {
                            // Workflow was deleted - unrecoverable
                            error!("Workflow deleted during execution: {}", e);
                            return Err(map_grpc_error(e));
                        }

                        Err(e) => {
                            // Transient error - retry with exponential backoff
                            warn!("CompleteStep failed, retrying in {:?}: {}", backoff, e);
                            tokio::time::sleep(backoff).await;
                            backoff = std::cmp::min(backoff * 2, Duration::from_secs(60));
                        }
                    }
                }

                Ok(val)
            }
            Err(e) => {
                error!(error = %e, "Step {} failed", step_id);

                let kagzi_err = e
                    .downcast_ref::<KagziError>()
                    .cloned()
                    .unwrap_or_else(|| KagziError::new(ErrorCode::Internal, e.to_string()));

                let fail_request = Request::new(FailStepRequest {
                    run_id: self.run_id.clone(),
                    step_id: step_id_resp,
                    error: Some(kagzi_err.to_detail()),
                });

                let fail_resp = self
                    .client
                    .fail_step(fail_request)
                    .await
                    .map_err(map_grpc_error)?
                    .into_inner();

                if fail_resp.scheduled_retry {
                    return Err(WorkflowPaused.into());
                }

                Err(anyhow::Error::new(kagzi_err))
            }
        }
    }

    pub async fn sleep(&mut self, duration: Duration) -> anyhow::Result<()> {
        let step_id = format!("__sleep_{}", self.sleep_counter);
        self.sleep_counter += 1;

        let begin_request = Request::new(BeginStepRequest {
            run_id: self.run_id.clone(),
            step_name: step_id.clone(),
            kind: StepKind::Sleep as i32,
            input: Some(ProtoPayload {
                data: Vec::new(),
                metadata: HashMap::new(),
            }),
            retry_policy: None,
        });

        let begin_resp = self
            .client
            .begin_step(begin_request)
            .await
            .map_err(map_grpc_error)?
            .into_inner();

        let step_id_resp = if begin_resp.step_id.is_empty() {
            step_id.clone()
        } else {
            begin_resp.step_id.clone()
        };

        if !begin_resp.should_execute {
            return Ok(());
        }

        let sleep_request = Request::new(SleepRequest {
            run_id: self.run_id.clone(),
            step_id: step_id_resp.clone(),
            duration: Some(ProstDuration {
                seconds: duration.as_secs() as i64,
                nanos: 0,
            }),
        });

        self.client
            .sleep(sleep_request)
            .await
            .map_err(map_grpc_error)?;

        // Step will be completed lazily when workflow resumes
        // (see server's begin_step implementation for lazy sleep completion)
        Err(WorkflowPaused.into())
    }
}
