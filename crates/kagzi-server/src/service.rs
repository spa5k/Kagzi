use crate::tracing_utils::{
    extract_or_generate_correlation_id, extract_or_generate_trace_id, log_grpc_request,
    log_grpc_response,
};
use crate::work_distributor::WorkDistributorHandle;
use kagzi_proto::kagzi::workflow_service_server::WorkflowService;
use kagzi_proto::kagzi::{
    BeginStepRequest, BeginStepResponse, CancelWorkflowRunRequest, CompleteStepRequest,
    CompleteWorkflowRequest, DeregisterWorkerRequest, Empty, FailStepRequest, FailWorkflowRequest,
    GetStepAttemptRequest, GetStepAttemptResponse, GetWorkerRequest, GetWorkerResponse,
    GetWorkflowRunRequest, GetWorkflowRunResponse, HealthCheckRequest, HealthCheckResponse,
    ListStepAttemptsRequest, ListStepAttemptsResponse, ListWorkersRequest, ListWorkersResponse,
    ListWorkflowRunsRequest, ListWorkflowRunsResponse, PollActivityRequest, PollActivityResponse,
    RegisterWorkerRequest, RegisterWorkerResponse, RetryPolicy, ScheduleSleepRequest,
    StartWorkflowRequest, StartWorkflowResponse, StepAttempt, StepAttemptStatus, StepKind, Worker,
    WorkerHeartbeatRequest, WorkerHeartbeatResponse, WorkflowRun, WorkflowStatus,
};
use kagzi_store::{
    BeginStepParams, CreateWorkflow, FailStepParams, ListWorkersParams, ListWorkflowsParams,
    PgStore, RegisterWorkerParams, StepRepository, WorkerHeartbeatParams, WorkerRepository,
    WorkerStatus, WorkflowCursor, WorkflowRepository,
};
use std::collections::HashMap;
use std::time::Duration;
use tonic::{Request, Response, Status};
use tracing::{debug, info, instrument};

/// Convert proto RetryPolicy to store RetryPolicy
fn proto_policy_to_store(p: Option<RetryPolicy>) -> Option<kagzi_store::RetryPolicy> {
    p.map(|proto| kagzi_store::RetryPolicy {
        maximum_attempts: if proto.maximum_attempts == 0 {
            5
        } else {
            proto.maximum_attempts
        },
        initial_interval_ms: if proto.initial_interval_ms == 0 {
            1000
        } else {
            proto.initial_interval_ms
        },
        backoff_coefficient: if proto.backoff_coefficient == 0.0 {
            2.0
        } else {
            proto.backoff_coefficient
        },
        maximum_interval_ms: if proto.maximum_interval_ms == 0 {
            60000
        } else {
            proto.maximum_interval_ms
        },
        non_retryable_errors: proto.non_retryable_errors,
    })
}

/// Map store workflow status to proto status
fn map_workflow_status(status: kagzi_store::WorkflowStatus) -> WorkflowStatus {
    match status {
        kagzi_store::WorkflowStatus::Pending => WorkflowStatus::Pending,
        kagzi_store::WorkflowStatus::Running => WorkflowStatus::Running,
        kagzi_store::WorkflowStatus::Sleeping => WorkflowStatus::Sleeping,
        kagzi_store::WorkflowStatus::Completed => WorkflowStatus::Completed,
        kagzi_store::WorkflowStatus::Failed => WorkflowStatus::Failed,
        kagzi_store::WorkflowStatus::Cancelled => WorkflowStatus::Cancelled,
    }
}

/// Map store step status to proto status
fn map_step_status(status: kagzi_store::StepStatus) -> StepAttemptStatus {
    match status {
        kagzi_store::StepStatus::Pending => StepAttemptStatus::Pending,
        kagzi_store::StepStatus::Running => StepAttemptStatus::Running,
        kagzi_store::StepStatus::Completed => StepAttemptStatus::Completed,
        kagzi_store::StepStatus::Failed => StepAttemptStatus::Failed,
    }
}

/// Convert store WorkflowRun to proto WorkflowRun
fn workflow_to_proto(w: kagzi_store::WorkflowRun) -> Result<WorkflowRun, Status> {
    let input_bytes = serde_json::to_vec(&w.input).map_err(|e| {
        tracing::error!("Failed to serialize workflow input: {:?}", e);
        Status::internal("Failed to serialize workflow input")
    })?;

    let output_bytes = w
        .output
        .map(|o| {
            serde_json::to_vec(&o).map_err(|e| {
                tracing::error!("Failed to serialize workflow output: {:?}", e);
                Status::internal("Failed to serialize workflow output")
            })
        })
        .transpose()?
        .unwrap_or_default();

    let context_bytes = w
        .context
        .map(|c| {
            serde_json::to_vec(&c).map_err(|e| {
                tracing::error!("Failed to serialize workflow context: {:?}", e);
                Status::internal("Failed to serialize workflow context")
            })
        })
        .transpose()?
        .unwrap_or_default();

    Ok(WorkflowRun {
        run_id: w.run_id.to_string(),
        business_id: w.business_id,
        task_queue: w.task_queue,
        workflow_type: w.workflow_type,
        status: map_workflow_status(w.status).into(),
        input: input_bytes,
        output: output_bytes,
        error: w.error.unwrap_or_default(),
        attempts: w.attempts,
        created_at: w.created_at.map(|t| prost_types::Timestamp {
            seconds: t.timestamp(),
            nanos: t.timestamp_subsec_nanos() as i32,
        }),
        started_at: w.started_at.map(|t| prost_types::Timestamp {
            seconds: t.timestamp(),
            nanos: t.timestamp_subsec_nanos() as i32,
        }),
        finished_at: w.finished_at.map(|t| prost_types::Timestamp {
            seconds: t.timestamp(),
            nanos: t.timestamp_subsec_nanos() as i32,
        }),
        wake_up_at: w.wake_up_at.map(|t| prost_types::Timestamp {
            seconds: t.timestamp(),
            nanos: t.timestamp_subsec_nanos() as i32,
        }),
        namespace_id: w.namespace_id,
        context: context_bytes,
        deadline_at: w.deadline_at.map(|t| prost_types::Timestamp {
            seconds: t.timestamp(),
            nanos: t.timestamp_subsec_nanos() as i32,
        }),
        worker_id: w.locked_by.unwrap_or_default(),
        version: w.version.unwrap_or_default(),
        parent_step_attempt_id: w.parent_step_attempt_id.unwrap_or_default(),
    })
}

/// Convert store StepRun to proto StepAttempt
fn step_to_proto(s: kagzi_store::StepRun) -> Result<StepAttempt, Status> {
    // Determine step kind from step_id patterns
    let kind = if s.step_id.contains("sleep") || s.step_id.contains("wait") {
        StepKind::Sleep
    } else if s.step_id.contains("function") || s.step_id.contains("task") {
        StepKind::Function
    } else {
        StepKind::Unspecified
    };

    let output_bytes = serde_json::to_vec(&s.output).map_err(|e| {
        tracing::error!("Failed to serialize step output: {:?}", e);
        Status::internal("Failed to serialize step output")
    })?;

    Ok(StepAttempt {
        step_attempt_id: s.attempt_id.to_string(),
        workflow_run_id: s.run_id.to_string(),
        step_id: s.step_id,
        kind: kind.into(),
        status: map_step_status(s.status).into(),
        config: vec![],
        context: vec![],
        output: output_bytes,
        error: s.error.map(|e| e.into_bytes()).unwrap_or_default(),
        started_at: s.started_at.map(|t| prost_types::Timestamp {
            seconds: t.timestamp(),
            nanos: t.timestamp_subsec_nanos() as i32,
        }),
        finished_at: s.finished_at.map(|t| prost_types::Timestamp {
            seconds: t.timestamp(),
            nanos: t.timestamp_subsec_nanos() as i32,
        }),
        created_at: s.created_at.map(|t| prost_types::Timestamp {
            seconds: t.timestamp(),
            nanos: t.timestamp_subsec_nanos() as i32,
        }),
        updated_at: s
            .finished_at
            .or(s.created_at)
            .map(|t| prost_types::Timestamp {
                seconds: t.timestamp(),
                nanos: t.timestamp_subsec_nanos() as i32,
            }),
        child_workflow_run_id: s
            .child_workflow_run_id
            .map(|u| u.to_string())
            .unwrap_or_default(),
        namespace_id: s.namespace_id,
    })
}

fn worker_to_proto(w: kagzi_store::Worker) -> Worker {
    let labels = match w.labels {
        serde_json::Value::Object(map) => map
            .into_iter()
            .filter_map(|(k, v)| v.as_str().map(|s| (k, s.to_string())))
            .collect(),
        _ => HashMap::new(),
    };

    Worker {
        worker_id: w.worker_id.to_string(),
        namespace_id: w.namespace_id,
        task_queue: w.task_queue,
        status: match w.status {
            WorkerStatus::Online => 1,
            WorkerStatus::Draining => 2,
            WorkerStatus::Offline => 3,
        },
        hostname: w.hostname.unwrap_or_default(),
        pid: w.pid.unwrap_or(0),
        version: w.version.unwrap_or_default(),
        workflow_types: w.workflow_types,
        max_concurrent: w.max_concurrent,
        active_count: w.active_count,
        total_completed: w.total_completed,
        total_failed: w.total_failed,
        registered_at: Some(prost_types::Timestamp {
            seconds: w.registered_at.timestamp(),
            nanos: w.registered_at.timestamp_subsec_nanos() as i32,
        }),
        last_heartbeat_at: Some(prost_types::Timestamp {
            seconds: w.last_heartbeat_at.timestamp(),
            nanos: w.last_heartbeat_at.timestamp_subsec_nanos() as i32,
        }),
        labels,
    }
}

/// Map StoreError to gRPC Status
fn map_store_error(e: kagzi_store::StoreError) -> Status {
    match e {
        kagzi_store::StoreError::NotFound { entity, id } => {
            Status::not_found(format!("{} not found: {}", entity, id))
        }
        kagzi_store::StoreError::InvalidState { message } => Status::invalid_argument(message),
        kagzi_store::StoreError::LockConflict { message } => Status::failed_precondition(message),
        kagzi_store::StoreError::PreconditionFailed { message } => {
            Status::failed_precondition(message)
        }
        kagzi_store::StoreError::Database(e) => {
            tracing::error!("Database error: {:?}", e);
            Status::internal("Database error")
        }
        kagzi_store::StoreError::Serialization(e) => {
            tracing::error!("Serialization error: {:?}", e);
            Status::internal("Serialization error")
        }
    }
}

pub struct MyWorkflowService {
    pub store: PgStore,
    pub work_distributor: WorkDistributorHandle,
}

impl MyWorkflowService {
    pub fn new(store: PgStore) -> Self {
        let work_distributor = WorkDistributorHandle::new(store.clone());
        Self {
            store,
            work_distributor,
        }
    }
}

#[tonic::async_trait]
impl WorkflowService for MyWorkflowService {
    async fn register_worker(
        &self,
        request: Request<RegisterWorkerRequest>,
    ) -> Result<Response<RegisterWorkerResponse>, Status> {
        let req = request.into_inner();

        if req.workflow_types.is_empty() {
            return Err(Status::invalid_argument("workflow_types cannot be empty"));
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
            })
            .await
            .map_err(map_store_error)?;

        info!(worker_id = %worker_id, "Worker registered");

        Ok(Response::new(RegisterWorkerResponse {
            worker_id: worker_id.to_string(),
            heartbeat_interval_secs: 10,
        }))
    }

    async fn worker_heartbeat(
        &self,
        request: Request<WorkerHeartbeatRequest>,
    ) -> Result<Response<WorkerHeartbeatResponse>, Status> {
        let req = request.into_inner();
        let worker_id = uuid::Uuid::parse_str(&req.worker_id)
            .map_err(|_| Status::invalid_argument("Invalid worker_id"))?;

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
            return Err(Status::not_found("Worker not found or offline"));
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
            .map(|w| w.status == WorkerStatus::Draining)
            .unwrap_or(false);

        Ok(Response::new(WorkerHeartbeatResponse {
            accepted: true,
            should_drain,
        }))
    }

    async fn deregister_worker(
        &self,
        request: Request<DeregisterWorkerRequest>,
    ) -> Result<Response<Empty>, Status> {
        let req = request.into_inner();
        let worker_id = uuid::Uuid::parse_str(&req.worker_id)
            .map_err(|_| Status::invalid_argument("Invalid worker_id"))?;

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

        Ok(Response::new(Empty {}))
    }

    async fn list_workers(
        &self,
        request: Request<ListWorkersRequest>,
    ) -> Result<Response<ListWorkersResponse>, Status> {
        let req = request.into_inner();

        let filter_status = match req.filter_status.as_str() {
            "ONLINE" => Some(WorkerStatus::Online),
            "DRAINING" => Some(WorkerStatus::Draining),
            "OFFLINE" => Some(WorkerStatus::Offline),
            _ => None,
        };

        let cursor = if req.page_token.is_empty() {
            None
        } else {
            Some(
                uuid::Uuid::parse_str(&req.page_token)
                    .map_err(|_| Status::invalid_argument("Invalid page_token"))?,
            )
        };

        let namespace_id = if req.namespace_id.is_empty() {
            "default".to_string()
        } else {
            req.namespace_id.clone()
        };

        let page_size = req.page_size.clamp(1, 100);

        let workers = self
            .store
            .workers()
            .list(ListWorkersParams {
                namespace_id: namespace_id.clone(),
                task_queue: if req.task_queue.is_empty() {
                    None
                } else {
                    Some(req.task_queue.clone())
                },
                filter_status,
                page_size,
                cursor,
            })
            .await
            .map_err(map_store_error)?;

        let total_count = self
            .store
            .workers()
            .count(
                &namespace_id,
                if req.task_queue.is_empty() {
                    None
                } else {
                    Some(req.task_queue.as_str())
                },
                filter_status,
            )
            .await
            .map_err(map_store_error)?;

        let mut workers = workers;
        let mut next_page_token = String::new();
        if workers.len() as i32 > page_size
            && let Some(last) = workers.pop()
        {
            next_page_token = last.worker_id.to_string();
        }

        let proto_workers = workers.into_iter().map(worker_to_proto).collect();

        Ok(Response::new(ListWorkersResponse {
            workers: proto_workers,
            next_page_token,
            total_count: total_count as i32,
        }))
    }

    async fn get_worker(
        &self,
        request: Request<GetWorkerRequest>,
    ) -> Result<Response<GetWorkerResponse>, Status> {
        let req = request.into_inner();
        let worker_id = uuid::Uuid::parse_str(&req.worker_id)
            .map_err(|_| Status::invalid_argument("Invalid worker_id"))?;

        let worker = self
            .store
            .workers()
            .find_by_id(worker_id)
            .await
            .map_err(map_store_error)?;

        match worker {
            Some(w) => Ok(Response::new(GetWorkerResponse {
                worker: Some(worker_to_proto(w)),
            })),
            None => Err(Status::not_found("Worker not found")),
        }
    }

    #[instrument(skip(self), fields(
        correlation_id = %extract_or_generate_correlation_id(&request),
        trace_id = %extract_or_generate_trace_id(&request),
        workflow_id = %request.get_ref().workflow_id,
        task_queue = %request.get_ref().task_queue,
        workflow_type = %request.get_ref().workflow_type
    ))]
    async fn start_workflow(
        &self,
        request: Request<StartWorkflowRequest>,
    ) -> Result<Response<StartWorkflowResponse>, Status> {
        let correlation_id = extract_or_generate_correlation_id(&request);
        let trace_id = extract_or_generate_trace_id(&request);

        log_grpc_request("StartWorkflow", &correlation_id, &trace_id, None);

        let req = request.into_inner();

        let input_json: serde_json::Value = if req.input.is_empty() {
            serde_json::json!(null)
        } else {
            serde_json::from_slice(&req.input)
                .map_err(|e| Status::invalid_argument(format!("Input must be valid JSON: {}", e)))?
        };

        let context_json: Option<serde_json::Value> = if req.context.is_empty() {
            None
        } else {
            Some(serde_json::from_slice(&req.context).map_err(|e| {
                Status::invalid_argument(format!("Context must be valid JSON: {}", e))
            })?)
        };

        let namespace_id = if req.namespace_id.is_empty() {
            "default".to_string()
        } else {
            req.namespace_id
        };

        let workflows = self.store.workflows();

        if !req.idempotency_key.is_empty()
            && let Some(existing_id) = workflows
                .find_by_idempotency_key(&namespace_id, &req.idempotency_key)
                .await
                .map_err(map_store_error)?
        {
            return Ok(Response::new(StartWorkflowResponse {
                run_id: existing_id.to_string(),
            }));
        }

        let version = if req.version.is_empty() {
            "1".to_string()
        } else {
            req.version
        };

        let run_id = workflows
            .create(CreateWorkflow {
                business_id: req.workflow_id,
                task_queue: req.task_queue,
                workflow_type: req.workflow_type,
                input: input_json,
                namespace_id,
                idempotency_key: if req.idempotency_key.is_empty() {
                    None
                } else {
                    Some(req.idempotency_key)
                },
                context: context_json,
                deadline_at: req.deadline_at.map(|ts| {
                    chrono::DateTime::from_timestamp(ts.seconds, ts.nanos as u32)
                        .unwrap_or_default()
                }),
                version,
                retry_policy: proto_policy_to_store(req.retry_policy),
            })
            .await
            .map_err(map_store_error)?;

        log_grpc_response(
            "StartWorkflow",
            &correlation_id,
            &trace_id,
            Status::code(&Status::ok("")),
            None,
        );

        Ok(Response::new(StartWorkflowResponse {
            run_id: run_id.to_string(),
        }))
    }

    #[instrument(skip(self), fields(
        correlation_id = %extract_or_generate_correlation_id(&request),
        trace_id = %extract_or_generate_trace_id(&request),
        run_id = %request.get_ref().run_id,
        namespace_id = %request.get_ref().namespace_id
    ))]
    async fn get_workflow_run(
        &self,
        request: Request<GetWorkflowRunRequest>,
    ) -> Result<Response<GetWorkflowRunResponse>, Status> {
        let correlation_id = extract_or_generate_correlation_id(&request);
        let trace_id = extract_or_generate_trace_id(&request);

        log_grpc_request("GetWorkflowRun", &correlation_id, &trace_id, None);

        let req = request.into_inner();

        let run_id = uuid::Uuid::parse_str(&req.run_id)
            .map_err(|_| Status::invalid_argument("Invalid run_id: must be a valid UUID"))?;

        let namespace_id = if req.namespace_id.is_empty() {
            "default".to_string()
        } else {
            req.namespace_id
        };

        let workflow = self
            .store
            .workflows()
            .find_by_id(run_id, &namespace_id)
            .await
            .map_err(map_store_error)?;

        match workflow {
            Some(w) => {
                let proto = workflow_to_proto(w)?;

                log_grpc_response(
                    "GetWorkflowRun",
                    &correlation_id,
                    &trace_id,
                    Status::code(&Status::ok("")),
                    None,
                );

                Ok(Response::new(GetWorkflowRunResponse {
                    workflow_run: Some(proto),
                }))
            }
            None => {
                let status = Status::not_found(format!(
                    "Workflow run not found: run_id={}, namespace_id={}",
                    run_id, namespace_id
                ));

                log_grpc_response(
                    "GetWorkflowRun",
                    &correlation_id,
                    &trace_id,
                    Status::code(&status),
                    Some("Workflow run not found"),
                );

                Err(status)
            }
        }
    }

    #[instrument(skip(self), fields(
        correlation_id = %extract_or_generate_correlation_id(&request),
        trace_id = %extract_or_generate_trace_id(&request),
        namespace_id = %request.get_ref().namespace_id,
        page_size = %request.get_ref().page_size,
        filter_status = %request.get_ref().filter_status
    ))]
    async fn list_workflow_runs(
        &self,
        request: Request<ListWorkflowRunsRequest>,
    ) -> Result<Response<ListWorkflowRunsResponse>, Status> {
        use base64::Engine;

        let correlation_id = extract_or_generate_correlation_id(&request);
        let trace_id = extract_or_generate_trace_id(&request);

        log_grpc_request("ListWorkflowRuns", &correlation_id, &trace_id, None);

        let req = request.into_inner();

        let namespace_id = if req.namespace_id.is_empty() {
            "default".to_string()
        } else {
            req.namespace_id
        };

        let page_size = if req.page_size <= 0 {
            20
        } else if req.page_size > 100 {
            100
        } else {
            req.page_size
        };

        // Parse cursor from page_token
        let cursor: Option<WorkflowCursor> = if req.page_token.is_empty() {
            None
        } else {
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(&req.page_token)
                .map_err(|_| Status::invalid_argument("Invalid page_token"))?;
            let cursor_str = String::from_utf8(decoded)
                .map_err(|_| Status::invalid_argument("Invalid page_token encoding"))?;
            let parts: Vec<&str> = cursor_str.splitn(2, ':').collect();
            if parts.len() != 2 {
                return Err(Status::invalid_argument("Invalid page_token format"));
            }
            let millis: i64 = parts[0]
                .parse()
                .map_err(|_| Status::invalid_argument("Invalid page_token timestamp"))?;
            let run_id = uuid::Uuid::parse_str(parts[1])
                .map_err(|_| Status::invalid_argument("Invalid page_token run_id"))?;
            let cursor_time = chrono::DateTime::from_timestamp_millis(millis)
                .ok_or_else(|| Status::invalid_argument("Invalid cursor timestamp"))?;
            Some(WorkflowCursor {
                created_at: cursor_time,
                run_id,
            })
        };

        let result = self
            .store
            .workflows()
            .list(ListWorkflowsParams {
                namespace_id,
                filter_status: if req.filter_status.is_empty() {
                    None
                } else {
                    Some(req.filter_status)
                },
                page_size,
                cursor,
            })
            .await
            .map_err(map_store_error)?;

        // Generate next page token
        let next_page_token = result
            .next_cursor
            .map(|c| {
                let cursor_str = format!("{}:{}", c.created_at.timestamp_millis(), c.run_id);
                base64::engine::general_purpose::STANDARD.encode(cursor_str.as_bytes())
            })
            .unwrap_or_default();

        // Convert to proto
        let workflow_runs: Result<Vec<_>, Status> = result
            .workflows
            .into_iter()
            .map(workflow_to_proto)
            .collect();
        let workflow_runs = workflow_runs?;

        let response = Response::new(ListWorkflowRunsResponse {
            workflow_runs,
            next_page_token,
            prev_page_token: String::new(),
            has_more: result.has_more,
        });

        log_grpc_response(
            "ListWorkflowRuns",
            &correlation_id,
            &trace_id,
            Status::code(&Status::ok("")),
            None,
        );

        Ok(response)
    }

    #[instrument(skip(self), fields(
        correlation_id = %extract_or_generate_correlation_id(&request),
        trace_id = %extract_or_generate_trace_id(&request),
        run_id = %request.get_ref().run_id,
        namespace_id = %request.get_ref().namespace_id
    ))]
    async fn cancel_workflow_run(
        &self,
        request: Request<CancelWorkflowRunRequest>,
    ) -> Result<Response<Empty>, Status> {
        let correlation_id = extract_or_generate_correlation_id(&request);
        let trace_id = extract_or_generate_trace_id(&request);

        log_grpc_request("CancelWorkflowRun", &correlation_id, &trace_id, None);

        let req = request.into_inner();

        let run_id = uuid::Uuid::parse_str(&req.run_id)
            .map_err(|_| Status::invalid_argument("Invalid run_id: must be a valid UUID"))?;

        let namespace_id = if req.namespace_id.is_empty() {
            "default".to_string()
        } else {
            req.namespace_id
        };

        let workflows = self.store.workflows();

        let cancelled = workflows
            .cancel(run_id, &namespace_id)
            .await
            .map_err(map_store_error)?;

        if cancelled {
            info!("Workflow {} cancelled successfully", run_id);

            log_grpc_response(
                "CancelWorkflowRun",
                &correlation_id,
                &trace_id,
                Status::code(&Status::ok("")),
                None,
            );

            Ok(Response::new(Empty {}))
        } else {
            // Check why cancellation failed
            let exists = workflows
                .check_exists(run_id, &namespace_id)
                .await
                .map_err(map_store_error)?;

            let status = if exists.exists {
                Status::failed_precondition(format!(
                    "Cannot cancel workflow with status '{:?}'. Only PENDING, RUNNING, or SLEEPING workflows can be cancelled.",
                    exists.status
                ))
            } else {
                Status::not_found(format!(
                    "Workflow run not found: run_id={}, namespace_id={}",
                    run_id, namespace_id
                ))
            };

            log_grpc_response(
                "CancelWorkflowRun",
                &correlation_id,
                &trace_id,
                Status::code(&status),
                Some("Workflow cancellation failed"),
            );

            Err(status)
        }
    }

    #[instrument(skip(self), fields(
        correlation_id = %extract_or_generate_correlation_id(&request),
        trace_id = %extract_or_generate_trace_id(&request),
        task_queue = %request.get_ref().task_queue,
        worker_id = %request.get_ref().worker_id
    ))]
    async fn poll_activity(
        &self,
        request: Request<PollActivityRequest>,
    ) -> Result<Response<PollActivityResponse>, Status> {
        let correlation_id = extract_or_generate_correlation_id(&request);
        let trace_id = extract_or_generate_trace_id(&request);

        log_grpc_request("PollActivity", &correlation_id, &trace_id, None);

        let req = request.into_inner();

        let worker_id = uuid::Uuid::parse_str(&req.worker_id)
            .map_err(|_| Status::invalid_argument("Invalid worker_id"))?;

        if req.supported_workflow_types.is_empty() {
            return Err(Status::invalid_argument(
                "supported_workflow_types cannot be empty",
            ));
        }

        let namespace_id = if req.namespace_id.is_empty() {
            "default".to_string()
        } else {
            req.namespace_id
        };

        // Load worker record and validate queue/namespace/status.
        let worker = self
            .store
            .workers()
            .find_by_id(worker_id)
            .await
            .map_err(map_store_error)?
            .ok_or_else(|| {
                Status::failed_precondition(
                    "Worker not registered or offline. Call RegisterWorker first.",
                )
            })?;

        if worker.namespace_id != namespace_id || worker.task_queue != req.task_queue {
            return Err(Status::failed_precondition(
                "Worker not registered for the requested namespace/task_queue",
            ));
        }

        if worker.status == WorkerStatus::Offline {
            return Err(Status::failed_precondition(
                "Worker not registered or offline. Call RegisterWorker first.",
            ));
        }

        if worker.status == WorkerStatus::Draining {
            return Err(Status::failed_precondition(
                "Worker is draining and not accepting new work",
            ));
        }

        // Use the worker's registered types, optionally narrowed by client request.
        let effective_types: Vec<String> = worker
            .workflow_types
            .iter()
            .filter(|t| req.supported_workflow_types.iter().any(|r| r == *t))
            .cloned()
            .collect();

        if effective_types.is_empty() {
            return Err(Status::failed_precondition(
                "Worker is not registered for the requested workflow types",
            ));
        }

        // Fast path: attempt immediate claim before long-polling.
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

            let input_bytes = serde_json::to_vec(&work_item.input).map_err(|e| {
                tracing::error!("Failed to serialize workflow input: {:?}", e);
                Status::internal("Failed to serialize workflow input")
            })?;

            log_grpc_response(
                "PollActivity",
                &correlation_id,
                &trace_id,
                Status::code(&Status::ok("")),
                None,
            );

            return Ok(Response::new(PollActivityResponse {
                run_id: work_item.run_id.to_string(),
                workflow_type: work_item.workflow_type,
                workflow_input: input_bytes,
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

                let input_bytes = serde_json::to_vec(&work_item.input).map_err(|e| {
                    tracing::error!("Failed to serialize workflow input: {:?}", e);
                    Status::internal("Failed to serialize workflow input")
                })?;

                info!(
                    correlation_id = correlation_id,
                    trace_id = trace_id,
                    run_id = %work_item.run_id,
                    workflow_type = %work_item.workflow_type,
                    worker_id = %req.worker_id,
                    "Worker claimed workflow via distributor"
                );

                log_grpc_response(
                    "PollActivity",
                    &correlation_id,
                    &trace_id,
                    Status::code(&Status::ok("")),
                    None,
                );

                Ok(Response::new(PollActivityResponse {
                    run_id: work_item.run_id.to_string(),
                    workflow_type: work_item.workflow_type,
                    workflow_input: input_bytes,
                }))
            }
            None => {
                log_grpc_response(
                    "PollActivity",
                    &correlation_id,
                    &trace_id,
                    Status::code(&Status::ok("")),
                    Some("No work available - timeout"),
                );

                Ok(Response::new(PollActivityResponse {
                    run_id: String::new(),
                    workflow_type: String::new(),
                    workflow_input: vec![],
                }))
            }
        }
    }

    #[instrument(skip(self), fields(
        correlation_id = %extract_or_generate_correlation_id(&request),
        trace_id = %extract_or_generate_trace_id(&request),
        step_attempt_id = %request.get_ref().step_attempt_id
    ))]
    async fn get_step_attempt(
        &self,
        request: Request<GetStepAttemptRequest>,
    ) -> Result<Response<GetStepAttemptResponse>, Status> {
        let correlation_id = extract_or_generate_correlation_id(&request);
        let trace_id = extract_or_generate_trace_id(&request);

        log_grpc_request("GetStepAttempt", &correlation_id, &trace_id, None);

        let req = request.into_inner();
        let attempt_id = uuid::Uuid::parse_str(&req.step_attempt_id)
            .map_err(|_| Status::invalid_argument("Invalid step_attempt_id"))?;

        let step = self
            .store
            .steps()
            .find_by_id(attempt_id)
            .await
            .map_err(map_store_error)?;

        match step {
            Some(s) => {
                let proto = step_to_proto(s)?;

                log_grpc_response(
                    "GetStepAttempt",
                    &correlation_id,
                    &trace_id,
                    Status::code(&Status::ok("")),
                    None,
                );

                Ok(Response::new(GetStepAttemptResponse {
                    step_attempt: Some(proto),
                }))
            }
            None => {
                let status = Status::not_found("Step attempt not found");

                log_grpc_response(
                    "GetStepAttempt",
                    &correlation_id,
                    &trace_id,
                    Status::code(&status),
                    Some("Step attempt not found"),
                );

                Err(status)
            }
        }
    }

    #[instrument(skip(self), fields(
        correlation_id = %extract_or_generate_correlation_id(&request),
        trace_id = %extract_or_generate_trace_id(&request),
        workflow_run_id = %request.get_ref().workflow_run_id,
        step_id = %request.get_ref().step_id,
        page_size = %request.get_ref().page_size
    ))]
    async fn list_step_attempts(
        &self,
        request: Request<ListStepAttemptsRequest>,
    ) -> Result<Response<ListStepAttemptsResponse>, Status> {
        let correlation_id = extract_or_generate_correlation_id(&request);
        let trace_id = extract_or_generate_trace_id(&request);

        log_grpc_request("ListStepAttempts", &correlation_id, &trace_id, None);

        let req = request.into_inner();
        let run_id = uuid::Uuid::parse_str(&req.workflow_run_id)
            .map_err(|_| Status::invalid_argument("Invalid workflow_run_id"))?;

        let page_size = if req.page_size <= 0 {
            50
        } else if req.page_size > 100 {
            100
        } else {
            req.page_size
        };

        let step_id = if req.step_id.is_empty() {
            None
        } else {
            Some(req.step_id.as_str())
        };

        let steps = self
            .store
            .steps()
            .list_by_workflow(run_id, step_id, page_size)
            .await
            .map_err(map_store_error)?;

        let attempts: Result<Vec<_>, Status> = steps.into_iter().map(step_to_proto).collect();
        let attempts = attempts?;

        log_grpc_response(
            "ListStepAttempts",
            &correlation_id,
            &trace_id,
            Status::code(&Status::ok("")),
            None,
        );

        Ok(Response::new(ListStepAttemptsResponse {
            step_attempts: attempts,
            next_page_token: String::new(),
        }))
    }

    #[instrument(skip(self), fields(
        correlation_id = %extract_or_generate_correlation_id(&request),
        trace_id = %extract_or_generate_trace_id(&request),
        run_id = %request.get_ref().run_id,
        step_id = %request.get_ref().step_id
    ))]
    async fn begin_step(
        &self,
        request: Request<BeginStepRequest>,
    ) -> Result<Response<BeginStepResponse>, Status> {
        let correlation_id = extract_or_generate_correlation_id(&request);
        let trace_id = extract_or_generate_trace_id(&request);

        log_grpc_request("BeginStep", &correlation_id, &trace_id, None);

        let req = request.into_inner();

        let run_id = uuid::Uuid::parse_str(&req.run_id)
            .map_err(|_| Status::invalid_argument("Invalid run_id"))?;

        let input: Option<serde_json::Value> = if !req.input.is_empty() {
            Some(serde_json::from_slice(&req.input).map_err(|e| {
                Status::invalid_argument(format!("Input must be valid JSON: {}", e))
            })?)
        } else {
            None
        };

        let result = self
            .store
            .steps()
            .begin(BeginStepParams {
                run_id,
                step_id: req.step_id,
                input,
                retry_policy: proto_policy_to_store(req.retry_policy),
            })
            .await
            .map_err(map_store_error)?;

        let cached_result = result
            .cached_output
            .map(|o| serde_json::to_vec(&o))
            .transpose()
            .map_err(|e| {
                tracing::error!("Failed to serialize cached step output: {:?}", e);
                Status::internal("Failed to serialize cached step output")
            })?
            .unwrap_or_default();

        log_grpc_response(
            "BeginStep",
            &correlation_id,
            &trace_id,
            Status::code(&Status::ok("")),
            None,
        );

        Ok(Response::new(BeginStepResponse {
            should_execute: result.should_execute,
            cached_result,
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
    ) -> Result<Response<Empty>, Status> {
        let correlation_id = extract_or_generate_correlation_id(&request);
        let trace_id = extract_or_generate_trace_id(&request);

        log_grpc_request("CompleteStep", &correlation_id, &trace_id, None);

        let req = request.into_inner();

        let run_id = uuid::Uuid::parse_str(&req.run_id)
            .map_err(|_| Status::invalid_argument("Invalid run_id"))?;

        let output_json: serde_json::Value = serde_json::from_slice(&req.output)
            .map_err(|e| Status::invalid_argument(format!("Output must be valid JSON: {}", e)))?;

        self.store
            .steps()
            .complete(run_id, &req.step_id, output_json)
            .await
            .map_err(map_store_error)?;

        log_grpc_response(
            "CompleteStep",
            &correlation_id,
            &trace_id,
            Status::code(&Status::ok("")),
            None,
        );

        Ok(Response::new(Empty {}))
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
    ) -> Result<Response<Empty>, Status> {
        let correlation_id = extract_or_generate_correlation_id(&request);
        let trace_id = extract_or_generate_trace_id(&request);

        log_grpc_request("FailStep", &correlation_id, &trace_id, None);

        let req = request.into_inner();

        let run_id = uuid::Uuid::parse_str(&req.run_id)
            .map_err(|_| Status::invalid_argument("Invalid run_id: must be a valid UUID"))?;

        if req.step_id.is_empty() {
            return Err(Status::invalid_argument("step_id is required"));
        }

        let result = self
            .store
            .steps()
            .fail(FailStepParams {
                run_id,
                step_id: req.step_id.clone(),
                error: req.error,
                non_retryable: req.non_retryable,
                retry_after_ms: if req.retry_after_ms > 0 {
                    Some(req.retry_after_ms)
                } else {
                    None
                },
            })
            .await
            .map_err(map_store_error)?;

        if result.scheduled_retry {
            info!(
                run_id = %run_id,
                step_id = %req.step_id,
                retry_at = ?result.retry_at,
                "Step failed, scheduling retry"
            );
        } else {
            info!(
                run_id = %run_id,
                step_id = %req.step_id,
                "Step failed permanently"
            );
        }

        log_grpc_response(
            "FailStep",
            &correlation_id,
            &trace_id,
            Status::code(&Status::ok("")),
            None,
        );

        Ok(Response::new(Empty {}))
    }

    #[instrument(skip(self), fields(
        correlation_id = %extract_or_generate_correlation_id(&request),
        trace_id = %extract_or_generate_trace_id(&request),
        run_id = %request.get_ref().run_id
    ))]
    async fn complete_workflow(
        &self,
        request: Request<CompleteWorkflowRequest>,
    ) -> Result<Response<Empty>, Status> {
        let correlation_id = extract_or_generate_correlation_id(&request);
        let trace_id = extract_or_generate_trace_id(&request);

        log_grpc_request("CompleteWorkflow", &correlation_id, &trace_id, None);

        let req = request.into_inner();

        let run_id = uuid::Uuid::parse_str(&req.run_id)
            .map_err(|_| Status::invalid_argument("Invalid run_id"))?;

        let output_json: serde_json::Value = serde_json::from_slice(&req.output)
            .map_err(|e| Status::invalid_argument(format!("Output must be valid JSON: {}", e)))?;

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

        Ok(Response::new(Empty {}))
    }

    #[instrument(skip(self), fields(
        correlation_id = %extract_or_generate_correlation_id(&request),
        trace_id = %extract_or_generate_trace_id(&request),
        run_id = %request.get_ref().run_id
    ))]
    async fn fail_workflow(
        &self,
        request: Request<FailWorkflowRequest>,
    ) -> Result<Response<Empty>, Status> {
        let correlation_id = extract_or_generate_correlation_id(&request);
        let trace_id = extract_or_generate_trace_id(&request);

        log_grpc_request("FailWorkflow", &correlation_id, &trace_id, None);

        let req = request.into_inner();

        let run_id = uuid::Uuid::parse_str(&req.run_id)
            .map_err(|_| Status::invalid_argument("Invalid run_id"))?;

        self.store
            .workflows()
            .fail(run_id, &req.error)
            .await
            .map_err(map_store_error)?;

        log_grpc_response(
            "FailWorkflow",
            &correlation_id,
            &trace_id,
            Status::code(&Status::ok("")),
            None,
        );

        Ok(Response::new(Empty {}))
    }

    #[instrument(skip(self), fields(
        correlation_id = %extract_or_generate_correlation_id(&request),
        trace_id = %extract_or_generate_trace_id(&request),
        run_id = %request.get_ref().run_id,
        duration_seconds = %request.get_ref().duration_seconds
    ))]
    async fn schedule_sleep(
        &self,
        request: Request<ScheduleSleepRequest>,
    ) -> Result<Response<Empty>, Status> {
        let correlation_id = extract_or_generate_correlation_id(&request);
        let trace_id = extract_or_generate_trace_id(&request);

        log_grpc_request("ScheduleSleep", &correlation_id, &trace_id, None);

        let req = request.into_inner();

        let run_id = uuid::Uuid::parse_str(&req.run_id)
            .map_err(|_| Status::invalid_argument("Invalid run_id"))?;

        self.store
            .workflows()
            .schedule_sleep(run_id, req.duration_seconds)
            .await
            .map_err(map_store_error)?;

        log_grpc_response(
            "ScheduleSleep",
            &correlation_id,
            &trace_id,
            Status::code(&Status::ok("")),
            None,
        );

        Ok(Response::new(Empty {}))
    }

    async fn health_check(
        &self,
        request: Request<HealthCheckRequest>,
    ) -> Result<Response<HealthCheckResponse>, Status> {
        let correlation_id = extract_or_generate_correlation_id(&request);
        let trace_id = extract_or_generate_trace_id(&request);

        log_grpc_request("HealthCheck", &correlation_id, &trace_id, None);

        let _req = request.into_inner();
        let db_status = match sqlx::query("SELECT 1").fetch_one(self.store.pool()).await {
            Ok(_) => {
                info!(
                    correlation_id = correlation_id,
                    trace_id = trace_id,
                    "Database health check passed"
                );
                kagzi_proto::kagzi::health_check_response::ServingStatus::Serving
            }
            Err(e) => {
                tracing::error!(
                    correlation_id = correlation_id,
                    trace_id = trace_id,
                    error = %e,
                    "Database health check failed"
                );
                kagzi_proto::kagzi::health_check_response::ServingStatus::NotServing
            }
        };

        let response = HealthCheckResponse {
            status: db_status as i32,
            message: match db_status {
                kagzi_proto::kagzi::health_check_response::ServingStatus::Serving => {
                    "Service is healthy and serving requests".to_string()
                }
                kagzi_proto::kagzi::health_check_response::ServingStatus::NotServing => {
                    "Service is not healthy - database connection failed".to_string()
                }
                _ => "Unknown status".to_string(),
            },
            timestamp: Some(prost_types::Timestamp {
                seconds: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs() as i64,
                nanos: 0,
            }),
        };

        log_grpc_response(
            "HealthCheck",
            &correlation_id,
            &trace_id,
            Status::code(&Status::ok("")),
            None,
        );

        Ok(Response::new(response))
    }
}
