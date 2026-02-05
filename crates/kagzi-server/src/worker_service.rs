use std::pin::Pin;
use std::time::Duration;

use kagzi_proto::kagzi::worker_service_server::WorkerService;
use kagzi_proto::kagzi::{
    BeginStepRequest, BeginStepResponse, ClaimTaskRequest, ClaimTaskResponse, ClaimedTask,
    CompleteStepRequest, CompleteStepResponse, CompleteWorkflowRequest, CompleteWorkflowResponse,
    DeregisterRequest, DeregisterResponse, ErrorCode, ErrorDetail, FailStepRequest,
    FailStepResponse, FailWorkflowRequest, FailWorkflowResponse, HeartbeatRequest,
    HeartbeatResponse, NoTask, RegisterRequest, RegisterResponse, SleepRequest, SleepResponse,
    SubscribeWorkRequest, WorkAvailable as ProtoWorkAvailable,
};
use kagzi_queue::WorkSignalBus;
use kagzi_store::repository::NamespaceRepository;
use kagzi_store::{
    BeginStepParams, FailStepParams, PgStore, RegisterWorkerParams, StepRepository,
    WorkerHeartbeatParams, WorkerRepository, WorkerStatus as StoreWorkerStatus, WorkflowRepository,
    WorkflowTypeConcurrency,
};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};
use tracing::{info, instrument, warn};
use tracing_opentelemetry::OpenTelemetrySpanExt;
use uuid::Uuid;

use crate::config::WorkerSettings;
use crate::helpers::{
    bytes_to_payload, invalid_argument_error, map_store_error, merge_proto_policy, not_found_error,
    payload_to_optional_bytes, precondition_failed_error, require_non_empty, resolve_task_queue,
};
use crate::proto_convert::{map_proto_step_kind, step_to_proto};
use crate::queue_store::ensure_task_queue_exists;
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
pub struct WorkerServiceImpl<Q: WorkSignalBus = kagzi_queue::PostgresNotifier> {
    pub store: PgStore,
    pub worker_settings: WorkerSettings,
    pub queue_settings: crate::config::QueueSettings,
    pub queue: Q,
    pub subscribe_work_enabled: bool,
}

impl<Q: WorkSignalBus> WorkerServiceImpl<Q> {
    pub fn new(
        store: PgStore,
        worker_settings: WorkerSettings,
        queue_settings: crate::config::QueueSettings,
        queue: Q,
        subscribe_work_enabled: bool,
    ) -> Self {
        Self {
            store,
            worker_settings,
            queue_settings,
            queue,
            subscribe_work_enabled,
        }
    }

    async fn validate_workflow_action(
        &self,
        run_id: Uuid,
        allow_terminal: bool,
    ) -> Result<String, Status> {
        let namespace = self
            .store
            .workflows()
            .get_namespace(run_id)
            .await
            .map_err(map_store_error)?
            .ok_or_else(|| not_found_error("Workflow not found", "workflow", run_id.to_string()))?;

        let workflow_check = self
            .store
            .workflows()
            .check_status(run_id, &namespace)
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

        Ok(namespace)
    }
}

#[tonic::async_trait]
impl<Q: WorkSignalBus + 'static> WorkerService for WorkerServiceImpl<Q> {
    type SubscribeWorkStream =
        Pin<Box<dyn tokio_stream::Stream<Item = Result<ProtoWorkAvailable, Status>> + Send>>;

    #[instrument(skip(self, request), fields(task_queue = ?request.get_ref().task_queue))]
    async fn register(
        &self,
        request: Request<RegisterRequest>,
    ) -> Result<Response<RegisterResponse>, Status> {
        let req = request.into_inner();

        if req.workflow_types.is_empty() {
            return Err(invalid_argument_error("workflow_types cannot be empty"));
        }

        let namespace = require_non_empty(req.namespace, "namespace")?;
        let task_queue = resolve_task_queue(req.task_queue);
        let workflow_types = req.workflow_types;

        // Clone for logging after worker_id is assigned
        let namespace_for_log = namespace.clone();
        let workflows_for_log = workflow_types.clone();

        // Ensure namespace exists (auto-create if it doesn't)
        self.store
            .namespaces()
            .get_or_create(&namespace)
            .await
            .map_err(map_store_error)?;

        let _ = ensure_task_queue_exists(&self.store, &namespace, &task_queue).await;

        let worker_id = self
            .store
            .workers()
            .register(RegisterWorkerParams {
                namespace,
                task_queue,
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
    ) -> Result<Response<DeregisterResponse>, Status> {
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

        let namespace = worker.namespace;

        if req.drain {
            self.store
                .workers()
                .start_drain(worker_id, &namespace)
                .await
                .map_err(map_store_error)?;
            info!(worker_id = %worker_id, namespace = %namespace, "Worker draining");
        } else {
            self.store
                .workers()
                .deregister(worker_id, &namespace)
                .await
                .map_err(map_store_error)?;
            info!(worker_id = %worker_id, namespace = %namespace, "Worker disconnected");
        }

        Ok(Response::new(DeregisterResponse { drained: req.drain }))
    }

    #[instrument(skip(self, request), fields(worker_id = %request.get_ref().worker_id, task_queue = %request.get_ref().task_queue))]
    async fn subscribe_work(
        &self,
        request: Request<SubscribeWorkRequest>,
    ) -> Result<Response<Self::SubscribeWorkStream>, Status> {
        if !self.subscribe_work_enabled {
            return Err(precondition_failed_error(
                "SubscribeWork is only supported with the Postgres work-signal backend. Use broker direct-subscribe and call ClaimTask on wakeup.",
            ));
        }

        let req = request.into_inner();

        let worker_id = Uuid::parse_str(&req.worker_id)
            .map_err(|_| invalid_argument_error("Invalid worker_id"))?;
        let namespace = require_non_empty(req.namespace, "namespace")?;
        let task_queue = require_non_empty(req.task_queue, "task_queue")?;

        let worker = self
            .store
            .workers()
            .find_by_id(worker_id)
            .await
            .map_err(map_store_error)?
            .ok_or_else(|| {
                precondition_failed_error("Worker not registered or offline. Call Register first.")
            })?;

        if worker.namespace != namespace || worker.task_queue != task_queue {
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

        if !req.workflow_types.is_empty() {
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
        }

        let mut rx = self.queue.subscribe(&namespace, &task_queue);
        let (tx, out_rx) = mpsc::channel::<Result<ProtoWorkAvailable, Status>>(64);

        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(evt) => {
                        if tx
                            .send(Ok(ProtoWorkAvailable {
                                namespace: evt.namespace,
                                task_queue: evt.task_queue,
                            }))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        // If lagged, still emit a wakeup; signal is lossy by design.
                        if tx
                            .send(Ok(ProtoWorkAvailable {
                                namespace: namespace.clone(),
                                task_queue: task_queue.clone(),
                            }))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });

        Ok(Response::new(
            Box::pin(ReceiverStream::new(out_rx)) as Self::SubscribeWorkStream
        ))
    }

    #[instrument(skip(self, request), fields(worker_id = %request.get_ref().worker_id, task_queue = %request.get_ref().task_queue))]
    async fn claim_task(
        &self,
        request: Request<ClaimTaskRequest>,
    ) -> Result<Response<ClaimTaskResponse>, Status> {
        let req = request.into_inner();

        let worker_id = Uuid::parse_str(&req.worker_id)
            .map_err(|_| invalid_argument_error("Invalid worker_id"))?;
        let namespace = require_non_empty(req.namespace, "namespace")?;
        let task_queue = require_non_empty(req.task_queue, "task_queue")?;

        let worker = self
            .store
            .workers()
            .find_by_id(worker_id)
            .await
            .map_err(map_store_error)?
            .ok_or_else(|| {
                precondition_failed_error("Worker not registered or offline. Call Register first.")
            })?;

        if worker.namespace != namespace || worker.task_queue != task_queue {
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

        // Server-authoritative workflow type filtering:
        // treat request workflow_types as a requested subset, then intersect with the worker's registered types.
        let effective_types: Vec<String> = if req.workflow_types.is_empty() {
            worker.workflow_types.clone()
        } else {
            worker
                .workflow_types
                .iter()
                .filter(|t| req.workflow_types.iter().any(|r| r == *t))
                .cloned()
                .collect()
        };

        if effective_types.is_empty() {
            return Err(precondition_failed_error(
                "Worker is not registered for the requested workflow types",
            ));
        }

        let work_item = self
            .store
            .workflows()
            .poll_workflow(
                &namespace,
                &task_queue,
                &req.worker_id,
                &effective_types,
                self.worker_settings.visibility_timeout_secs,
            )
            .await
            .map_err(map_store_error)?;

        let Some(work_item) = work_item else {
            // Best-effort: record server-derived claim outcome for UI.
            let _ = sqlx::query(
                r#"
                INSERT INTO kagzi.worker_telemetry_state (
                    worker_id, namespace, task_queue, updated_at,
                    last_claim_at, last_claim_result, last_claim_error
                )
                VALUES ($1, $2, $3, NOW(), NOW(), 'no_task', '')
                ON CONFLICT (worker_id) DO UPDATE SET
                    namespace = EXCLUDED.namespace,
                    task_queue = EXCLUDED.task_queue,
                    updated_at = NOW(),
                    last_claim_at = NOW(),
                    last_claim_result = 'no_task',
                    last_claim_error = ''
                "#,
            )
            .bind(worker_id)
            .bind(&namespace)
            .bind(&task_queue)
            .execute(self.store.pool())
            .await;

            return Ok(Response::new(ClaimTaskResponse {
                result: Some(kagzi_proto::kagzi::claim_task_response::Result::NoTask(
                    NoTask {},
                )),
            }));
        };

        // Best-effort: record server-derived claim outcome for UI.
        let _ = sqlx::query(
            r#"
            INSERT INTO kagzi.worker_telemetry_state (
                worker_id, namespace, task_queue, updated_at,
                last_claim_at, last_claim_result, last_claim_error
            )
            VALUES ($1, $2, $3, NOW(), NOW(), 'task', '')
            ON CONFLICT (worker_id) DO UPDATE SET
                namespace = EXCLUDED.namespace,
                task_queue = EXCLUDED.task_queue,
                updated_at = NOW(),
                last_claim_at = NOW(),
                last_claim_result = 'task',
                last_claim_error = ''
            "#,
        )
        .bind(worker_id)
        .bind(&namespace)
        .bind(&task_queue)
        .execute(self.store.pool())
        .await;

        let _ = self.complete_pending_sleep_steps(work_item.run_id).await;

        if let Err(err) = self
            .store
            .steps()
            .record_lifecycle_event(
                work_item.run_id,
                kagzi_store::StepKind::WorkflowStarted,
                None,
            )
            .await
        {
            warn!(
                run_id = %work_item.run_id,
                error = %err,
                "Failed to record WorkflowStarted lifecycle event"
            );
        }

        info!(
            run_id = %work_item.run_id,
            workflow_type = %work_item.workflow_type,
            worker_id = %req.worker_id,
            "Claimed workflow"
        );

        let payload = bytes_to_payload(Some(work_item.input));

        Ok(Response::new(ClaimTaskResponse {
            result: Some(kagzi_proto::kagzi::claim_task_response::Result::Task(
                ClaimedTask {
                    run_id: work_item.run_id.to_string(),
                    workflow_type: work_item.workflow_type,
                    input: Some(payload),
                },
            )),
        }))
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

        let namespace = self.validate_workflow_action(run_id, false).await?;

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
                namespace,
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

        let output = payload_to_optional_bytes(req.output.clone()).unwrap_or_default();

        self.store
            .workflows()
            .complete(run_id, output.clone())
            .await
            .map_err(map_store_error)?;

        if let Err(err) = self
            .store
            .steps()
            .record_lifecycle_event(
                run_id,
                kagzi_store::StepKind::WorkflowCompleted,
                Some(output),
            )
            .await
        {
            warn!(
                run_id = %run_id,
                error = %err,
                "Failed to record WorkflowCompleted lifecycle event"
            );
        }

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

        if let Err(err) = self
            .store
            .steps()
            .record_lifecycle_event(
                run_id,
                kagzi_store::StepKind::WorkflowFailed,
                Some(error_detail.message.clone().into_bytes()),
            )
            .await
        {
            warn!(
                run_id = %run_id,
                error = %err,
                "Failed to record WorkflowFailed lifecycle event"
            );
        }

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
    async fn sleep(
        &self,
        request: Request<SleepRequest>,
    ) -> Result<Response<SleepResponse>, Status> {
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
            return Ok(Response::new(SleepResponse {}));
        }

        self.store
            .workflows()
            .schedule_sleep(run_id, duration_seconds)
            .await
            .map_err(map_store_error)?;

        Ok(Response::new(SleepResponse {}))
    }
}

impl<Q: WorkSignalBus> WorkerServiceImpl<Q> {
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
