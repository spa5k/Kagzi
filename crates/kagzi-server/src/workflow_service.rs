use kagzi_proto::kagzi::workflow_service_server::WorkflowService;
use kagzi_proto::kagzi::{
    CancelWorkflowRequest, CancelWorkflowResponse, GetWorkflowByExternalIdRequest,
    GetWorkflowByExternalIdResponse, GetWorkflowRequest, GetWorkflowResponse, ListWorkflowsRequest,
    ListWorkflowsResponse, PageInfo, RetryWorkflowRequest, RetryWorkflowResponse,
    StartWorkflowRequest, StartWorkflowResponse, TerminateWorkflowRequest,
    TerminateWorkflowResponse, WorkflowStatus,
};
use kagzi_queue::QueueNotifier;
use kagzi_store::{
    CreateWorkflow, ListWorkflowsParams, PgStore, WorkflowCursor, WorkflowRepository,
};
use tonic::{Request, Response, Status};
use tracing::instrument;
use tracing_opentelemetry::OpenTelemetrySpanExt;
use uuid::Uuid;

use crate::constants::DEFAULT_VERSION;
use crate::helpers::{
    decode_cursor, encode_cursor, invalid_argument_error, map_store_error, merge_proto_policy,
    normalize_page_size, not_found_error, payload_to_bytes, precondition_failed_error,
    require_non_empty,
};
use crate::proto_convert::{workflow_status_to_string, workflow_to_proto};
use crate::telemetry::extract_context;

pub struct WorkflowServiceImpl<Q: QueueNotifier = kagzi_queue::PostgresNotifier> {
    pub store: PgStore,
    pub queue: Q,
}

impl<Q: QueueNotifier> WorkflowServiceImpl<Q> {
    pub fn new(store: PgStore, queue: Q) -> Self {
        Self { store, queue }
    }
}

#[tonic::async_trait]
impl<Q: QueueNotifier + 'static> WorkflowService for WorkflowServiceImpl<Q> {
    #[instrument(
        skip(self, request),
        fields(
            workflow_type = tracing::field::Empty,
            external_id = tracing::field::Empty,
            namespace_id = tracing::field::Empty,
        )
    )]
    async fn start_workflow(
        &self,
        request: Request<StartWorkflowRequest>,
    ) -> Result<Response<StartWorkflowResponse>, Status> {
        // Extract parent trace context and set it as the parent of current span
        let parent_cx = extract_context(request.metadata());
        let _ = tracing::Span::current().set_parent(parent_cx);

        let req = request.into_inner();
        tracing::Span::current().record("workflow_type", &req.workflow_type);
        tracing::Span::current().record("external_id", &req.external_id);

        let external_id = require_non_empty(req.external_id, "external_id")?;
        let task_queue = require_non_empty(req.task_queue, "task_queue")?;
        let workflow_type = require_non_empty(req.workflow_type, "workflow_type")?;

        let input_bytes = payload_to_bytes(req.input);

        let namespace_id = require_non_empty(req.namespace_id, "namespace_id")?;
        tracing::Span::current().record("namespace_id", &namespace_id);

        let version = if req.version.is_empty() {
            DEFAULT_VERSION.to_string()
        } else {
            req.version
        };

        let workflows = self.store.workflows();

        // task_queue clone line removed - using task_queue.clone() in struct instead

        let create_result = workflows
            .create(CreateWorkflow {
                run_id: Uuid::now_v7(),
                external_id: external_id.clone(),
                task_queue: task_queue.clone(),
                workflow_type,
                input: input_bytes,
                namespace_id: namespace_id.clone(),
                version,
                retry_policy: merge_proto_policy(req.retry_policy, None),
                cron_expr: None,
                schedule_id: None,
            })
            .await;

        let (run_id, already_exists) = match create_result {
            Ok(id) => (id, false),
            Err(ref e) if e.is_unique_violation() => {
                let existing_id = workflows
                    .find_active_by_external_id(&namespace_id, &external_id)
                    .await
                    .map_err(map_store_error)?
                    .ok_or_else(|| map_store_error(create_result.unwrap_err()))?;
                (existing_id, true)
            }
            Err(e) => return Err(map_store_error(e)),
        };

        if !already_exists {
            let _ = self.queue.notify(&namespace_id, &task_queue).await;
        }

        Ok(Response::new(StartWorkflowResponse {
            run_id: run_id.to_string(),
            already_exists,
        }))
    }

    #[instrument(skip(self, request), fields(run_id = %request.get_ref().run_id))]
    async fn get_workflow(
        &self,
        request: Request<GetWorkflowRequest>,
    ) -> Result<Response<GetWorkflowResponse>, Status> {
        let req = request.into_inner();
        let run_id = uuid::Uuid::parse_str(&req.run_id)
            .map_err(|_| invalid_argument_error("Invalid run_id: must be a valid UUID"))?;

        let namespace_id = require_non_empty(req.namespace_id, "namespace_id")?;

        let workflow = self
            .store
            .workflows()
            .find_by_id(run_id, &namespace_id)
            .await
            .map_err(map_store_error)?;

        match workflow {
            Some(w) => {
                let proto = workflow_to_proto(w)?;
                Ok(Response::new(GetWorkflowResponse {
                    workflow: Some(proto),
                }))
            }
            None => Err(not_found_error(
                format!(
                    "Workflow not found: run_id={}, namespace_id={}",
                    run_id, namespace_id
                ),
                "workflow",
                run_id.to_string(),
            )),
        }
    }

    #[instrument(skip(self, request), fields(namespace_id = tracing::field::Empty))]
    async fn list_workflows(
        &self,
        request: Request<ListWorkflowsRequest>,
    ) -> Result<Response<ListWorkflowsResponse>, Status> {
        // removed base64 engine import

        let req = request.into_inner();

        let namespace_id = require_non_empty(req.namespace_id, "namespace_id")?;

        tracing::Span::current().record("namespace_id", &namespace_id);

        let page = req.page.unwrap_or_default();
        let page_size = normalize_page_size(page.page_size, 20, 100);

        let cursor: Option<WorkflowCursor> = if page.page_token.is_empty() {
            None
        } else {
            let (created_at, run_id) = decode_cursor(&page.page_token)?;
            Some(WorkflowCursor { created_at, run_id })
        };

        let filter_status = req
            .status_filter
            .map(WorkflowStatus::try_from)
            .transpose()
            .map_err(|_| invalid_argument_error("Invalid status_filter"))?
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
                schedule_id: None,
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
            .map(|c| encode_cursor(c.created_at.timestamp_millis(), &c.run_id))
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

        Ok(response)
    }

    #[instrument(skip(self, request), fields(run_id = %request.get_ref().run_id))]
    async fn cancel_workflow(
        &self,
        request: Request<CancelWorkflowRequest>,
    ) -> Result<Response<CancelWorkflowResponse>, Status> {
        let req = request.into_inner();

        let run_id = uuid::Uuid::parse_str(&req.run_id)
            .map_err(|_| invalid_argument_error("Invalid run_id: must be a valid UUID"))?;

        let namespace_id = require_non_empty(req.namespace_id, "namespace_id")?;

        let workflows = self.store.workflows();

        let cancelled = workflows
            .cancel(run_id, &namespace_id)
            .await
            .map_err(map_store_error)?;

        if cancelled {
            Ok(Response::new(CancelWorkflowResponse {
                cancelled: true,
                status: kagzi_proto::kagzi::WorkflowStatus::Cancelled as i32,
            }))
        } else {
            let exists = workflows
                .check_exists(run_id, &namespace_id)
                .await
                .map_err(map_store_error)?;

            if exists.exists {
                Err(precondition_failed_error(format!(
                    "Cannot cancel workflow with status '{:?}'. Only PENDING, RUNNING, or SLEEPING workflows can be cancelled.",
                    exists.status
                )))
            } else {
                Err(not_found_error(
                    format!(
                        "Workflow not found: run_id={}, namespace_id={}",
                        run_id, namespace_id
                    ),
                    "workflow",
                    run_id.to_string(),
                ))
            }
        }
    }

    #[instrument(skip(self, request), fields(external_id = %request.get_ref().external_id))]
    async fn get_workflow_by_external_id(
        &self,
        request: Request<GetWorkflowByExternalIdRequest>,
    ) -> Result<Response<GetWorkflowByExternalIdResponse>, Status> {
        let req = request.into_inner();
        let external_id = require_non_empty(req.external_id, "external_id")?;
        let namespace_id = require_non_empty(req.namespace_id, "namespace_id")?;

        let run_id = self
            .store
            .workflows()
            .find_active_by_external_id(&namespace_id, &external_id)
            .await
            .map_err(map_store_error)?
            .ok_or_else(|| not_found_error("Workflow not found", "workflow", external_id))?;

        let workflow = self
            .store
            .workflows()
            .find_by_id(run_id, &namespace_id)
            .await
            .map_err(map_store_error)?
            .ok_or_else(|| not_found_error("Workflow not found", "workflow", run_id.to_string()))?;

        let proto = workflow_to_proto(workflow)?;

        Ok(Response::new(GetWorkflowByExternalIdResponse {
            workflow: Some(proto),
        }))
    }

    #[instrument(skip(self, request), fields(run_id = %request.get_ref().run_id))]
    async fn retry_workflow(
        &self,
        request: Request<RetryWorkflowRequest>,
    ) -> Result<Response<RetryWorkflowResponse>, Status> {
        let req = request.into_inner();
        let run_id = uuid::Uuid::parse_str(&req.run_id)
            .map_err(|_| invalid_argument_error("Invalid run_id: must be a valid UUID"))?;

        let namespace_id = require_non_empty(req.namespace_id, "namespace_id")?;

        let workflow = self
            .store
            .workflows()
            .find_by_id(run_id, &namespace_id)
            .await
            .map_err(map_store_error)?
            .ok_or_else(|| not_found_error("Workflow not found", "workflow", run_id.to_string()))?;

        // Check if workflow is still running
        if !workflow.status.is_terminal() {
            return Ok(Response::new(RetryWorkflowResponse {
                new_run_id: String::new(),
                already_running: true,
            }));
        }

        // Create a new workflow run with the same input
        let new_run_id = self
            .store
            .workflows()
            .create(CreateWorkflow {
                run_id: Uuid::now_v7(),
                external_id: format!("retry-{}", workflow.external_id),
                task_queue: workflow.task_queue.clone(),
                workflow_type: workflow.workflow_type.clone(),
                input: workflow.input.clone(),
                namespace_id: namespace_id.clone(),
                version: workflow.version.clone().unwrap_or_default(),
                retry_policy: None,
                cron_expr: None,
                schedule_id: None,
            })
            .await
            .map_err(map_store_error)?;

        Ok(Response::new(RetryWorkflowResponse {
            new_run_id: new_run_id.to_string(),
            already_running: false,
        }))
    }

    #[instrument(skip(self, request), fields(run_id = %request.get_ref().run_id))]
    async fn terminate_workflow(
        &self,
        request: Request<TerminateWorkflowRequest>,
    ) -> Result<Response<TerminateWorkflowResponse>, Status> {
        let req = request.into_inner();
        let run_id = uuid::Uuid::parse_str(&req.run_id)
            .map_err(|_| invalid_argument_error("Invalid run_id: must be a valid UUID"))?;

        let _namespace_id = require_non_empty(req.namespace_id, "namespace_id")?;

        let workflows = self.store.workflows();

        // Fail the workflow with the termination reason
        workflows
            .fail(run_id, &req.reason)
            .await
            .map_err(map_store_error)?;

        Ok(Response::new(TerminateWorkflowResponse {
            terminated: true,
            status: kagzi_proto::kagzi::WorkflowStatus::Failed as i32,
        }))
    }
}
