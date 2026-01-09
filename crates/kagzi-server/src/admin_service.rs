use kagzi_proto::kagzi::admin_service_server::AdminService;
use kagzi_proto::kagzi::{
    DrainWorkerRequest, DrainWorkerResponse, GetQueueDepthRequest, GetQueueDepthResponse,
    GetServerInfoRequest, GetServerInfoResponse, GetStatsRequest, GetStatsResponse, GetStepRequest,
    GetStepResponse, GetWorkerRequest, GetWorkerResponse, HealthCheckRequest, HealthCheckResponse,
    ListStepsRequest, ListStepsResponse, ListWorkersRequest, ListWorkersResponse,
    ListWorkflowTypesRequest, ListWorkflowTypesResponse, PageInfo, ServingStatus, WorkerStatus,
};
use kagzi_store::{
    NamespaceRepository, PgStore, StepRepository, WorkerRepository,
    WorkerStatus as StoreWorkerStatus, WorkflowRepository,
};
use tonic::{Request, Response, Status};
use uuid::Uuid;

use crate::helpers::{
    decode_cursor, encode_cursor, invalid_argument_error, map_store_error, normalize_page_size,
    not_found_error, require_non_empty,
};
use crate::proto_convert::{step_to_proto, worker_to_proto};

fn normalize_worker_status(status: Option<i32>) -> Result<Option<StoreWorkerStatus>, Status> {
    match status {
        None => Ok(None),
        Some(raw) => match WorkerStatus::try_from(raw)
            .map_err(|_| invalid_argument_error("Invalid status_filter"))?
        {
            WorkerStatus::Draining => Ok(Some(StoreWorkerStatus::Draining)),
            WorkerStatus::Offline => Ok(Some(StoreWorkerStatus::Offline)),
            WorkerStatus::Unspecified => Ok(None),
            WorkerStatus::Online => Ok(Some(StoreWorkerStatus::Online)),
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
    async fn list_workers(
        &self,
        request: Request<ListWorkersRequest>,
    ) -> Result<Response<ListWorkersResponse>, Status> {
        let req = request.into_inner();
        let page = req.page.unwrap_or_default();

        if req.namespace.is_empty() {
            return Err(invalid_argument_error("namespace is required"));
        }
        let namespace = req.namespace;

        let page_size = normalize_page_size(page.page_size, 20, 100);

        let cursor = if page.page_token.is_empty() {
            None
        } else {
            Some(
                Uuid::parse_str(&page.page_token)
                    .map_err(|_| invalid_argument_error("Invalid page_token"))?,
            )
        };

        let filter_status = normalize_worker_status(req.status_filter)?;

        let task_queue = req.task_queue.clone();

        let workers_result = self
            .store
            .workers()
            .list(kagzi_store::ListWorkersParams {
                namespace: namespace.clone(),
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
                    &namespace,
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

        Ok(Response::new(ListWorkersResponse {
            workers: proto_workers,
            page: Some(page_info),
        }))
    }

    async fn get_worker(
        &self,
        request: Request<GetWorkerRequest>,
    ) -> Result<Response<GetWorkerResponse>, Status> {
        let req = request.into_inner();
        let worker_id = Uuid::parse_str(&req.worker_id)
            .map_err(|_| invalid_argument_error("Invalid worker_id"))?;

        let worker = self
            .store
            .workers()
            .find_by_id(worker_id)
            .await
            .map_err(map_store_error)?;

        match worker {
            Some(w) => {
                let proto = worker_to_proto(w);
                Ok(Response::new(GetWorkerResponse {
                    worker: Some(proto),
                }))
            }
            None => Err(not_found_error("Worker not found", "worker", req.worker_id)),
        }
    }

    async fn get_step(
        &self,
        request: Request<GetStepRequest>,
    ) -> Result<Response<GetStepResponse>, Status> {
        let req = request.into_inner();
        let step_id =
            Uuid::parse_str(&req.step_id).map_err(|_| invalid_argument_error("Invalid step_id"))?;

        let step = self
            .store
            .steps()
            .find_by_id(step_id)
            .await
            .map_err(map_store_error)?;

        match step {
            Some(s) => {
                let proto = step_to_proto(s)?;
                Ok(Response::new(GetStepResponse { step: Some(proto) }))
            }
            None => Err(not_found_error("Step not found", "step", req.step_id)),
        }
    }

    async fn list_steps(
        &self,
        request: Request<ListStepsRequest>,
    ) -> Result<Response<ListStepsResponse>, Status> {
        let req = request.into_inner();
        let run_id =
            Uuid::parse_str(&req.run_id).map_err(|_| invalid_argument_error("Invalid run_id"))?;

        // Discover namespace from run_id to ensure proper isolation
        let namespace = self
            .store
            .workflows()
            .get_namespace(run_id)
            .await
            .map_err(map_store_error)?
            .ok_or_else(|| not_found_error("Workflow not found", "workflow", run_id.to_string()))?;

        let page = req.page.unwrap_or_default();
        let page_size = if page.page_size <= 0 {
            50
        } else if page.page_size > 100 {
            100
        } else {
            page.page_size
        };

        let cursor: Option<kagzi_store::StepCursor> = if page.page_token.is_empty() {
            None
        } else {
            let (created_at, attempt_id) = decode_cursor(&page.page_token)?;
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
                namespace,
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
                .map(|c| encode_cursor(c.created_at.timestamp_millis(), &c.attempt_id))
                .unwrap_or_default(),
            has_more: steps_result.has_more,
            total_count: 0,
        };

        Ok(Response::new(ListStepsResponse {
            steps: attempts,
            page: Some(page_info),
        }))
    }

    async fn health_check(
        &self,
        request: Request<HealthCheckRequest>,
    ) -> Result<Response<HealthCheckResponse>, Status> {
        let _req = request.into_inner();
        let db_status =
            match kagzi_store::HealthRepository::health_check(&self.store.health()).await {
                Ok(_) => ServingStatus::Serving,
                Err(_) => ServingStatus::NotServing,
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

        Ok(Response::new(response))
    }

    async fn get_server_info(
        &self,
        request: Request<GetServerInfoRequest>,
    ) -> Result<Response<GetServerInfoResponse>, Status> {
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

        Ok(Response::new(response))
    }

    async fn get_stats(
        &self,
        request: Request<GetStatsRequest>,
    ) -> Result<Response<GetStatsResponse>, Status> {
        let req = request.into_inner();

        // Use "default" namespace if not specified
        let namespace = req.namespace.unwrap_or_else(|| "default".to_string());

        let stats = self
            .store
            .namespaces()
            .get_stats(&namespace)
            .await
            .map_err(map_store_error)?;

        // Convert Vec<WorkflowStatusCount> to map
        let workflows_by_status: std::collections::HashMap<String, i64> = stats
            .workflows_by_status
            .into_iter()
            .map(|s| (s.status, s.count))
            .collect();

        // Convert Vec<WorkflowTypeCount> to map
        let workflows_by_type: std::collections::HashMap<String, i64> = stats
            .workflows_by_type
            .into_iter()
            .map(|t| (t.workflow_type, t.total_runs))
            .collect();

        Ok(Response::new(GetStatsResponse {
            total_workflows: stats.total_workflows,
            pending_workflows: stats.pending_workflows,
            running_workflows: stats.running_workflows,
            completed_workflows: stats.completed_workflows,
            failed_workflows: stats.failed_workflows,
            total_workers: stats.total_workers,
            online_workers: stats.online_workers,
            total_schedules: stats.total_schedules,
            enabled_schedules: stats.enabled_schedules,
            workflows_by_status,
            workflows_by_type,
        }))
    }

    async fn drain_worker(
        &self,
        request: Request<DrainWorkerRequest>,
    ) -> Result<Response<DrainWorkerResponse>, Status> {
        let req = request.into_inner();
        let worker_id = Uuid::parse_str(&req.worker_id)
            .map_err(|_| invalid_argument_error("Invalid worker_id"))?;

        let worker = self
            .store
            .workers()
            .find_by_id(worker_id)
            .await
            .map_err(map_store_error)?
            .ok_or_else(|| not_found_error("Worker not found", "worker", req.worker_id))?;

        self.store
            .workers()
            .start_drain(worker_id, &worker.namespace)
            .await
            .map_err(map_store_error)?;

        Ok(Response::new(DrainWorkerResponse {
            success: true,
            message: "Worker is now draining".to_string(),
            error: None,
        }))
    }

    async fn get_queue_depth(
        &self,
        request: Request<GetQueueDepthRequest>,
    ) -> Result<Response<GetQueueDepthResponse>, Status> {
        let req = request.into_inner();
        let namespace = require_non_empty(req.namespace, "namespace")?;

        let pending_count = self
            .store
            .workflows()
            .count(&namespace, Some("PENDING"))
            .await
            .map_err(map_store_error)?;
        let running_count = self
            .store
            .workflows()
            .count(&namespace, Some("RUNNING"))
            .await
            .map_err(map_store_error)?;
        let sleeping_count = self
            .store
            .workflows()
            .count(&namespace, Some("SLEEPING"))
            .await
            .map_err(map_store_error)?;

        Ok(Response::new(GetQueueDepthResponse {
            pending_count,
            running_count,
            sleeping_count,
            depth_by_queue: Default::default(),
        }))
    }

    async fn list_workflow_types(
        &self,
        request: Request<ListWorkflowTypesRequest>,
    ) -> Result<Response<ListWorkflowTypesResponse>, Status> {
        let req = request.into_inner();
        let namespace = require_non_empty(req.namespace, "namespace")?;

        let page_req = req
            .page
            .ok_or_else(|| invalid_argument_error("page is required"))?;
        let page_size = normalize_page_size(page_req.page_size, 20, 100);
        let cursor = if page_req.page_token.is_empty() {
            None
        } else {
            Some(page_req.page_token)
        };

        let result = self
            .store
            .namespaces()
            .list_workflow_types(&namespace, page_size, cursor)
            .await
            .map_err(map_store_error)?;

        let workflow_types: Vec<kagzi_proto::kagzi::WorkflowTypeInfo> = result
            .items
            .into_iter()
            .map(|info| kagzi_proto::kagzi::WorkflowTypeInfo {
                workflow_type: info.workflow_type,
                total_runs: info.total_runs,
                active_runs: info.active_runs,
                task_queues: info.task_queues,
            })
            .collect();

        Ok(Response::new(ListWorkflowTypesResponse {
            workflow_types,
            page: Some(PageInfo {
                next_page_token: result.next_cursor.unwrap_or_default(),
                has_more: result.has_more,
                total_count: 0,
            }),
        }))
    }
}
