use crate::config::WorkerSettings;
use crate::helpers::{
    invalid_argument, json_to_payload, map_store_error, merge_proto_policy, not_found,
    payload_to_optional_json, precondition_failed,
};
use crate::proto_convert::{empty_payload, map_proto_step_kind, step_to_proto};
use crate::tracing_utils::{
    extract_or_generate_correlation_id, extract_or_generate_trace_id, log_grpc_request,
    log_grpc_response,
};
use crate::work_distributor::WorkDistributorHandle;
use kagzi_proto::kagzi::worker_service_server::WorkerService;
use kagzi_proto::kagzi::{
    BeginStepRequest, BeginStepResponse, CompleteStepRequest, CompleteStepResponse,
    CompleteWorkflowRequest, CompleteWorkflowResponse, DeregisterRequest, ErrorCode, ErrorDetail,
    FailStepRequest, FailStepResponse, FailWorkflowRequest, FailWorkflowResponse, HeartbeatRequest,
    HeartbeatResponse, PollTaskRequest, PollTaskResponse, RegisterRequest, RegisterResponse,
    SleepRequest,
};
use kagzi_store::{
    BeginStepParams, FailStepParams, PgStore, RegisterWorkerParams, StepRepository,
    WorkerHeartbeatParams, WorkerRepository, WorkerStatus as StoreWorkerStatus, WorkflowRepository,
    WorkflowTypeConcurrency,
};
use std::collections::HashMap;
use std::convert::TryInto;
use std::time::Duration;
use tonic::{Request, Response, Status};
use tracing::{debug, info, instrument, warn};
use uuid::Uuid;

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
pub struct WorkerServiceImpl {
    pub store: PgStore,
    pub work_distributor: WorkDistributorHandle,
    pub worker_settings: WorkerSettings,
}

impl WorkerServiceImpl {
    const WORKFLOW_LOCK_DURATION_SECS: i64 = 30;
    pub fn new(store: PgStore, worker_settings: WorkerSettings) -> Self {
        let work_distributor = WorkDistributorHandle::new(store.clone());
        Self {
            store,
            work_distributor,
            worker_settings,
        }
    }
}

#[tonic::async_trait]
impl WorkerService for WorkerServiceImpl {
    #[instrument(skip(self), fields(
        correlation_id = %extract_or_generate_correlation_id(&request),
        trace_id = %extract_or_generate_trace_id(&request),
        task_queue = %request.get_ref().task_queue,
        workflow_types = ?request.get_ref().workflow_types
    ))]
    async fn register(
        &self,
        request: Request<RegisterRequest>,
    ) -> Result<Response<RegisterResponse>, Status> {
        let req = request.into_inner();

        if req.workflow_types.is_empty() {
            return Err(invalid_argument("workflow_types cannot be empty"));
        }

        let namespace_id = if req.namespace_id.is_empty() {
            "default".to_string()
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

        info!(worker_id = %worker_id, "Worker registered");

        Ok(Response::new(RegisterResponse {
            worker_id: worker_id.to_string(),
            heartbeat_interval_secs: 10,
        }))
    }

    #[instrument(skip(self), fields(
        correlation_id = %extract_or_generate_correlation_id(&request),
        trace_id = %extract_or_generate_trace_id(&request),
        worker_id = %request.get_ref().worker_id
    ))]
    async fn heartbeat(
        &self,
        request: Request<HeartbeatRequest>,
    ) -> Result<Response<HeartbeatResponse>, Status> {
        let req = request.into_inner();
        let worker_id =
            Uuid::parse_str(&req.worker_id).map_err(|_| invalid_argument("Invalid worker_id"))?;

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
            return Err(not_found(
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

        if extended > 0 {
            debug!(worker_id = %worker_id, extended = extended, "Extended workflow locks");
        }

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

    #[instrument(skip(self), fields(
        correlation_id = %extract_or_generate_correlation_id(&request),
        trace_id = %extract_or_generate_trace_id(&request),
        worker_id = %request.get_ref().worker_id
    ))]
    async fn deregister(
        &self,
        request: Request<DeregisterRequest>,
    ) -> Result<Response<()>, Status> {
        let req = request.into_inner();
        let worker_id =
            Uuid::parse_str(&req.worker_id).map_err(|_| invalid_argument("Invalid worker_id"))?;

        let worker = self
            .store
            .workers()
            .find_by_id(worker_id)
            .await
            .map_err(map_store_error)?
            .ok_or_else(|| not_found("Worker not found", "worker", req.worker_id.clone()))?;

        let namespace_id = worker.namespace_id;

        if req.drain {
            self.store
                .workers()
                .start_drain(worker_id, &namespace_id)
                .await
                .map_err(map_store_error)?;
            info!(worker_id = %worker_id, "Worker draining");
        } else {
            self.store
                .workers()
                .deregister(worker_id, &namespace_id)
                .await
                .map_err(map_store_error)?;
            info!(worker_id = %worker_id, "Worker deregistered");
        }

        Ok(Response::new(()))
    }

    #[instrument(skip(self), fields(
        correlation_id = %extract_or_generate_correlation_id(&request),
        trace_id = %extract_or_generate_trace_id(&request),
        task_queue = %request.get_ref().task_queue,
        worker_id = %request.get_ref().worker_id
    ))]
    async fn poll_task(
        &self,
        request: Request<PollTaskRequest>,
    ) -> Result<Response<PollTaskResponse>, Status> {
        let correlation_id = extract_or_generate_correlation_id(&request);
        let trace_id = extract_or_generate_trace_id(&request);

        log_grpc_request("PollTask", &correlation_id, &trace_id, None);

        let req = request.into_inner();

        let worker_id =
            Uuid::parse_str(&req.worker_id).map_err(|_| invalid_argument("Invalid worker_id"))?;

        if req.workflow_types.is_empty() {
            return Err(invalid_argument("workflow_types cannot be empty"));
        }

        let namespace_id = if req.namespace_id.is_empty() {
            "default".to_string()
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
                precondition_failed("Worker not registered or offline. Call Register first.")
            })?;

        if worker.namespace_id != namespace_id || worker.task_queue != req.task_queue {
            return Err(precondition_failed(
                "Worker not registered for the requested namespace/task_queue",
            ));
        }

        if worker.status == StoreWorkerStatus::Offline {
            return Err(precondition_failed(
                "Worker not registered or offline. Call Register first.",
            ));
        }

        if worker.status == StoreWorkerStatus::Draining {
            return Err(precondition_failed(
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
            return Err(precondition_failed(
                "Worker is not registered for the requested workflow types",
            ));
        }

        if let Some(work_item) = self
            .store
            .workflows()
            .claim_next_workflow(
                &req.task_queue,
                &namespace_id,
                &req.worker_id,
                &effective_types,
                Self::WORKFLOW_LOCK_DURATION_SECS,
            )
            .await
            .map_err(map_store_error)?
        {
            if let Err(e) = self
                .store
                .workers()
                .update_active_count(worker_id, &worker.namespace_id, 1)
                .await
            {
                tracing::warn!(worker_id = %worker_id, error = ?e, "Failed to update active count");
            }

            let payload = json_to_payload(Some(work_item.input))?;

            log_grpc_response(
                "PollTask",
                &correlation_id,
                &trace_id,
                Status::code(&Status::ok("")),
                None,
            );

            return Ok(Response::new(PollTaskResponse {
                run_id: work_item.run_id.to_string(),
                workflow_type: work_item.workflow_type,
                input: Some(payload),
            }));
        }

        let timeout = Duration::from_secs(self.worker_settings.poll_timeout_secs);

        match self
            .work_distributor
            .wait_for_work(
                &req.task_queue,
                &namespace_id,
                &req.worker_id,
                &effective_types,
                timeout,
            )
            .await
        {
            Some(work_item) => {
                if let Err(e) = self
                    .store
                    .workers()
                    .update_active_count(worker_id, &worker.namespace_id, 1)
                    .await
                {
                    tracing::warn!(
                        worker_id = %worker_id,
                        error = ?e,
                        "Failed to update active count"
                    );
                }

                let payload = json_to_payload(Some(work_item.input))?;

                info!(
                    correlation_id = correlation_id,
                    trace_id = trace_id,
                    run_id = %work_item.run_id,
                    workflow_type = %work_item.workflow_type,
                    worker_id = %req.worker_id,
                    "Worker claimed workflow via distributor"
                );

                log_grpc_response(
                    "PollTask",
                    &correlation_id,
                    &trace_id,
                    Status::code(&Status::ok("")),
                    None,
                );

                Ok(Response::new(PollTaskResponse {
                    run_id: work_item.run_id.to_string(),
                    workflow_type: work_item.workflow_type,
                    input: Some(payload),
                }))
            }
            None => {
                log_grpc_response(
                    "PollTask",
                    &correlation_id,
                    &trace_id,
                    Status::code(&Status::ok("")),
                    Some("No work available - timeout"),
                );

                Ok(Response::new(PollTaskResponse {
                    run_id: String::new(),
                    workflow_type: String::new(),
                    input: Some(empty_payload()),
                }))
            }
        }
    }

    #[instrument(skip(self), fields(
        correlation_id = %extract_or_generate_correlation_id(&request),
        trace_id = %extract_or_generate_trace_id(&request),
        run_id = %request.get_ref().run_id,
        step_name = %request.get_ref().step_name
    ))]
    async fn begin_step(
        &self,
        request: Request<BeginStepRequest>,
    ) -> Result<Response<BeginStepResponse>, Status> {
        let correlation_id = extract_or_generate_correlation_id(&request);
        let trace_id = extract_or_generate_trace_id(&request);

        log_grpc_request("BeginStep", &correlation_id, &trace_id, None);

        let req = request.into_inner();

        let run_id =
            Uuid::parse_str(&req.run_id).map_err(|_| invalid_argument("Invalid run_id"))?;

        let workflow = self
            .store
            .workflows()
            .find_by_id(run_id, "default")
            .await
            .map_err(map_store_error)?;
        let namespace_id = workflow
            .as_ref()
            .map(|w| w.namespace_id.clone())
            .unwrap_or_else(|| "default".to_string());

        // Validate workflow exists and is in a valid state for steps
        let workflow_check = self
            .store
            .workflows()
            .check_status(run_id, &namespace_id)
            .await
            .map_err(map_store_error)?;

        if !workflow_check.exists {
            return Err(not_found(
                format!("Workflow not found: run_id={}", run_id),
                "workflow",
                run_id.to_string(),
            ));
        }

        // Only allow steps on RUNNING workflows
        if let Some(status) = workflow_check.status
            && status.is_terminal()
        {
            return Err(precondition_failed(format!(
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

        let input = payload_to_optional_json(req.input)?;
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

        let cached_output = json_to_payload(result.cached_output)?;

        log_grpc_response(
            "BeginStep",
            &correlation_id,
            &trace_id,
            Status::code(&Status::ok("")),
            None,
        );

        Ok(Response::new(BeginStepResponse {
            step_id: req.step_name,
            should_execute: result.should_execute,
            cached_output: Some(cached_output),
        }))
    }

    #[instrument(skip(self), fields(
        correlation_id = %extract_or_generate_correlation_id(&request),
        trace_id = %extract_or_generate_trace_id(&request),
        run_id = %request.get_ref().run_id,
        step_id = %request.get_ref().step_id
    ))]
    async fn complete_step(
        &self,
        request: Request<CompleteStepRequest>,
    ) -> Result<Response<CompleteStepResponse>, Status> {
        let correlation_id = extract_or_generate_correlation_id(&request);
        let trace_id = extract_or_generate_trace_id(&request);

        log_grpc_request("CompleteStep", &correlation_id, &trace_id, None);

        let req = request.into_inner();

        let run_id =
            Uuid::parse_str(&req.run_id).map_err(|_| invalid_argument("Invalid run_id"))?;

        let workflow = self
            .store
            .workflows()
            .find_by_id(run_id, "default")
            .await
            .map_err(map_store_error)?;
        let namespace_id = workflow
            .map(|w| w.namespace_id)
            .unwrap_or_else(|| "default".to_string());

        let output_json = payload_to_optional_json(req.output)?.unwrap_or(serde_json::Value::Null);

        self.store
            .steps()
            .complete(run_id, &req.step_id, output_json)
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
            .ok_or_else(|| not_found("Step not found", "step", req.step_id.clone()))?;

        log_grpc_response(
            "CompleteStep",
            &correlation_id,
            &trace_id,
            Status::code(&Status::ok("")),
            None,
        );

        Ok(Response::new(CompleteStepResponse { step: Some(step) }))
    }

    #[instrument(skip(self), fields(
        correlation_id = %extract_or_generate_correlation_id(&request),
        trace_id = %extract_or_generate_trace_id(&request),
        run_id = %request.get_ref().run_id,
        step_id = %request.get_ref().step_id
    ))]
    async fn fail_step(
        &self,
        request: Request<FailStepRequest>,
    ) -> Result<Response<FailStepResponse>, Status> {
        let correlation_id = extract_or_generate_correlation_id(&request);
        let trace_id = extract_or_generate_trace_id(&request);

        log_grpc_request("FailStep", &correlation_id, &trace_id, None);

        let req = request.into_inner();

        let run_id =
            Uuid::parse_str(&req.run_id).map_err(|_| invalid_argument("Invalid run_id"))?;

        if req.step_id.is_empty() {
            return Err(invalid_argument("step_id is required"));
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

        let retry_at = result.retry_at.map(|dt| prost_types::Timestamp {
            seconds: dt.timestamp(),
            nanos: dt.timestamp_subsec_nanos() as i32,
        });

        log_grpc_response(
            "FailStep",
            &correlation_id,
            &trace_id,
            Status::code(&Status::ok("")),
            None,
        );

        Ok(Response::new(FailStepResponse {
            scheduled_retry: result.scheduled_retry,
            retry_at,
        }))
    }

    #[instrument(skip(self), fields(
        correlation_id = %extract_or_generate_correlation_id(&request),
        trace_id = %extract_or_generate_trace_id(&request),
        run_id = %request.get_ref().run_id
    ))]
    async fn complete_workflow(
        &self,
        request: Request<CompleteWorkflowRequest>,
    ) -> Result<Response<CompleteWorkflowResponse>, Status> {
        let correlation_id = extract_or_generate_correlation_id(&request);
        let trace_id = extract_or_generate_trace_id(&request);

        log_grpc_request("CompleteWorkflow", &correlation_id, &trace_id, None);

        let req = request.into_inner();

        let run_id =
            Uuid::parse_str(&req.run_id).map_err(|_| invalid_argument("Invalid run_id"))?;

        let workflow = self
            .store
            .workflows()
            .find_by_id(run_id, "default")
            .await
            .map_err(map_store_error)?;
        let namespace_id = workflow
            .as_ref()
            .map(|w| w.namespace_id.clone())
            .unwrap_or_else(|| "default".to_string());

        // Verify workflow exists and is in a completable state
        let workflow_check = self
            .store
            .workflows()
            .check_status(run_id, &namespace_id)
            .await
            .map_err(map_store_error)?;

        if !workflow_check.exists {
            return Err(not_found(
                format!("Workflow not found: run_id={}", run_id),
                "workflow",
                run_id.to_string(),
            ));
        }

        if let Some(status) = workflow_check.status
            && status.is_terminal()
        {
            return Err(precondition_failed(format!(
                "Cannot complete workflow with terminal status '{:?}'",
                status
            )));
        }

        let output_json = payload_to_optional_json(req.output)?.unwrap_or(serde_json::Value::Null);

        self.store
            .workflows()
            .complete(run_id, output_json)
            .await
            .map_err(map_store_error)?;

        if let Some(locked_by) = workflow_check.locked_by {
            if let Ok(worker_uuid) = Uuid::parse_str(&locked_by) {
                if let Err(e) = self
                    .store
                    .workers()
                    .update_active_count(worker_uuid, &namespace_id, -1)
                    .await
                {
                    warn!(worker_id = %locked_by, error = ?e, "Failed to decrement active_count after completion");
                }
            } else {
                warn!(worker_id = %locked_by, "Invalid worker_id stored in locked_by; active_count not decremented");
            }
        }

        log_grpc_response(
            "CompleteWorkflow",
            &correlation_id,
            &trace_id,
            Status::code(&Status::ok("")),
            None,
        );

        Ok(Response::new(CompleteWorkflowResponse {
            status: kagzi_proto::kagzi::WorkflowStatus::Completed as i32,
        }))
    }

    #[instrument(skip(self), fields(
        correlation_id = %extract_or_generate_correlation_id(&request),
        trace_id = %extract_or_generate_trace_id(&request),
        run_id = %request.get_ref().run_id
    ))]
    async fn fail_workflow(
        &self,
        request: Request<FailWorkflowRequest>,
    ) -> Result<Response<FailWorkflowResponse>, Status> {
        let correlation_id = extract_or_generate_correlation_id(&request);
        let trace_id = extract_or_generate_trace_id(&request);

        log_grpc_request("FailWorkflow", &correlation_id, &trace_id, None);

        let req = request.into_inner();

        let run_id =
            Uuid::parse_str(&req.run_id).map_err(|_| invalid_argument("Invalid run_id"))?;

        let workflow = self
            .store
            .workflows()
            .find_by_id(run_id, "default")
            .await
            .map_err(map_store_error)?;
        let namespace_id = workflow
            .as_ref()
            .map(|w| w.namespace_id.clone())
            .unwrap_or_else(|| "default".to_string());

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
        if let Some(locked_by) = workflow_status.locked_by {
            if let Ok(worker_uuid) = Uuid::parse_str(&locked_by) {
                if let Err(e) = self
                    .store
                    .workers()
                    .update_active_count(worker_uuid, &namespace_id, -1)
                    .await
                {
                    warn!(worker_id = %locked_by, error = ?e, "Failed to decrement active_count after failure");
                }
            } else {
                warn!(worker_id = %locked_by, "Invalid worker_id stored in locked_by; active_count not decremented");
            }
        }

        log_grpc_response(
            "FailWorkflow",
            &correlation_id,
            &trace_id,
            Status::code(&Status::ok("")),
            None,
        );

        Ok(Response::new(FailWorkflowResponse {
            status: kagzi_proto::kagzi::WorkflowStatus::Failed as i32,
        }))
    }

    #[instrument(skip(self), fields(
        correlation_id = %extract_or_generate_correlation_id(&request),
        trace_id = %extract_or_generate_trace_id(&request),
        run_id = %request.get_ref().run_id,
        step_id = %request.get_ref().step_id,
        duration = ?request.get_ref().duration
    ))]
    async fn sleep(&self, request: Request<SleepRequest>) -> Result<Response<()>, Status> {
        const MAX_SLEEP_SECONDS: u64 = 30 * 24 * 60 * 60; // 30 days

        let correlation_id = extract_or_generate_correlation_id(&request);
        let trace_id = extract_or_generate_trace_id(&request);

        log_grpc_request("Sleep", &correlation_id, &trace_id, None);

        let req = request.into_inner();

        let run_id =
            Uuid::parse_str(&req.run_id).map_err(|_| invalid_argument("Invalid run_id"))?;

        let duration_proto = req
            .duration
            .ok_or_else(|| invalid_argument("duration is required"))?;
        let duration: Duration = duration_proto
            .try_into()
            .map_err(|_| invalid_argument("duration must be non-negative"))?;
        let duration_seconds = duration.as_secs();

        // Validate duration
        if duration_seconds == 0 {
            // Zero duration sleep is a no-op, return immediately
            log_grpc_response(
                "Sleep",
                &correlation_id,
                &trace_id,
                Status::code(&Status::ok("")),
                Some("Zero duration - no-op"),
            );
            return Ok(Response::new(()));
        }

        if duration_seconds > MAX_SLEEP_SECONDS {
            return Err(invalid_argument(format!(
                "Sleep duration cannot exceed {} seconds (30 days)",
                MAX_SLEEP_SECONDS
            )));
        }

        self.store
            .workflows()
            .schedule_sleep(run_id, duration_seconds)
            .await
            .map_err(map_store_error)?;

        log_grpc_response(
            "Sleep",
            &correlation_id,
            &trace_id,
            Status::code(&Status::ok("")),
            None,
        );

        Ok(Response::new(()))
    }
}
