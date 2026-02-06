use kagzi_proto::kagzi::workflow_service_server::WorkflowService;
use kagzi_proto::kagzi::{
    CancelWorkflowRequest, CancelWorkflowResponse, GetWorkflowByExternalIdRequest,
    GetWorkflowByExternalIdResponse, GetWorkflowRequest, GetWorkflowResponse, ListWorkflowsRequest,
    ListWorkflowsResponse, PageInfo, RetryWorkflowRequest, RetryWorkflowResponse,
    StartWorkflowRequest, StartWorkflowResponse, TerminateWorkflowRequest,
    TerminateWorkflowResponse, WorkflowStatus,
};
use kagzi_queue::WorkSignalBus;
use kagzi_store::repository::NamespaceRepository;
use kagzi_store::{
    CreateWorkflow, ListWorkflowsParams, PgStore, StepRepository, WorkflowCursor,
    WorkflowRepository,
};
use tonic::{Request, Response, Status};
use tracing::{instrument, warn};
use tracing_opentelemetry::OpenTelemetrySpanExt;
use uuid::Uuid;

use crate::constants::DEFAULT_VERSION;
use crate::helpers::{
    decode_cursor, encode_cursor, invalid_argument_error, map_store_error, merge_proto_policy,
    normalize_page_size, not_found_error, payload_to_bytes, precondition_failed_error,
    require_non_empty, resolve_task_queue,
};
use crate::proto_convert::{workflow_status_to_string, workflow_to_proto};
use crate::queue_store::ensure_task_queue_exists;
use crate::telemetry::extract_context;
use crate::telemetry_store;

fn set_parent_from_metadata(metadata: &tonic::metadata::MetadataMap) {
    let parent_cx = extract_context(metadata);
    let _ = tracing::Span::current().set_parent(parent_cx);
}

pub struct WorkflowServiceImpl<Q: WorkSignalBus = kagzi_queue::PostgresNotifier> {
    pub store: PgStore,
    pub queue: Q,
    pub telemetry_enabled: bool,
}

impl<Q: WorkSignalBus> WorkflowServiceImpl<Q> {
    pub fn new(store: PgStore, queue: Q, telemetry_enabled: bool) -> Self {
        Self {
            store,
            queue,
            telemetry_enabled,
        }
    }

    async fn publish_wakeup(&self, namespace: &str, task_queue: &str) {
        match self.queue.publish(namespace, task_queue).await {
            Ok(_) => {
                telemetry_store::record_queue_publish_result(
                    &self.store,
                    self.telemetry_enabled,
                    namespace,
                    task_queue,
                    true,
                    None,
                )
                .await;
            }
            Err(e) => {
                telemetry_store::record_queue_publish_result(
                    &self.store,
                    self.telemetry_enabled,
                    namespace,
                    task_queue,
                    false,
                    Some(&format!("{e:?}")),
                )
                .await;
                tracing::warn!(
                    error = ?e,
                    namespace = %namespace,
                    task_queue = %task_queue,
                    "Failed to publish work wakeup"
                );
            }
        }
    }
}

#[tonic::async_trait]
impl<Q: WorkSignalBus + 'static> WorkflowService for WorkflowServiceImpl<Q> {
    #[instrument(
        skip(self, request),
        fields(
            workflow_type = tracing::field::Empty,
            external_id = tracing::field::Empty,
            namespace = tracing::field::Empty,
        )
    )]
    async fn start_workflow(
        &self,
        request: Request<StartWorkflowRequest>,
    ) -> Result<Response<StartWorkflowResponse>, Status> {
        set_parent_from_metadata(request.metadata());

        let req = request.into_inner();
        tracing::Span::current().record("workflow_type", &req.workflow_type);
        tracing::Span::current().record("external_id", &req.external_id);

        let external_id = require_non_empty(req.external_id, "external_id")?;
        let task_queue = resolve_task_queue(req.task_queue);
        let workflow_type = require_non_empty(req.workflow_type, "workflow_type")?;

        let input_bytes = payload_to_bytes(req.input);

        let namespace = require_non_empty(req.namespace, "namespace")?;
        tracing::Span::current().record("namespace", &namespace);

        // Ensure namespace exists (auto-create if it doesn't)
        self.store
            .namespaces()
            .get_or_create(&namespace)
            .await
            .map_err(map_store_error)?;

        // Best-effort: ensure queue is registered for UI/governance.
        let _ = ensure_task_queue_exists(&self.store, &namespace, &task_queue).await;

        let version = if req.version.is_empty() {
            DEFAULT_VERSION.to_string()
        } else {
            req.version
        };

        let workflows = self.store.workflows();

        let (run_id, already_exists) = match workflows
            .create(CreateWorkflow {
                run_id: Uuid::now_v7(),
                external_id: external_id.clone(),
                task_queue: task_queue.clone(),
                workflow_type,
                input: input_bytes,
                namespace: namespace.clone(),
                version,
                retry_policy: merge_proto_policy(req.retry_policy, None),
                cron_expr: None,
                schedule_id: None,
            })
            .await
        {
            Ok(id) => (id, false),
            Err(e) if e.is_unique_violation() => match workflows
                .find_active_by_external_id(&namespace, &external_id)
                .await
                .map_err(map_store_error)?
            {
                Some(existing_id) => (existing_id, true),
                None => return Err(map_store_error(e)),
            },
            Err(e) => return Err(map_store_error(e)),
        };

        if !already_exists {
            self.publish_wakeup(&namespace, &task_queue).await;
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

        let namespace = require_non_empty(req.namespace, "namespace")?;

        let workflow = self
            .store
            .workflows()
            .find_by_id(run_id, &namespace)
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
                    "Workflow not found: run_id={}, namespace={}",
                    run_id, namespace
                ),
                "workflow",
                run_id.to_string(),
            )),
        }
    }

    #[instrument(skip(self, request), fields(namespace = tracing::field::Empty))]
    async fn list_workflows(
        &self,
        request: Request<ListWorkflowsRequest>,
    ) -> Result<Response<ListWorkflowsResponse>, Status> {
        // removed base64 engine import

        let req = request.into_inner();

        let namespace = require_non_empty(req.namespace, "namespace")?;

        tracing::Span::current().record("namespace", &namespace);

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
            .and_then(|s| {
                (s != WorkflowStatus::Unspecified).then_some(workflow_status_to_string(s))
            });

        let result = self
            .store
            .workflows()
            .list(ListWorkflowsParams {
                namespace: namespace.clone(),
                filter_status: filter_status.clone(),
                page_size,
                cursor,
                schedule_id: None,
            })
            .await
            .map_err(map_store_error)?;

        let total_count = if page.include_total_count {
            self.store
                .workflows()
                .count(&namespace, filter_status.as_deref())
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

        let namespace = require_non_empty(req.namespace, "namespace")?;

        let workflows = self.store.workflows();

        let cancelled = workflows
            .cancel(run_id, &namespace)
            .await
            .map_err(map_store_error)?;

        if cancelled {
            if let Err(err) = self
                .store
                .steps()
                .record_lifecycle_event(run_id, kagzi_store::StepKind::WorkflowCancelled, None)
                .await
            {
                warn!(
                    run_id = %run_id,
                    error = %err,
                    "Failed to record WorkflowCancelled lifecycle event"
                );
            }

            Ok(Response::new(CancelWorkflowResponse {
                cancelled: true,
                status: kagzi_proto::kagzi::WorkflowStatus::Cancelled as i32,
            }))
        } else {
            let exists = workflows
                .check_exists(run_id, &namespace)
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
                        "Workflow not found: run_id={}, namespace={}",
                        run_id, namespace
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
        let namespace = require_non_empty(req.namespace, "namespace")?;

        let run_id = self
            .store
            .workflows()
            .find_active_by_external_id(&namespace, &external_id)
            .await
            .map_err(map_store_error)?
            .ok_or_else(|| not_found_error("Workflow not found", "workflow", external_id))?;

        let workflow = self
            .store
            .workflows()
            .find_by_id(run_id, &namespace)
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

        let namespace = require_non_empty(req.namespace, "namespace")?;

        let workflow = self
            .store
            .workflows()
            .find_by_id(run_id, &namespace)
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
        let new_run_id = Uuid::now_v7();
        self.store
            .workflows()
            .create(CreateWorkflow {
                run_id: new_run_id,
                external_id: format!("retry:{}:{new_run_id}", workflow.external_id),
                task_queue: workflow.task_queue.clone(),
                workflow_type: workflow.workflow_type.clone(),
                input: workflow.input.clone(),
                namespace: namespace.clone(),
                version: workflow.version.clone().unwrap_or_default(),
                retry_policy: None,
                cron_expr: None,
                schedule_id: None,
            })
            .await
            .map_err(map_store_error)?;

        self.publish_wakeup(&namespace, &workflow.task_queue).await;

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
        set_parent_from_metadata(request.metadata());

        let req = request.into_inner();
        let run_id = uuid::Uuid::parse_str(&req.run_id)
            .map_err(|_| invalid_argument_error("Invalid run_id: must be a valid UUID"))?;

        let namespace = require_non_empty(req.namespace, "namespace")?;
        let workflows = self.store.workflows();

        let exists = workflows
            .check_exists(run_id, &namespace)
            .await
            .map_err(map_store_error)?;
        if !exists.exists {
            return Err(not_found_error(
                format!(
                    "Workflow not found: run_id={}, namespace={}",
                    run_id, namespace
                ),
                "workflow",
                run_id.to_string(),
            ));
        }

        // Fail the workflow with the termination reason
        workflows
            .fail(run_id, &req.reason)
            .await
            .map_err(map_store_error)?;

        if let Err(err) = self
            .store
            .steps()
            .record_lifecycle_event(
                run_id,
                kagzi_store::StepKind::WorkflowFailed,
                Some(req.reason.clone().into_bytes()),
            )
            .await
        {
            warn!(
                run_id = %run_id,
                error = %err,
                "Failed to record WorkflowFailed lifecycle event"
            );
        }

        Ok(Response::new(TerminateWorkflowResponse {
            terminated: true,
            status: kagzi_proto::kagzi::WorkflowStatus::Failed as i32,
        }))
    }
}
