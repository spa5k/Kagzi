use kagzi_proto::kagzi::workflow_service_server::WorkflowService;
use kagzi_proto::kagzi::{
    CancelWorkflowRequest, GetWorkflowRequest, GetWorkflowResponse, ListWorkflowsRequest,
    ListWorkflowsResponse, PageInfo, StartWorkflowRequest, StartWorkflowResponse, WorkflowStatus,
};
use kagzi_store::{
    CreateWorkflow, ListWorkflowsParams, PgStore, WorkflowCursor, WorkflowRepository,
};
use tonic::{Request, Response, Status};
use tracing::{info, instrument};

use crate::helpers::{
    invalid_argument, map_store_error, merge_proto_policy, not_found, payload_to_bytes,
    payload_to_optional_json, precondition_failed,
};
use crate::proto_convert::{workflow_status_to_string, workflow_to_proto};
use crate::tracing_utils::{
    extract_or_generate_correlation_id, extract_or_generate_trace_id, log_grpc_request,
    log_grpc_response,
};

pub struct WorkflowServiceImpl {
    pub store: PgStore,
}

impl WorkflowServiceImpl {
    pub fn new(store: PgStore) -> Self {
        Self { store }
    }
}

#[tonic::async_trait]
impl WorkflowService for WorkflowServiceImpl {
    #[instrument(skip(self), fields(
        correlation_id = %extract_or_generate_correlation_id(&request),
        trace_id = %extract_or_generate_trace_id(&request),
        external_id = %request.get_ref().external_id,
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

        if req.external_id.is_empty() {
            return Err(invalid_argument("external_id is required"));
        }
        if req.task_queue.is_empty() {
            return Err(invalid_argument("task_queue is required"));
        }
        if req.workflow_type.is_empty() {
            return Err(invalid_argument("workflow_type is required"));
        }

        let input_bytes = payload_to_bytes(req.input);
        let context_json = payload_to_optional_json(req.context)?;

        let namespace_id = if req.namespace_id.is_empty() {
            "default".to_string()
        } else {
            req.namespace_id
        };

        let workflows = self.store.workflows();

        // Fast path: check if workflow already exists
        if let Some(existing_id) = workflows
            .find_active_by_external_id(&namespace_id, &req.external_id, None)
            .await
            .map_err(map_store_error)?
        {
            return Ok(Response::new(StartWorkflowResponse {
                run_id: existing_id.to_string(),
                already_exists: true,
            }));
        }

        let version = if req.version.is_empty() {
            "1".to_string()
        } else {
            req.version
        };

        // Create workflow - handle race condition where concurrent request created it
        let create_result = workflows
            .create(CreateWorkflow {
                external_id: req.external_id.clone(),
                task_queue: req.task_queue,
                workflow_type: req.workflow_type,
                input: input_bytes,
                namespace_id: namespace_id.clone(),
                idempotency_suffix: None,
                context: context_json,
                deadline_at: req
                    .deadline_at
                    .map(|ts| {
                        chrono::DateTime::from_timestamp(ts.seconds, ts.nanos as u32)
                            .ok_or_else(|| invalid_argument("Invalid deadline_at timestamp"))
                    })
                    .transpose()?,
                version,
                retry_policy: merge_proto_policy(req.retry_policy, None),
            })
            .await;

        // Handle unique constraint violation (concurrent idempotent request)
        let run_id = match create_result {
            Ok(id) => id,
            Err(ref e) if e.is_unique_violation() => {
                // Race condition: another request created the workflow between our check and insert
                if let Some(existing_id) = workflows
                    .find_active_by_external_id(&namespace_id, &req.external_id, None)
                    .await
                    .map_err(map_store_error)?
                {
                    return Ok(Response::new(StartWorkflowResponse {
                        run_id: existing_id.to_string(),
                        already_exists: true,
                    }));
                }
                // Should not happen, but fall through to error
                return Err(map_store_error(create_result.unwrap_err()));
            }
            Err(e) => return Err(map_store_error(e)),
        };

        log_grpc_response(
            "StartWorkflow",
            &correlation_id,
            &trace_id,
            Status::code(&Status::ok("")),
            None,
        );

        Ok(Response::new(StartWorkflowResponse {
            run_id: run_id.to_string(),
            already_exists: false,
        }))
    }

    #[instrument(skip(self), fields(
        correlation_id = %extract_or_generate_correlation_id(&request),
        trace_id = %extract_or_generate_trace_id(&request),
        run_id = %request.get_ref().run_id,
        namespace_id = %request.get_ref().namespace_id
    ))]
    async fn get_workflow(
        &self,
        request: Request<GetWorkflowRequest>,
    ) -> Result<Response<GetWorkflowResponse>, Status> {
        let correlation_id = extract_or_generate_correlation_id(&request);
        let trace_id = extract_or_generate_trace_id(&request);

        log_grpc_request("GetWorkflow", &correlation_id, &trace_id, None);

        let req = request.into_inner();
        let run_id = uuid::Uuid::parse_str(&req.run_id)
            .map_err(|_| invalid_argument("Invalid run_id: must be a valid UUID"))?;

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
                    "GetWorkflow",
                    &correlation_id,
                    &trace_id,
                    Status::code(&Status::ok("")),
                    None,
                );

                Ok(Response::new(GetWorkflowResponse {
                    workflow: Some(proto),
                }))
            }
            None => {
                let status = not_found(
                    format!(
                        "Workflow not found: run_id={}, namespace_id={}",
                        run_id, namespace_id
                    ),
                    "workflow",
                    run_id.to_string(),
                );

                log_grpc_response(
                    "GetWorkflow",
                    &correlation_id,
                    &trace_id,
                    Status::code(&status),
                    Some("Workflow not found"),
                );

                Err(status)
            }
        }
    }

    #[instrument(skip(self), fields(
        correlation_id = %extract_or_generate_correlation_id(&request),
        trace_id = %extract_or_generate_trace_id(&request),
        namespace_id = %request.get_ref().namespace_id
    ))]
    async fn list_workflows(
        &self,
        request: Request<ListWorkflowsRequest>,
    ) -> Result<Response<ListWorkflowsResponse>, Status> {
        use base64::Engine;

        let correlation_id = extract_or_generate_correlation_id(&request);
        let trace_id = extract_or_generate_trace_id(&request);

        log_grpc_request("ListWorkflows", &correlation_id, &trace_id, None);

        let req = request.into_inner();

        let namespace_id = if req.namespace_id.is_empty() {
            "default".to_string()
        } else {
            req.namespace_id
        };

        let page = req.page.unwrap_or_default();
        let page_size = if page.page_size <= 0 {
            20
        } else if page.page_size > 100 {
            100
        } else {
            page.page_size
        };

        let cursor: Option<WorkflowCursor> = if page.page_token.is_empty() {
            None
        } else {
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(&page.page_token)
                .map_err(|_| invalid_argument("Invalid page_token"))?;
            let cursor_str = String::from_utf8(decoded)
                .map_err(|_| invalid_argument("Invalid page_token encoding"))?;
            let parts: Vec<&str> = cursor_str.splitn(2, ':').collect();
            if parts.len() != 2 {
                return Err(invalid_argument("Invalid page_token format"));
            }
            let millis: i64 = parts[0]
                .parse()
                .map_err(|_| invalid_argument("Invalid page_token timestamp"))?;
            let run_id = uuid::Uuid::parse_str(parts[1])
                .map_err(|_| invalid_argument("Invalid page_token run_id"))?;
            let cursor_time = chrono::DateTime::from_timestamp_millis(millis)
                .ok_or_else(|| invalid_argument("Invalid cursor timestamp"))?;
            Some(WorkflowCursor {
                created_at: cursor_time,
                run_id,
            })
        };

        let filter_status = req
            .status_filter
            .map(WorkflowStatus::try_from)
            .transpose()
            .map_err(|_| invalid_argument("Invalid status_filter"))?
            .map(workflow_status_to_string);

        let filter_status_for_list = filter_status.clone();

        let namespace_for_list = namespace_id.clone();

        let result = self
            .store
            .workflows()
            .list(ListWorkflowsParams {
                namespace_id: namespace_for_list,
                filter_status: filter_status_for_list,
                page_size,
                cursor,
            })
            .await
            .map_err(map_store_error)?;

        let total_count = if page.include_total_count {
            self.store
                .workflows()
                .count(&namespace_id, filter_status.as_deref())
                .await
                .map_err(map_store_error)?
        } else {
            0
        };

        let next_page_token = result
            .next_cursor
            .map(|c| {
                let cursor_str = format!("{}:{}", c.created_at.timestamp_millis(), c.run_id);
                base64::engine::general_purpose::STANDARD.encode(cursor_str.as_bytes())
            })
            .unwrap_or_default();

        let workflows: Result<Vec<_>, Status> =
            result.items.into_iter().map(workflow_to_proto).collect();
        let workflows = workflows?;

        let response = Response::new(ListWorkflowsResponse {
            workflows,
            page: Some(PageInfo {
                next_page_token,
                has_more: result.has_more,
                total_count,
            }),
        });

        log_grpc_response(
            "ListWorkflows",
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
    async fn cancel_workflow(
        &self,
        request: Request<CancelWorkflowRequest>,
    ) -> Result<Response<()>, Status> {
        let correlation_id = extract_or_generate_correlation_id(&request);
        let trace_id = extract_or_generate_trace_id(&request);

        log_grpc_request("CancelWorkflow", &correlation_id, &trace_id, None);

        let req = request.into_inner();

        let run_id = uuid::Uuid::parse_str(&req.run_id)
            .map_err(|_| invalid_argument("Invalid run_id: must be a valid UUID"))?;

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
                "CancelWorkflow",
                &correlation_id,
                &trace_id,
                Status::code(&Status::ok("")),
                None,
            );

            Ok(Response::new(()))
        } else {
            let exists = workflows
                .check_exists(run_id, &namespace_id)
                .await
                .map_err(map_store_error)?;

            let status = if exists.exists {
                precondition_failed(format!(
                    "Cannot cancel workflow with status '{:?}'. Only PENDING, RUNNING, or SLEEPING workflows can be cancelled.",
                    exists.status
                ))
            } else {
                not_found(
                    format!(
                        "Workflow not found: run_id={}, namespace_id={}",
                        run_id, namespace_id
                    ),
                    "workflow",
                    run_id.to_string(),
                )
            };

            log_grpc_response(
                "CancelWorkflow",
                &correlation_id,
                &trace_id,
                Status::code(&status),
                Some("Workflow cancellation failed"),
            );

            Err(status)
        }
    }
}
