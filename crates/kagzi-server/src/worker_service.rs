use std::collections::HashMap;
use std::convert::TryInto;
use std::time::Duration;

use chrono::Utc;
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
use uuid::Uuid;

use crate::config::WorkerSettings;
use crate::constants::DEFAULT_NAMESPACE;
use crate::helpers::{
    bytes_to_payload, invalid_argument_error, map_store_error, merge_proto_policy, not_found_error,
    payload_to_optional_bytes, precondition_failed_error,
};
use crate::proto_convert::{empty_payload, map_proto_step_kind, step_to_proto};

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
    const WORKFLOW_LOCK_DURATION_SECS: i64 = 30;

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
}

#[tonic::async_trait]
impl<Q: QueueNotifier + 'static> WorkerService for WorkerServiceImpl<Q> {
    async fn register(
        &self,
        request: Request<RegisterRequest>,
    ) -> Result<Response<RegisterResponse>, Status> {
        let req = request.into_inner();

        if req.workflow_types.is_empty() {
            return Err(invalid_argument_error("workflow_types cannot be empty"));
        }

        let namespace_id = if req.namespace_id.is_empty() {
            DEFAULT_NAMESPACE.to_string()
        } else {
            req.namespace_id
        };

        let worker_id = self
            .store
            .workers()
            .register(RegisterWorkerParams {
                namespace_id,
                task_queue: req.task_queue,
                workflow_types: req.workflow_types,
                hostname: if req.hostname.is_empty() {
                    None
                } else {
                    Some(req.hostname)
                },
                pid: if req.pid == 0 { None } else { Some(req.pid) },
                version: if req.version.is_empty() {
                    None
                } else {
                    Some(req.version)
                },
                max_concurrent: req.max_concurrent.max(1),
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

        Ok(Response::new(RegisterResponse {
            worker_id: worker_id.to_string(),
            heartbeat_interval_secs: self.worker_settings.heartbeat_interval_secs as i32,
        }))
    }

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
            .heartbeat(WorkerHeartbeatParams {
                worker_id,
                active_count: req.active_count,
                completed_delta: req.completed_delta,
                failed_delta: req.failed_delta,
            })
            .await
            .map_err(map_store_error)?;

        if !accepted {
            return Err(not_found_error(
                "Worker not found or offline",
                "worker",
                req.worker_id,
            ));
        }

        let extended = self
            .store
            .workflows()
            .extend_worker_locks(&req.worker_id, 30)
            .await
            .map_err(map_store_error)?;

        let _ = extended; // We don't need to log this

        let worker = self
            .store
            .workers()
            .find_by_id(worker_id)
            .await
            .map_err(map_store_error)?;

        let should_drain = worker
            .map(|w| w.status == StoreWorkerStatus::Draining)
            .unwrap_or(false);

        Ok(Response::new(HeartbeatResponse {
            accepted: true,
            should_drain,
        }))
    }

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
        } else {
            self.store
                .workers()
                .deregister(worker_id, &namespace_id)
                .await
                .map_err(map_store_error)?;
        }

        Ok(Response::new(()))
    }

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

        let namespace_id = if req.namespace_id.is_empty() {
            DEFAULT_NAMESPACE.to_string()
        } else {
            req.namespace_id
        };

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
                    Self::WORKFLOW_LOCK_DURATION_SECS,
                )
                .await
                .map_err(map_store_error)?;

            if let Some(work_item) = work_item {
                let _ = self.complete_pending_sleep_steps(work_item.run_id).await;

                let _ = self
                    .store
                    .workers()
                    .update_active_count(worker_id, &worker.namespace_id, 1)
                    .await;

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

    async fn begin_step(
        &self,
        request: Request<BeginStepRequest>,
    ) -> Result<Response<BeginStepResponse>, Status> {
        let req = request.into_inner();

        let run_id =
            Uuid::parse_str(&req.run_id).map_err(|_| invalid_argument_error("Invalid run_id"))?;

        let workflow = self
            .store
            .workflows()
            .find_by_id(run_id, DEFAULT_NAMESPACE)
            .await
            .map_err(map_store_error)?;
        let namespace_id = workflow
            .as_ref()
            .map(|w| w.namespace_id.clone())
            .unwrap_or_else(|| DEFAULT_NAMESPACE.to_string());

        // Validate workflow exists and is in a valid state for steps
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

        // Only allow steps on RUNNING workflows
        if let Some(status) = workflow_check.status
            && status.is_terminal()
        {
            return Err(precondition_failed_error(format!(
                "Cannot begin step on workflow with terminal status '{:?}'",
                status
            )));
        }

        let workflow_retry = self
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
                retry_policy: merge_proto_policy(req.retry_policy, workflow_retry.as_ref()),
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

    async fn complete_step(
        &self,
        request: Request<CompleteStepRequest>,
    ) -> Result<Response<CompleteStepResponse>, Status> {
        let req = request.into_inner();

        let run_id =
            Uuid::parse_str(&req.run_id).map_err(|_| invalid_argument_error("Invalid run_id"))?;

        let workflow = self
            .store
            .workflows()
            .find_by_id(run_id, DEFAULT_NAMESPACE)
            .await
            .map_err(map_store_error)?;
        let namespace_id = workflow
            .map(|w| w.namespace_id)
            .unwrap_or_else(|| DEFAULT_NAMESPACE.to_string());

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

    async fn fail_step(
        &self,
        request: Request<FailStepRequest>,
    ) -> Result<Response<FailStepResponse>, Status> {
        let req = request.into_inner();

        let run_id =
            Uuid::parse_str(&req.run_id).map_err(|_| invalid_argument_error("Invalid run_id"))?;

        if req.step_id.is_empty() {
            return Err(invalid_argument_error("step_id is required"));
        }

        let error_detail = req.error.unwrap_or_else(|| ErrorDetail {
            code: ErrorCode::Unspecified as i32,
            message: String::new(),
            non_retryable: false,
            retry_after_ms: 0,
            subject: String::new(),
            subject_id: String::new(),
            metadata: HashMap::new(),
        });

        let result = self
            .store
            .steps()
            .fail(FailStepParams {
                run_id,
                step_id: req.step_id.clone(),
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

        let retry_at_dt = result.retry_at;
        let retry_at = retry_at_dt.map(|dt| prost_types::Timestamp {
            seconds: dt.timestamp(),
            nanos: dt.timestamp_subsec_nanos() as i32,
        });

        if result.scheduled_retry {
            let delay_ms = retry_at_dt
                .map(|dt| (dt - Utc::now()).num_milliseconds().max(0) as u64)
                .unwrap_or(0);

            self.store
                .workflows()
                .schedule_retry(run_id, delay_ms)
                .await
                .map_err(map_store_error)?;
        }

        Ok(Response::new(FailStepResponse {
            scheduled_retry: result.scheduled_retry,
            retry_at,
        }))
    }

    async fn complete_workflow(
        &self,
        request: Request<CompleteWorkflowRequest>,
    ) -> Result<Response<CompleteWorkflowResponse>, Status> {
        let req = request.into_inner();

        let run_id =
            Uuid::parse_str(&req.run_id).map_err(|_| invalid_argument_error("Invalid run_id"))?;

        let workflow = self
            .store
            .workflows()
            .find_by_id(run_id, DEFAULT_NAMESPACE)
            .await
            .map_err(map_store_error)?;
        let namespace_id = workflow
            .as_ref()
            .map(|w| w.namespace_id.clone())
            .unwrap_or_else(|| DEFAULT_NAMESPACE.to_string());

        // Verify workflow exists and is in a completable state
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

        if let Some(status) = workflow_check.status
            && status.is_terminal()
        {
            return Err(precondition_failed_error(format!(
                "Cannot complete workflow with terminal status '{:?}'",
                status
            )));
        }

        let output = payload_to_optional_bytes(req.output).unwrap_or_default();

        self.store
            .workflows()
            .complete(run_id, output)
            .await
            .map_err(map_store_error)?;

        if let Some(locked_by) = workflow_check.locked_by
            && let Ok(worker_uuid) = Uuid::parse_str(&locked_by)
        {
            let _ = self
                .store
                .workers()
                .update_active_count(worker_uuid, &namespace_id, -1)
                .await;
        }

        Ok(Response::new(CompleteWorkflowResponse {
            status: kagzi_proto::kagzi::WorkflowStatus::Completed as i32,
        }))
    }

    async fn fail_workflow(
        &self,
        request: Request<FailWorkflowRequest>,
    ) -> Result<Response<FailWorkflowResponse>, Status> {
        let req = request.into_inner();

        let run_id =
            Uuid::parse_str(&req.run_id).map_err(|_| invalid_argument_error("Invalid run_id"))?;

        let workflow = self
            .store
            .workflows()
            .find_by_id(run_id, DEFAULT_NAMESPACE)
            .await
            .map_err(map_store_error)?;
        let namespace_id = workflow
            .as_ref()
            .map(|w| w.namespace_id.clone())
            .unwrap_or_else(|| DEFAULT_NAMESPACE.to_string());

        let error_detail = req.error.unwrap_or_else(|| ErrorDetail {
            code: ErrorCode::Unspecified as i32,
            message: String::new(),
            non_retryable: false,
            retry_after_ms: 0,
            subject: String::new(),
            subject_id: String::new(),
            metadata: HashMap::new(),
        });

        let workflow_status = self
            .store
            .workflows()
            .check_status(run_id, &namespace_id)
            .await
            .map_err(map_store_error)?;

        self.store
            .workflows()
            .fail(run_id, &error_detail.message)
            .await
            .map_err(map_store_error)?;

        // Decrement active count for the worker that held the lock, if any.
        if let Some(locked_by) = workflow_status.locked_by
            && let Ok(worker_uuid) = Uuid::parse_str(&locked_by)
        {
            let _ = self
                .store
                .workers()
                .update_active_count(worker_uuid, &namespace_id, -1)
                .await;
        }

        Ok(Response::new(FailWorkflowResponse {
            status: kagzi_proto::kagzi::WorkflowStatus::Failed as i32,
        }))
    }

    async fn sleep(&self, request: Request<SleepRequest>) -> Result<Response<()>, Status> {
        const MAX_SLEEP_SECONDS: u64 = 30 * 24 * 60 * 60; // 30 days

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

        if duration_seconds > MAX_SLEEP_SECONDS {
            return Err(invalid_argument_error(format!(
                "Sleep duration cannot exceed {} seconds (30 days)",
                MAX_SLEEP_SECONDS
            )));
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
        let steps = self
            .store
            .steps()
            .list(kagzi_store::ListStepsParams {
                run_id,
                namespace_id: DEFAULT_NAMESPACE.to_string(),
                step_id: None,
                page_size: 100,
                cursor: None,
            })
            .await
            .map_err(map_store_error)?;

        for step in steps.items {
            if step.step_kind == kagzi_store::StepKind::Sleep
                && step.status == kagzi_store::StepStatus::Running
            {
                self.store
                    .steps()
                    .complete(run_id, &step.step_id, vec![])
                    .await
                    .map_err(map_store_error)?;
            }
        }

        Ok(())
    }
}
