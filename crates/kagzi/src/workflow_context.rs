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
use tracing::{error, instrument};

use crate::errors::{KagziError, WorkflowPaused, map_grpc_error};
use crate::retry::RetryPolicy;
use crate::tracing_utils::{
    add_tracing_metadata, get_or_generate_correlation_id, get_or_generate_trace_id,
};

pub struct WorkflowContext {
    pub(crate) client: WorkerServiceClient<Channel>,
    pub(crate) run_id: String,
    pub(crate) sleep_counter: u32,
    pub(crate) default_step_retry: Option<RetryPolicy>,
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
            step_name: step_id.to_string(),
            kind: StepKind::Function as i32,
            input: Some(ProtoPayload {
                data: Vec::new(),
                metadata: HashMap::new(),
            }),
            retry_policy: self.default_step_retry.clone().map(Into::into),
        }));

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
                let complete_request = add_tracing_metadata(Request::new(CompleteStepRequest {
                    run_id: self.run_id.clone(),
                    step_id: step_id_resp.clone(),
                    output: Some(ProtoPayload {
                        data: output_bytes,
                        metadata: HashMap::new(),
                    }),
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
                    step_id: step_id_resp,
                    error: Some(kagzi_err.to_detail()),
                }));

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
            step_name: step_id.to_string(),
            kind: StepKind::Function as i32,
            input: Some(ProtoPayload {
                data: input_bytes,
                metadata: HashMap::new(),
            }),
            retry_policy: effective_retry.clone().map(Into::into),
        }));

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
                let complete_request = add_tracing_metadata(Request::new(CompleteStepRequest {
                    run_id: self.run_id.clone(),
                    step_id: step_id_resp.clone(),
                    output: Some(ProtoPayload {
                        data: output_bytes,
                        metadata: HashMap::new(),
                    }),
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
                    step_id: step_id_resp,
                    error: Some(kagzi_err.to_detail()),
                }));

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
            step_name: step_id.clone(),
            kind: StepKind::Sleep as i32,
            input: Some(ProtoPayload {
                data: Vec::new(),
                metadata: HashMap::new(),
            }),
            retry_policy: None,
        }));

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

        let sleep_request = add_tracing_metadata(Request::new(SleepRequest {
            run_id: self.run_id.clone(),
            step_id: step_id_resp.clone(),
            duration: Some(ProstDuration {
                seconds: duration.as_secs() as i64,
                nanos: 0,
            }),
        }));

        self.client
            .sleep(sleep_request)
            .await
            .map_err(map_grpc_error)?;

        let complete_request = add_tracing_metadata(Request::new(CompleteStepRequest {
            run_id: self.run_id.clone(),
            step_id: step_id_resp,
            output: Some(ProtoPayload {
                data: serde_json::to_vec(&())?,
                metadata: HashMap::new(),
            }),
        }));
        self.client
            .complete_step(complete_request)
            .await
            .map_err(map_grpc_error)?;

        Err(WorkflowPaused.into())
    }
}
