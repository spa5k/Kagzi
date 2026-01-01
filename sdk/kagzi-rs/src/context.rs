use std::collections::HashMap;
use std::future::Future;
use std::time::Duration;

use chrono::Utc;
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
use tracing::{Instrument, error, info_span, warn};

use crate::errors::{KagziError, WorkflowPaused, map_grpc_error};
use crate::propagation::inject_context;
use crate::retry::Retry;

pub struct Context {
    pub(crate) client: WorkerServiceClient<Channel>,
    pub(crate) run_id: String,
    pub(crate) default_retry: Option<Retry>,
}

impl Context {
    /// Start building a step
    pub fn step(&mut self, name: &str) -> StepBuilder<'_> {
        StepBuilder {
            ctx: self,
            name: name.to_string(),
            retry: None,
        }
    }

    /// Sleep for a duration with a descriptive name
    ///
    /// The name is used for observability (logs, UI) but does not need to be unique.
    ///
    /// Duration formats: "30s", "5m", "1h", "1 day", "2 weeks"
    pub async fn sleep(&mut self, name: &str, duration: &str) -> anyhow::Result<()> {
        let d = humantime::parse_duration(duration)
            .map_err(|e| anyhow::anyhow!("Invalid duration '{}': {}", duration, e))?;
        self.sleep_internal(name, d).await
    }

    /// Sleep until a specific timestamp
    pub async fn sleep_until(
        &mut self,
        name: &str,
        until: chrono::DateTime<Utc>,
    ) -> anyhow::Result<()> {
        let now = Utc::now();
        if until <= now {
            return Ok(()); // Already past, no sleep needed
        }
        let duration = (until - now).to_std().unwrap_or(Duration::ZERO);
        self.sleep_internal(name, duration).await
    }

    async fn sleep_internal(&mut self, name: &str, duration: Duration) -> anyhow::Result<()> {
        let mut begin_request = Request::new(BeginStepRequest {
            run_id: self.run_id.clone(),
            step_name: name.to_string(),
            kind: StepKind::Sleep as i32,
            input: Some(ProtoPayload {
                data: Vec::new(),
                metadata: HashMap::new(),
            }),
            retry_policy: None,
        });
        inject_context(begin_request.metadata_mut());

        let begin_resp = self
            .client
            .begin_step(begin_request)
            .await
            .map_err(map_grpc_error)?
            .into_inner();

        let step_id = if begin_resp.step_id.is_empty() {
            name.to_string()
        } else {
            begin_resp.step_id.clone()
        };

        if !begin_resp.should_execute {
            return Ok(());
        }

        let mut sleep_request = Request::new(SleepRequest {
            run_id: self.run_id.clone(),
            step_id: step_id.clone(),
            duration: Some(ProstDuration {
                seconds: duration.as_secs() as i64,
                nanos: duration.subsec_nanos() as i32,
            }),
        });
        inject_context(sleep_request.metadata_mut());

        self.client
            .sleep(sleep_request)
            .await
            .map_err(map_grpc_error)?;

        // Signal that workflow should pause
        Err(WorkflowPaused.into())
    }
}

pub struct StepBuilder<'a> {
    ctx: &'a mut Context,
    name: String,
    retry: Option<Retry>,
}

impl<'a> StepBuilder<'a> {
    /// Override retry policy for this step
    pub fn retry(mut self, r: Retry) -> Self {
        self.retry = Some(r);
        self
    }

    /// Execute the step
    pub async fn run<F, Fut, R>(self, f: F) -> anyhow::Result<R>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = anyhow::Result<R>> + Send,
        R: Serialize + DeserializeOwned + Send + 'static,
    {
        let span = info_span!(
            "step_execution",
            run_id = %self.ctx.run_id,
            step_name = %self.name,
            otel.kind = "client",
        );

        self.run_instrumented(f).instrument(span).await
    }

    async fn run_instrumented<F, Fut, R>(self, f: F) -> anyhow::Result<R>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = anyhow::Result<R>> + Send,
        R: Serialize + DeserializeOwned + Send + 'static,
    {
        // Determine effective retry policy
        let retry = self.retry.or_else(|| self.ctx.default_retry.clone());

        // Call BeginStep with trace context
        let mut begin_request = Request::new(BeginStepRequest {
            run_id: self.ctx.run_id.clone(),
            step_name: self.name.clone(),
            kind: StepKind::Function as i32,
            input: None, // No longer serializing input
            retry_policy: retry.map(Into::into),
        });
        inject_context(begin_request.metadata_mut());

        let begin_resp = self
            .ctx
            .client
            .begin_step(begin_request)
            .await?
            .into_inner();

        let step_id = if begin_resp.step_id.is_empty() {
            self.name.clone()
        } else {
            begin_resp.step_id.clone()
        };

        // Return cached result if step already completed
        if !begin_resp.should_execute {
            let cached = begin_resp.cached_output.unwrap_or_default();
            return Ok(serde_json::from_slice(&cached.data)?);
        }

        // Execute the step
        let result = f().await;

        // Report result
        match result {
            Ok(value) => {
                let output = serde_json::to_vec(&value)?;
                let mut backoff = Duration::from_secs(1);
                loop {
                    let mut complete_request = Request::new(CompleteStepRequest {
                        run_id: self.ctx.run_id.clone(),
                        step_id: step_id.clone(),
                        output: Some(ProtoPayload {
                            data: output.clone(),
                            metadata: HashMap::new(),
                        }),
                    });
                    inject_context(complete_request.metadata_mut());

                    match self.ctx.client.complete_step(complete_request).await {
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
                Ok(value)
            }
            Err(e) => {
                error!(error = %e, "Step {} failed", self.name);

                let kagzi_err = e
                    .downcast_ref::<KagziError>()
                    .cloned()
                    .unwrap_or_else(|| KagziError::new(ErrorCode::Internal, e.to_string()));

                let mut fail_request = Request::new(FailStepRequest {
                    run_id: self.ctx.run_id.clone(),
                    step_id: step_id.clone(),
                    error: Some(kagzi_err.to_detail()),
                });
                inject_context(fail_request.metadata_mut());

                let fail_resp = self
                    .ctx
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
}
