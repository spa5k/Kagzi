use crate::helpers::{
    invalid_argument, json_to_payload, map_store_error, merge_proto_policy, not_found,
    payload_to_optional_json, precondition_failed,
};
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
    HeartbeatResponse, Payload, PollTaskRequest, PollTaskResponse, RegisterRequest,
    RegisterResponse, SleepRequest, Step, StepKind, StepStatus,
};
use kagzi_store::{
    BeginStepParams, FailStepParams, PgStore, RegisterWorkerParams, StepRepository,
    WorkerHeartbeatParams, WorkerRepository, WorkerStatus as StoreWorkerStatus, WorkflowRepository,
    WorkflowTypeConcurrency,
};
use std::collections::HashMap;
use std::time::Duration;
use tonic::{Request, Response, Status};
use tracing::{debug, info, instrument};
use uuid::Uuid;

const MAX_QUEUE_CONCURRENCY: i32 = 10_000;
const MAX_TYPE_CONCURRENCY: i32 = 10_000;

fn map_step_status(status: kagzi_store::StepStatus) -> StepStatus {
    match status {
        kagzi_store::StepStatus::Pending => StepStatus::Pending,
        kagzi_store::StepStatus::Running => StepStatus::Running,
        kagzi_store::StepStatus::Completed => StepStatus::Completed,
        kagzi_store::StepStatus::Failed => StepStatus::Failed,
    }
}

fn step_kind_from_id(step_id: &str) -> StepKind {
    if step_id.contains("sleep") || step_id.contains("wait") {
        StepKind::Sleep
    } else if step_id.contains("function") || step_id.contains("task") {
        StepKind::Function
    } else {
        StepKind::Unspecified
    }
}

fn step_to_proto(s: kagzi_store::StepRun) -> Result<Step, Status> {
    let input = json_to_payload(s.input)?;
    let output = json_to_payload(s.output)?;
    let error = ErrorDetail {
        code: ErrorCode::Unspecified as i32,
        message: s.error.unwrap_or_default(),
        non_retryable: false,
        retry_after_ms: 0,
        subject: String::new(),
        subject_id: String::new(),
        metadata: HashMap::new(),
    };

    let step_id = s.step_id.clone();

    Ok(Step {
        step_id: step_id.clone(),
        run_id: s.run_id.to_string(),
        namespace_id: s.namespace_id,
        name: step_id.clone(),
        kind: step_kind_from_id(&step_id) as i32,
        status: map_step_status(s.status) as i32,
        attempt_number: s.attempt_number,
        input: Some(input),
        output: Some(output),
        error: Some(error),
        created_at: s.created_at.map(timestamp_from),
        started_at: s.started_at.map(timestamp_from),
        finished_at: s.finished_at.map(timestamp_from),
        child_run_id: s.child_workflow_run_id.map(|u| u.to_string()),
    })
}

fn timestamp_from(dt: chrono::DateTime<chrono::Utc>) -> prost_types::Timestamp {
    prost_types::Timestamp {
        seconds: dt.timestamp(),
        nanos: dt.timestamp_subsec_nanos() as i32,
    }
}

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
}

impl WorkerServiceImpl {
    pub fn new(store: PgStore) -> Self {
        let work_distributor = WorkDistributorHandle::new(store.clone());
        Self {
            store,
            work_distributor,
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
            .extend_locks_for_worker(&req.worker_id, 30)
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

        if req.drain {
            self.store
                .workers()
                .start_drain(worker_id)
                .await
                .map_err(map_store_error)?;
            info!(worker_id = %worker_id, "Worker draining");
        } else {
            self.store
                .workers()
                .deregister(worker_id)
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
            .claim_next_filtered(
                &req.task_queue,
                &namespace_id,
                &req.worker_id,
                &effective_types,
            )
            .await
            .map_err(map_store_error)?
        {
            let _ = self.store.workers().update_active_count(worker_id, 1).await;

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

        let timeout = std::env::var("KAGZI_POLL_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .map(Duration::from_secs)
            .unwrap_or_else(|| Duration::from_secs(60));

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
                let _ = self.store.workers().update_active_count(worker_id, 1).await;

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
                    input: Some(Payload {
                        data: Vec::new(),
                        metadata: HashMap::new(),
                    }),
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

        let workflow_retry = self
            .store
            .workflows()
            .get_retry_policy(run_id)
            .await
            .map_err(map_store_error)?;

        let input = payload_to_optional_json(req.input)?;

        let result = self
            .store
            .steps()
            .begin(BeginStepParams {
                run_id,
                step_id: req.step_name.clone(),
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

        let output_json = payload_to_optional_json(req.output)?
            .ok_or_else(|| invalid_argument("output payload cannot be null"))?;

        self.store
            .steps()
            .complete(run_id, &req.step_id, output_json)
            .await
            .map_err(map_store_error)?;

        // Fetch latest step state to return
        let steps = self
            .store
            .steps()
            .list_by_workflow(run_id, Some(&req.step_id), 1)
            .await
            .map_err(map_store_error)?;

        let step = steps
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
    async fn fail_step(&self, request: Request<FailStepRequest>) -> Result<Response<FailStepResponse>, Status> {
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

        let retry_at = result
            .retry_at
            .map(|dt| prost_types::Timestamp {
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

        let output_json = payload_to_optional_json(req.output)?
            .ok_or_else(|| invalid_argument("output payload cannot be null"))?;

        self.store
            .workflows()
            .complete(run_id, output_json)
            .await
            .map_err(map_store_error)?;

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

        let error_detail = req.error.unwrap_or_else(|| ErrorDetail {
            code: ErrorCode::Unspecified as i32,
            message: String::new(),
            non_retryable: false,
            retry_after_ms: 0,
            subject: String::new(),
            subject_id: String::new(),
            metadata: HashMap::new(),
        });

        self.store
            .workflows()
            .fail(run_id, &error_detail.message)
            .await
            .map_err(map_store_error)?;

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
        duration_seconds = %request.get_ref().duration_seconds
    ))]
    async fn sleep(&self, request: Request<SleepRequest>) -> Result<Response<()>, Status> {
        let correlation_id = extract_or_generate_correlation_id(&request);
        let trace_id = extract_or_generate_trace_id(&request);

        log_grpc_request("Sleep", &correlation_id, &trace_id, None);

        let req = request.into_inner();

        let run_id =
            Uuid::parse_str(&req.run_id).map_err(|_| invalid_argument("Invalid run_id"))?;

        self.store
            .workflows()
            .schedule_sleep(run_id, req.duration_seconds)
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

