use crate::helpers::{invalid_argument, json_to_payload, map_store_error, not_found};
use crate::tracing_utils::{
    extract_or_generate_correlation_id, extract_or_generate_trace_id, log_grpc_request,
    log_grpc_response,
};
use kagzi_proto::kagzi::{
    GetServerInfoRequest, GetServerInfoResponse, GetStepRequest, GetStepResponse, GetWorkerRequest,
    GetWorkerResponse, HealthCheckRequest, HealthCheckResponse, ListStepsRequest,
    ListStepsResponse, ListWorkersRequest, ListWorkersResponse, PageInfo, ServingStatus, Step,
    Worker, WorkerStatus, admin_service_server::AdminService,
};
use kagzi_store::{
    PgStore, StepKind as StoreStepKind, StepRepository, WorkerRepository,
    WorkerStatus as StoreWorkerStatus,
};
use tonic::{Request, Response, Status};
use tracing::{info, instrument};
use uuid::Uuid;

fn worker_to_proto(w: kagzi_store::Worker) -> Worker {
    let labels = match w.labels {
        serde_json::Value::Object(map) => map
            .into_iter()
            .filter_map(|(k, v)| v.as_str().map(|s| (k, s.to_string())))
            .collect(),
        _ => std::collections::HashMap::new(),
    };

    Worker {
        worker_id: w.worker_id.to_string(),
        namespace_id: w.namespace_id,
        task_queue: w.task_queue,
        status: match w.status {
            StoreWorkerStatus::Online => WorkerStatus::Online as i32,
            StoreWorkerStatus::Draining => WorkerStatus::Draining as i32,
            StoreWorkerStatus::Offline => WorkerStatus::Offline as i32,
        },
        hostname: w.hostname.unwrap_or_default(),
        pid: w.pid.unwrap_or(0),
        version: w.version.unwrap_or_default(),
        workflow_types: w.workflow_types,
        max_concurrent: w.max_concurrent,
        active_count: w.active_count,
        total_completed: w.total_completed,
        total_failed: w.total_failed,
        registered_at: Some(timestamp_from(w.registered_at)),
        last_heartbeat_at: Some(timestamp_from(w.last_heartbeat_at)),
        labels,
        queue_concurrency_limit: w.queue_concurrency_limit,
        workflow_type_concurrency: w
            .workflow_type_concurrency
            .into_iter()
            .map(|c| kagzi_proto::kagzi::WorkflowTypeConcurrency {
                workflow_type: c.workflow_type,
                max_concurrent: c.max_concurrent,
            })
            .collect(),
    }
}

fn map_step_status(status: kagzi_store::StepStatus) -> kagzi_proto::kagzi::StepStatus {
    match status {
        kagzi_store::StepStatus::Pending => kagzi_proto::kagzi::StepStatus::Pending,
        kagzi_store::StepStatus::Running => kagzi_proto::kagzi::StepStatus::Running,
        kagzi_store::StepStatus::Completed => kagzi_proto::kagzi::StepStatus::Completed,
        kagzi_store::StepStatus::Failed => kagzi_proto::kagzi::StepStatus::Failed,
    }
}

fn map_step_kind(kind: StoreStepKind) -> kagzi_proto::kagzi::StepKind {
    match kind {
        StoreStepKind::Function => kagzi_proto::kagzi::StepKind::Function,
        StoreStepKind::Sleep => kagzi_proto::kagzi::StepKind::Sleep,
    }
}

fn step_to_proto(s: kagzi_store::StepRun) -> Result<Step, Status> {
    let input = json_to_payload(s.input)?;
    let output = json_to_payload(s.output)?;
    let error = s.error.map(|msg| kagzi_proto::kagzi::ErrorDetail {
        code: kagzi_proto::kagzi::ErrorCode::Unspecified as i32,
        message: msg,
        non_retryable: false,
        retry_after_ms: 0,
        subject: String::new(),
        subject_id: String::new(),
        metadata: std::collections::HashMap::new(),
    });

    let step_id = s.step_id.clone();

    Ok(Step {
        step_id: step_id.clone(),
        run_id: s.run_id.to_string(),
        namespace_id: s.namespace_id,
        name: step_id.clone(),
        kind: map_step_kind(s.step_kind) as i32,
        status: map_step_status(s.status) as i32,
        attempt_number: s.attempt_number,
        input: Some(input),
        output: Some(output),
        error,
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

fn normalize_worker_status(status: Option<i32>) -> Result<Option<StoreWorkerStatus>, Status> {
    match status {
        None => Ok(None),
        Some(raw) => match WorkerStatus::try_from(raw)
            .map_err(|_| invalid_argument("Invalid status_filter"))?
        {
            WorkerStatus::Online => Ok(Some(StoreWorkerStatus::Online)),
            WorkerStatus::Draining => Ok(Some(StoreWorkerStatus::Draining)),
            WorkerStatus::Offline => Ok(Some(StoreWorkerStatus::Offline)),
            WorkerStatus::Unspecified => Ok(None),
        },
    }
}

pub struct AdminServiceImpl {
    pub store: PgStore,
}

impl AdminServiceImpl {
    pub fn new(store: PgStore) -> Self {
        Self { store }
    }
}

#[tonic::async_trait]
impl AdminService for AdminServiceImpl {
    #[instrument(skip(self), fields(
        correlation_id = %extract_or_generate_correlation_id(&request),
        trace_id = %extract_or_generate_trace_id(&request),
        namespace_id = %request.get_ref().namespace_id
    ))]
    async fn list_workers(
        &self,
        request: Request<ListWorkersRequest>,
    ) -> Result<Response<ListWorkersResponse>, Status> {
        let correlation_id = extract_or_generate_correlation_id(&request);
        let trace_id = extract_or_generate_trace_id(&request);
        log_grpc_request("ListWorkers", &correlation_id, &trace_id, None);

        let req = request.into_inner();
        let page = req.page.unwrap_or_default();

        let namespace_id = if req.namespace_id.is_empty() {
            "default".to_string()
        } else {
            req.namespace_id
        };

        let page_size = if page.page_size <= 0 {
            20
        } else if page.page_size > 100 {
            100
        } else {
            page.page_size
        };

        let cursor = if page.page_token.is_empty() {
            None
        } else {
            Some(
                Uuid::parse_str(&page.page_token)
                    .map_err(|_| invalid_argument("Invalid page_token"))?,
            )
        };

        let filter_status = normalize_worker_status(req.status_filter)?;

        let task_queue = req.task_queue.clone();

        let mut workers = self
            .store
            .workers()
            .list(kagzi_store::ListWorkersParams {
                namespace_id: namespace_id.clone(),
                task_queue: task_queue.clone().filter(|t| !t.is_empty()),
                filter_status,
                page_size,
                cursor,
            })
            .await
            .map_err(map_store_error)?;

        let mut next_page_token = String::new();
        let mut has_more = false;
        if workers.len() as i32 > page_size
            && let Some(last) = workers.pop()
        {
            next_page_token = last.worker_id.to_string();
            has_more = true;
        }

        let total_count = if page.include_total_count {
            self.store
                .workers()
                .count(
                    &namespace_id,
                    task_queue.as_deref().filter(|s| !s.is_empty()),
                    filter_status,
                )
                .await
                .map_err(map_store_error)?
        } else {
            0
        };

        let proto_workers = workers.into_iter().map(worker_to_proto).collect();

        let page_info = PageInfo {
            next_page_token,
            has_more,
            total_count,
        };

        log_grpc_response(
            "ListWorkers",
            &correlation_id,
            &trace_id,
            Status::code(&Status::ok("")),
            None,
        );

        Ok(Response::new(ListWorkersResponse {
            workers: proto_workers,
            page: Some(page_info),
        }))
    }

    #[instrument(skip(self), fields(
        correlation_id = %extract_or_generate_correlation_id(&request),
        trace_id = %extract_or_generate_trace_id(&request),
        worker_id = %request.get_ref().worker_id
    ))]
    async fn get_worker(
        &self,
        request: Request<GetWorkerRequest>,
    ) -> Result<Response<GetWorkerResponse>, Status> {
        let correlation_id = extract_or_generate_correlation_id(&request);
        let trace_id = extract_or_generate_trace_id(&request);
        log_grpc_request("GetWorker", &correlation_id, &trace_id, None);

        let req = request.into_inner();
        let worker_id =
            Uuid::parse_str(&req.worker_id).map_err(|_| invalid_argument("Invalid worker_id"))?;

        let worker = self
            .store
            .workers()
            .find_by_id(worker_id)
            .await
            .map_err(map_store_error)?;

        match worker {
            Some(w) => {
                let proto = worker_to_proto(w);
                log_grpc_response(
                    "GetWorker",
                    &correlation_id,
                    &trace_id,
                    Status::code(&Status::ok("")),
                    None,
                );
                Ok(Response::new(GetWorkerResponse {
                    worker: Some(proto),
                }))
            }
            None => Err(not_found("Worker not found", "worker", req.worker_id)),
        }
    }

    #[instrument(skip(self), fields(
        correlation_id = %extract_or_generate_correlation_id(&request),
        trace_id = %extract_or_generate_trace_id(&request),
        step_id = %request.get_ref().step_id
    ))]
    async fn get_step(
        &self,
        request: Request<GetStepRequest>,
    ) -> Result<Response<GetStepResponse>, Status> {
        let correlation_id = extract_or_generate_correlation_id(&request);
        let trace_id = extract_or_generate_trace_id(&request);
        log_grpc_request("GetStep", &correlation_id, &trace_id, None);

        let req = request.into_inner();
        let step_id =
            Uuid::parse_str(&req.step_id).map_err(|_| invalid_argument("Invalid step_id"))?;

        let step = self
            .store
            .steps()
            .find_by_id(step_id)
            .await
            .map_err(map_store_error)?;

        match step {
            Some(s) => {
                let proto = step_to_proto(s)?;
                log_grpc_response(
                    "GetStep",
                    &correlation_id,
                    &trace_id,
                    Status::code(&Status::ok("")),
                    None,
                );
                Ok(Response::new(GetStepResponse { step: Some(proto) }))
            }
            None => Err(not_found("Step not found", "step", req.step_id)),
        }
    }

    #[instrument(skip(self), fields(
        correlation_id = %extract_or_generate_correlation_id(&request),
        trace_id = %extract_or_generate_trace_id(&request),
        run_id = %request.get_ref().run_id
    ))]
    async fn list_steps(
        &self,
        request: Request<ListStepsRequest>,
    ) -> Result<Response<ListStepsResponse>, Status> {
        let correlation_id = extract_or_generate_correlation_id(&request);
        let trace_id = extract_or_generate_trace_id(&request);
        log_grpc_request("ListSteps", &correlation_id, &trace_id, None);

        let req = request.into_inner();
        let run_id =
            Uuid::parse_str(&req.run_id).map_err(|_| invalid_argument("Invalid run_id"))?;

        let page = req.page.unwrap_or_default();
        let page_size = if page.page_size <= 0 {
            50
        } else if page.page_size > 100 {
            100
        } else {
            page.page_size
        };

        let step_name = req.step_name.filter(|s| !s.is_empty());

        let steps = self
            .store
            .steps()
            .list_by_workflow(run_id, step_name.as_deref(), page_size)
            .await
            .map_err(map_store_error)?;

        let attempts: Result<Vec<_>, Status> = steps.into_iter().map(step_to_proto).collect();
        let attempts = attempts?;

        let page_info = PageInfo {
            next_page_token: String::new(),
            has_more: false,
            total_count: 0,
        };

        log_grpc_response(
            "ListSteps",
            &correlation_id,
            &trace_id,
            Status::code(&Status::ok("")),
            None,
        );

        Ok(Response::new(ListStepsResponse {
            steps: attempts,
            page: Some(page_info),
        }))
    }

    #[instrument(skip(self), fields(
        correlation_id = %extract_or_generate_correlation_id(&request),
        trace_id = %extract_or_generate_trace_id(&request)
    ))]
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
                ServingStatus::Serving
            }
            Err(e) => {
                tracing::error!(
                    correlation_id = correlation_id,
                    trace_id = trace_id,
                    error = %e,
                    "Database health check failed"
                );
                ServingStatus::NotServing
            }
        };

        let response = HealthCheckResponse {
            status: db_status as i32,
            message: match db_status {
                ServingStatus::Serving => "Service is healthy and serving requests".to_string(),
                ServingStatus::NotServing => {
                    "Service is not healthy - database connection failed".to_string()
                }
                _ => "Unknown status".to_string(),
            },
            timestamp: Some(prost_types::Timestamp {
                seconds: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
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

    #[instrument(skip(self), fields(
        correlation_id = %extract_or_generate_correlation_id(&request),
        trace_id = %extract_or_generate_trace_id(&request)
    ))]
    async fn get_server_info(
        &self,
        request: Request<GetServerInfoRequest>,
    ) -> Result<Response<GetServerInfoResponse>, Status> {
        let correlation_id = extract_or_generate_correlation_id(&request);
        let trace_id = extract_or_generate_trace_id(&request);
        log_grpc_request("GetServerInfo", &correlation_id, &trace_id, None);

        let _req = request.into_inner();

        let response = GetServerInfoResponse {
            version: env!("CARGO_PKG_VERSION").to_string(),
            api_version: "v1".to_string(),
            supported_features: vec![
                "workflow".to_string(),
                "worker".to_string(),
                "workflow_schedule".to_string(),
                "admin".to_string(),
            ],
            min_sdk_version: "0.1.0".to_string(),
        };

        log_grpc_response(
            "GetServerInfo",
            &correlation_id,
            &trace_id,
            Status::code(&Status::ok("")),
            None,
        );

        Ok(Response::new(response))
    }
}
