use std::time::Duration;

use kagzi_proto::kagzi::worker_service_server::WorkerService;
use kagzi_proto::kagzi::{
    BeginStepRequest, BeginStepResponse, CompleteStepRequest, CompleteStepResponse,
    CompleteWorkflowRequest, CompleteWorkflowResponse, DeregisterRequest, ErrorCode, ErrorDetail,
    FailStepRequest, FailStepResponse, FailWorkflowRequest, FailWorkflowResponse, HeartbeatRequest,
    HeartbeatResponse, PollTaskRequest, PollTaskResponse, RegisterRequest, RegisterResponse,
    SleepRequest,
};
use kagzi_queue::QueueNotifier;
use kagzi_store::{
    BeginStepParams, FailStepParams, PgStore, RegisterWorkerParams, StepRepository,
    WorkerHeartbeatParams, WorkerRepository, WorkerStatus as StoreWorkerStatus, WorkflowRepository,
    WorkflowTypeConcurrency,
};
use rand::Rng;
use tonic::{Request, Response, Status};
use tracing::{info, instrument};
use tracing_opentelemetry::OpenTelemetrySpanExt;
use uuid::Uuid;

use crate::config::WorkerSettings;
use crate::helpers::{
    bytes_to_payload, invalid_argument_error, map_store_error, merge_proto_policy, not_found_error,
    payload_to_optional_bytes, precondition_failed_error, require_non_empty,
};
use crate::proto_convert::{empty_payload, map_proto_step_kind, step_to_proto};
use crate::telemetry::extract_context;

const MAX_QUEUE_CONCURRENCY: i32 = 10_000;
const MAX_TYPE_CONCURRENCY: i32 = 10_000;

fn normalize_limit(raw: i32, max_allowed: i32) -> Option<i32> {
    if raw <= 0 {
        None
    } else {
        Some(raw.min(max_allowed))
    }
}

#[derive(Clone)]
pub struct WorkerServiceImpl<Q: QueueNotifier = kagzi_queue::PostgresNotifier> {
    pub store: PgStore,
    pub worker_settings: WorkerSettings,
    pub queue_settings: crate::config::QueueSettings,
    pub queue: Q,
}

impl<Q: QueueNotifier> WorkerServiceImpl<Q> {
    pub fn new(
        store: PgStore,
        worker_settings: WorkerSettings,
        queue_settings: crate::config::QueueSettings,
        queue: Q,
    ) -> Self {
        Self {
            store,
            worker_settings,
            queue_settings,
            queue,
        }
    }

    async fn validate_workflow_action(
        &self,
        run_id: Uuid,
        allow_terminal: bool,
    ) -> Result<String, Status> {
        let namespace_id = self
            .store
            .workflows()
            .get_namespace(run_id)
            .await
            .map_err(map_store_error)?
            .ok_or_else(|| not_found_error("Workflow not found", "workflow", run_id.to_string()))?;

        let workflow_check = self
            .store
            .workflows()
            .check_status(run_id, &namespace_id)
            .await
            .map_err(map_store_error)?;

        if !workflow_check.exists {
            return Err(not_found_error(
                format!("Workflow not found: run_id={}", run_id),
                "workflow",
                run_id.to_string(),
            ));
        }

        if !allow_terminal
            && let Some(status) = workflow_check.status
            && status.is_terminal()
        {
            return Err(precondition_failed_error(format!(
                "Cannot perform action on workflow with terminal status '{:?}'",
                status
            )));
        }

        Ok(namespace_id)
    }
}

#[tonic::async_trait]
impl<Q: QueueNotifier + 'static> WorkerService for WorkerServiceImpl<Q> {
    #[instrument(skip(self, request), fields(task_queue = %request.get_ref().task_queue))]
    async fn register(
        &self,
        request: Request<RegisterRequest>,
    ) -> Result<Response<RegisterResponse>, Status> {
        let req = request.into_inner();

        if req.workflow_types.is_empty() {
            return Err(invalid_argument_error("workflow_types cannot be empty"));
        }

        let namespace_id = require_non_empty(req.namespace_id, "namespace_id")?;
        let workflow_types = req.workflow_types;

        // Clone for logging after worker_id is assigned
        let namespace_for_log = namespace_id.clone();
        let workflows_for_log = workflow_types.clone();

        let worker_id = self
            .store
            .workers()
            .register(RegisterWorkerParams {
                namespace_id,
                task_queue: req.task_queue,
                workflow_types,
                hostname: Some(req.hostname).filter(|s| !s.is_empty()),
                pid: (req.pid != 0).then_some(req.pid),
                version: Some(req.version).filter(|s| !s.is_empty()),
                labels: serde_json::to_value(&req.labels).unwrap_or_default(),
                queue_concurrency_limit: req
                    .queue_concurrency_limit
                    .and_then(|v| normalize_limit(v, MAX_QUEUE_CONCURRENCY)),
                workflow_type_concurrency: req
                    .workflow_type_concurrency
                    .into_iter()
                    .filter_map(|c| {
                        normalize_limit(c.max_concurrent, MAX_TYPE_CONCURRENCY).map(|limit| {
                            WorkflowTypeConcurrency {
                                workflow_type: c.workflow_type,
                                max_concurrent: limit,
                            }
                        })
                    })
                    .collect(),
            })
            .await
            .map_err(map_store_error)?;

        info!(
            worker_id = %worker_id,
            namespace = %namespace_for_log,
            workflows = ?workflows_for_log,
            "Worker connected"
        );

        Ok(Response::new(RegisterResponse {
            worker_id: worker_id.to_string(),
            heartbeat_interval_secs: self.worker_settings.heartbeat_interval_secs as i32,
        }))
    }

    #[instrument(skip(self, request), fields(worker_id = %request.get_ref().worker_id))]
    async fn heartbeat(
        &self,
        request: Request<HeartbeatRequest>,
    ) -> Result<Response<HeartbeatResponse>, Status> {
        let req = request.into_inner();
        let worker_id = Uuid::parse_str(&req.worker_id)
            .map_err(|_| invalid_argument_error("Invalid worker_id"))?;

        let accepted = self
            .store
            .workers()
            .heartbeat(WorkerHeartbeatParams { worker_id })
            .await
            .map_err(map_store_error)?;

        if !accepted {
            return Err(not_found_error(
                "Worker not found or offline",
                "worker",
                req.worker_id.clone(),
            ));
        }

        // Extend visibility for all workflows locked by this worker
        let extended = self
            .store
            .workflows()
            .extend_visibility(
                &req.worker_id,
                self.worker_settings.heartbeat_extension_secs,
            )
            .await
            .map_err(map_store_error)?;

        if extended > 0 {
            tracing::debug!(worker_id = %req.worker_id, count = extended, "Extended workflow visibility");
        }

        let worker = self
            .store
            .workers()
            .find_by_id(worker_id)
            .await
            .map_err(map_store_error)?;

        let should_drain = matches!(
            worker,
            Some(w) if w.status == StoreWorkerStatus::Draining
        );

        Ok(Response::new(HeartbeatResponse {
            accepted: true,
            should_drain,
        }))
    }

    #[instrument(skip(self, request), fields(worker_id = %request.get_ref().worker_id))]
    async fn deregister(
        &self,
        request: Request<DeregisterRequest>,
    ) -> Result<Response<()>, Status> {
        let req = request.into_inner();
        let worker_id = Uuid::parse_str(&req.worker_id)
            .map_err(|_| invalid_argument_error("Invalid worker_id"))?;

        let worker = self
            .store
            .workers()
            .find_by_id(worker_id)
            .await
            .map_err(map_store_error)?
            .ok_or_else(|| not_found_error("Worker not found", "worker", req.worker_id.clone()))?;

        let namespace_id = worker.namespace_id;

        if req.drain {
            self.store
                .workers()
                .start_drain(worker_id, &namespace_id)
                .await
                .map_err(map_store_error)?;
            info!(worker_id = %worker_id, namespace = %namespace_id, "Worker draining");
        } else {
            self.store
                .workers()
                .deregister(worker_id, &namespace_id)
                .await
                .map_err(map_store_error)?;
            info!(worker_id = %worker_id, namespace = %namespace_id, "Worker disconnected");
        }

        Ok(Response::new(()))
    }

    #[instrument(
        skip(self, request),
        fields(
            worker_id = %request.get_ref().worker_id,
            task_queue = %request.get_ref().task_queue,
        )
    )]
    async fn poll_task(
        &self,
        request: Request<PollTaskRequest>,
    ) -> Result<Response<PollTaskResponse>, Status> {
        let req = request.into_inner();

        let worker_id = Uuid::parse_str(&req.worker_id)
            .map_err(|_| invalid_argument_error("Invalid worker_id"))?;

        if req.workflow_types.is_empty() {
            return Err(invalid_argument_error("workflow_types cannot be empty"));
        }

        let namespace_id = require_non_empty(req.namespace_id, "namespace_id")?;

        let worker = self
            .store
            .workers()
            .find_by_id(worker_id)
            .await
            .map_err(map_store_error)?
            .ok_or_else(|| {
                precondition_failed_error("Worker not registered or offline. Call Register first.")
            })?;

        if worker.namespace_id != namespace_id || worker.task_queue != req.task_queue {
            return Err(precondition_failed_error(
                "Worker not registered for the requested namespace/task_queue",
            ));
        }

        if worker.status == StoreWorkerStatus::Offline {
            return Err(precondition_failed_error(
                "Worker not registered or offline. Call Register first.",
            ));
        }

        if worker.status == StoreWorkerStatus::Draining {
            return Err(precondition_failed_error(
                "Worker is draining and not accepting new work",
            ));
        }

        let effective_types: Vec<String> = worker
            .workflow_types
            .iter()
            .filter(|t| req.workflow_types.iter().any(|r| r == *t))
            .cloned()
            .collect();

        if effective_types.is_empty() {
            return Err(precondition_failed_error(
                "Worker is not registered for the requested workflow types",
            ));
        }

        let timeout = Duration::from_secs(self.worker_settings.poll_timeout_secs);
        let deadline = tokio::time::Instant::now() + timeout;

        loop {
            let work_item = self
                .store
                .workflows()
                .poll_workflow(
                    &namespace_id,
                    &req.task_queue,
                    &req.worker_id,
                    &effective_types,
                    self.worker_settings.visibility_timeout_secs,
                )
                .await
                .map_err(map_store_error)?;

            if let Some(work_item) = work_item {
                let _ = self.complete_pending_sleep_steps(work_item.run_id).await;

                info!(
                    run_id = %work_item.run_id,
                    workflow_type = %work_item.workflow_type,
                    worker_id = %req.worker_id,
                    "Dispatched workflow"
                );

                let payload = bytes_to_payload(Some(work_item.input));

                return Ok(Response::new(PollTaskResponse {
                    run_id: work_item.run_id.to_string(),
                    workflow_type: work_item.workflow_type,
                    input: Some(payload),
                }));
            }

            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Ok(Response::new(PollTaskResponse {
                    run_id: String::new(),
                    workflow_type: String::new(),
                    input: Some(empty_payload()),
                }));
            }

            let mut rx = self.queue.subscribe(&namespace_id, &req.task_queue);

            let notification_result = tokio::time::timeout(remaining, rx.recv()).await;

            match notification_result {
                Ok(Ok(_)) => {
                    // Add jitter to prevent thundering herd, but handle zero config gracefully
                    if self.queue_settings.poll_jitter_ms > 0 {
                        let jitter_ms =
                            rand::rng().random_range(0..self.queue_settings.poll_jitter_ms);
                        tokio::time::sleep(Duration::from_millis(jitter_ms)).await;
                    }
                }
                Ok(Err(_)) => {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                Err(_) => {}
            }
        }
    }

    #[instrument(
        skip(self, request),
        fields(
            run_id = %request.get_ref().run_id,
            step_name = %request.get_ref().step_name,
        )
    )]
    async fn begin_step(
        &self,
        request: Request<BeginStepRequest>,
    ) -> Result<Response<BeginStepResponse>, Status> {
        // Extract parent trace context and set it as the parent of current span
        let parent_cx = extract_context(request.metadata());
        let _ = tracing::Span::current().set_parent(parent_cx);

        let req = request.into_inner();

        let run_id =
            Uuid::parse_str(&req.run_id).map_err(|_| invalid_argument_error("Invalid run_id"))?;

        let _ = self.validate_workflow_action(run_id, false).await?;

        let workflow_retry_policy = self
            .store
            .workflows()
            .get_retry_policy(run_id)
            .await
            .map_err(map_store_error)?;

        let input = payload_to_optional_bytes(req.input);
        let step_kind = map_proto_step_kind(req.kind)?;

        let result = self
            .store
            .steps()
            .begin(BeginStepParams {
                run_id,
                step_id: req.step_name.clone(),
                step_kind,
                input,
                retry_policy: merge_proto_policy(req.retry_policy, workflow_retry_policy.as_ref()),
            })
            .await
            .map_err(map_store_error)?;

        if step_kind == kagzi_store::StepKind::Sleep && !result.should_execute {
            self.store
                .steps()
                .complete(run_id, &req.step_name, vec![])
                .await
                .map_err(map_store_error)?;
        }

        let cached_output = bytes_to_payload(result.cached_output);

        Ok(Response::new(BeginStepResponse {
            step_id: req.step_name,
            should_execute: result.should_execute,
            cached_output: Some(cached_output),
        }))
    }

    #[instrument(
        skip(self, request),
        fields(
            run_id = %request.get_ref().run_id,
            step_id = %request.get_ref().step_id,
        )
    )]
    async fn complete_step(
        &self,
        request: Request<CompleteStepRequest>,
    ) -> Result<Response<CompleteStepResponse>, Status> {
        let parent_cx = extract_context(request.metadata());
        let _ = tracing::Span::current().set_parent(parent_cx);

        let req = request.into_inner();

        let run_id =
            Uuid::parse_str(&req.run_id).map_err(|_| invalid_argument_error("Invalid run_id"))?;

        let namespace_id = self.validate_workflow_action(run_id, false).await?;

        let output = payload_to_optional_bytes(req.output).unwrap_or_default();

        self.store
            .steps()
            .complete(run_id, &req.step_id, output)
            .await
            .map_err(map_store_error)?;

        // Fetch latest step state to return
        let steps_result = self
            .store
            .steps()
            .list(kagzi_store::ListStepsParams {
                run_id,
                namespace_id,
                step_id: Some(req.step_id.clone()),
                page_size: 1,
                cursor: None,
            })
            .await
            .map_err(map_store_error)?;

        let step = steps_result
            .items
            .into_iter()
            .last()
            .map(step_to_proto)
            .transpose()?
            .ok_or_else(|| not_found_error("Step not found", "step", req.step_id.clone()))?;

        Ok(Response::new(CompleteStepResponse { step: Some(step) }))
    }

    #[instrument(
        skip(self, request),
        fields(
            run_id = %request.get_ref().run_id,
            step_id = %request.get_ref().step_id,
        )
    )]
    async fn fail_step(
        &self,
        request: Request<FailStepRequest>,
    ) -> Result<Response<FailStepResponse>, Status> {
        let parent_cx = extract_context(request.metadata());
        let _ = tracing::Span::current().set_parent(parent_cx);

        let req = request.into_inner();

        let run_id =
            Uuid::parse_str(&req.run_id).map_err(|_| invalid_argument_error("Invalid run_id"))?;

        let step_id = require_non_empty(req.step_id, "step_id")?;

        let error_detail = req.error.unwrap_or_else(|| ErrorDetail {
            code: ErrorCode::Unspecified as i32,
            ..Default::default()
        });

        let result = self
            .store
            .steps()
            .fail(FailStepParams {
                run_id,
                step_id: step_id.clone(),
                error: error_detail.message.clone(),
                non_retryable: error_detail.non_retryable,
                retry_after_ms: if error_detail.retry_after_ms > 0 {
                    Some(error_detail.retry_after_ms)
                } else {
                    None
                },
            })
            .await
            .map_err(map_store_error)?;

        // If step failure schedules a workflow retry, reschedule the workflow
        if let Some(delay_ms) = result.schedule_workflow_retry_ms {
            self.store
                .workflows()
                .schedule_retry(run_id, delay_ms)
                .await
                .map_err(map_store_error)?;
        }

        Ok(Response::new(FailStepResponse {
            scheduled_retry: result.scheduled_retry,
            retry_at: None, // No longer used - workflow is rescheduled, not step
        }))
    }

    #[instrument(skip(self, request), fields(run_id = %request.get_ref().run_id))]
    async fn complete_workflow(
        &self,
        request: Request<CompleteWorkflowRequest>,
    ) -> Result<Response<CompleteWorkflowResponse>, Status> {
        let parent_cx = extract_context(request.metadata());
        let _ = tracing::Span::current().set_parent(parent_cx);

        let req = request.into_inner();

        let run_id =
            Uuid::parse_str(&req.run_id).map_err(|_| invalid_argument_error("Invalid run_id"))?;

        let _ = self.validate_workflow_action(run_id, false).await?;

        let output = payload_to_optional_bytes(req.output).unwrap_or_default();

        self.store
            .workflows()
            .complete(run_id, output)
            .await
            .map_err(map_store_error)?;

        info!(run_id = %run_id, "Workflow completed");

        Ok(Response::new(CompleteWorkflowResponse {
            status: kagzi_proto::kagzi::WorkflowStatus::Completed as i32,
        }))
    }

    #[instrument(skip(self, request), fields(run_id = %request.get_ref().run_id))]
    async fn fail_workflow(
        &self,
        request: Request<FailWorkflowRequest>,
    ) -> Result<Response<FailWorkflowResponse>, Status> {
        // Extract parent trace context and set it as the parent of current span
        let parent_cx = extract_context(request.metadata());
        let _ = tracing::Span::current().set_parent(parent_cx);

        let req = request.into_inner();

        let run_id =
            Uuid::parse_str(&req.run_id).map_err(|_| invalid_argument_error("Invalid run_id"))?;

        let _ = self.validate_workflow_action(run_id, false).await?;

        let error_detail = req.error.unwrap_or_else(|| ErrorDetail {
            code: ErrorCode::Unspecified as i32,
            ..Default::default()
        });

        self.store
            .workflows()
            .fail(run_id, &error_detail.message)
            .await
            .map_err(map_store_error)?;

        info!(
            run_id = %run_id,
            error = %error_detail.message,
            "Workflow failed"
        );

        Ok(Response::new(FailWorkflowResponse {
            status: kagzi_proto::kagzi::WorkflowStatus::Failed as i32,
        }))
    }

    #[instrument(skip(self, request), fields(run_id = %request.get_ref().run_id))]
    async fn sleep(&self, request: Request<SleepRequest>) -> Result<Response<()>, Status> {
        let parent_cx = extract_context(request.metadata());
        let _ = tracing::Span::current().set_parent(parent_cx);

        let req = request.into_inner();

        let run_id =
            Uuid::parse_str(&req.run_id).map_err(|_| invalid_argument_error("Invalid run_id"))?;

        let duration_proto = req
            .duration
            .ok_or_else(|| invalid_argument_error("duration is required"))?;
        let duration: Duration = duration_proto
            .try_into()
            .map_err(|_| invalid_argument_error("duration must be non-negative"))?;
        let duration_seconds = duration.as_secs();

        // Validate duration
        if duration_seconds == 0 {
            // Zero duration sleep is a no-op, return immediately
            return Ok(Response::new(()));
        }

        self.store
            .workflows()
            .schedule_sleep(run_id, duration_seconds)
            .await
            .map_err(map_store_error)?;

        Ok(Response::new(()))
    }
}

impl<Q: QueueNotifier> WorkerServiceImpl<Q> {
    async fn complete_pending_sleep_steps(&self, run_id: Uuid) -> Result<(), Status> {
        let _ = self
            .store
            .steps()
            .complete_pending_sleeps(run_id)
            .await
            .map_err(map_store_error)?;

        Ok(())
    }
}
