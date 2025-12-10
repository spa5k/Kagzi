use chrono::{TimeZone, Utc};
use kagzi_proto::kagzi::admin_service_server::AdminService;
use kagzi_proto::kagzi::{
    GetServerInfoRequest, GetServerInfoResponse, GetStepRequest, GetStepResponse, GetWorkerRequest,
    GetWorkerResponse, HealthCheckRequest, HealthCheckResponse, ListStepsRequest,
    ListStepsResponse, ListWorkersRequest, ListWorkersResponse, PageInfo, ServingStatus,
    WorkerStatus,
};
use kagzi_store::{
    PgStore, StepRepository, WorkerRepository, WorkerStatus as StoreWorkerStatus,
    WorkflowRepository,
};
use tonic::{Request, Response, Status};
use tracing::{info, instrument};
use uuid::Uuid;

use crate::helpers::{invalid_argument, map_store_error, not_found};
use crate::proto_convert::{step_to_proto, worker_to_proto};
use crate::tracing_utils::{
    extract_or_generate_correlation_id, extract_or_generate_trace_id, log_grpc_request,
    log_grpc_response,
};

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

        let workers_result = self
            .store
            .workers()
            .list(kagzi_store::ListWorkersParams {
                namespace_id: namespace_id.clone(),
                task_queue: task_queue.clone().filter(|t| !t.is_empty()),
                filter_status,
                page_size,
                cursor: cursor.map(|c| kagzi_store::WorkerCursor { worker_id: c }),
            })
            .await
            .map_err(map_store_error)?;

        let next_page_token = workers_result
            .next_cursor
            .map(|c| c.worker_id.to_string())
            .unwrap_or_default();

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

        let proto_workers = workers_result
            .items
            .into_iter()
            .map(worker_to_proto)
            .collect();

        let page_info = PageInfo {
            next_page_token,
            has_more: workers_result.has_more,
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

        let workflow = self
            .store
            .workflows()
            .find_by_id(run_id, "default") // Try default first
            .await
            .map_err(map_store_error)?;
        let namespace_id = workflow
            .map(|w| w.namespace_id)
            .unwrap_or_else(|| "default".to_string());

        let page = req.page.unwrap_or_default();
        let page_size = if page.page_size <= 0 {
            50
        } else if page.page_size > 100 {
            100
        } else {
            page.page_size
        };

        let cursor = if page.page_token.is_empty() {
            None
        } else {
            let mut parts = page.page_token.splitn(2, ':');
            let created_at_ms = parts
                .next()
                .and_then(|p| p.parse::<i64>().ok())
                .ok_or_else(|| invalid_argument("Invalid page_token"))?;
            let attempt_id_str = parts
                .next()
                .ok_or_else(|| invalid_argument("Invalid page_token"))?;

            let created_at = Utc
                .timestamp_millis_opt(created_at_ms)
                .single()
                .ok_or_else(|| invalid_argument("Invalid page_token"))?;
            let attempt_id = Uuid::parse_str(attempt_id_str)
                .map_err(|_| invalid_argument("Invalid page_token"))?;

            Some(kagzi_store::StepCursor {
                created_at,
                attempt_id,
            })
        };

        let step_name = req.step_name.filter(|s| !s.is_empty());

        let steps_result = self
            .store
            .steps()
            .list(kagzi_store::ListStepsParams {
                run_id,
                namespace_id,
                step_id: step_name,
                page_size,
                cursor,
            })
            .await
            .map_err(map_store_error)?;

        let attempts: Result<Vec<_>, Status> =
            steps_result.items.into_iter().map(step_to_proto).collect();
        let attempts = attempts?;

        let page_info = PageInfo {
            next_page_token: steps_result
                .next_cursor
                .map(|c| format!("{}:{}", c.created_at.timestamp_millis(), c.attempt_id))
                .unwrap_or_default(),
            has_more: steps_result.has_more,
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
        let db_status =
            match kagzi_store::HealthRepository::health_check(&self.store.health()).await {
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
