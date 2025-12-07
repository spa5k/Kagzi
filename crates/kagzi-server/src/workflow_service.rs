use crate::tracing_utils::{
    extract_or_generate_correlation_id, extract_or_generate_trace_id, log_grpc_request,
    log_grpc_response,
};
use kagzi_proto::kagzi::workflow_service_server::WorkflowService;
use kagzi_proto::kagzi::{
    CancelWorkflowRequest, ErrorCode, ErrorDetail, GetWorkflowRequest, GetWorkflowResponse,
    ListWorkflowsRequest, ListWorkflowsResponse, PageInfo, Payload, RetryPolicy,
    StartWorkflowRequest, StartWorkflowResponse, Workflow, WorkflowStatus,
};
use kagzi_store::{
    CreateWorkflow, ListWorkflowsParams, PgStore, WorkflowCursor, WorkflowRepository,
};
use prost::Message;
use std::collections::HashMap;
use tonic::{Code, Request, Response, Status};
use tracing::{info, instrument};

pub struct WorkflowServiceImpl {
    pub store: PgStore,
}

impl WorkflowServiceImpl {
    pub fn new(store: PgStore) -> Self {
        Self { store }
    }
}

fn payload_to_json(payload: Option<Payload>) -> Result<serde_json::Value, Status> {
    match payload {
        None => Ok(serde_json::json!(null)),
        Some(p) => {
            if p.data.is_empty() {
                Ok(serde_json::json!(null))
            } else {
                serde_json::from_slice(&p.data)
                    .map_err(|e| invalid_argument(format!("Payload must be valid JSON: {}", e)))
            }
        }
    }
}

fn payload_to_optional_json(payload: Option<Payload>) -> Result<Option<serde_json::Value>, Status> {
    match payload {
        None => Ok(None),
        Some(p) if p.data.is_empty() => Ok(None),
        Some(p) => Ok(Some(serde_json::from_slice(&p.data).map_err(|e| {
            invalid_argument(format!("Payload must be valid JSON: {}", e))
        })?)),
    }
}

fn json_to_payload(value: Option<serde_json::Value>) -> Result<Payload, Status> {
    match value {
        None => Ok(Payload {
            data: Vec::new(),
            metadata: HashMap::new(),
        }),
        Some(v) => {
            let data = serde_json::to_vec(&v)
                .map_err(|e| internal(format!("Failed to serialize payload: {}", e)))?;
            Ok(Payload {
                data,
                metadata: HashMap::new(),
            })
        }
    }
}

fn workflow_to_proto(w: kagzi_store::WorkflowRun) -> Result<Workflow, Status> {
    Ok(Workflow {
        run_id: w.run_id.to_string(),
        workflow_id: w.business_id,
        namespace_id: w.namespace_id,
        task_queue: w.task_queue,
        workflow_type: w.workflow_type,
        status: map_workflow_status(w.status) as i32,
        input: Some(json_to_payload(Some(w.input))?),
        output: Some(json_to_payload(w.output)?),
        context: Some(json_to_payload(w.context)?),
        error: Some(string_error_detail(w.error)),
        attempts: w.attempts,
        created_at: w.created_at.map(timestamp_from),
        started_at: w.started_at.map(timestamp_from),
        finished_at: w.finished_at.map(timestamp_from),
        wake_up_at: w.wake_up_at.map(timestamp_from),
        deadline_at: w.deadline_at.map(timestamp_from),
        worker_id: w.locked_by.unwrap_or_default(),
        version: w.version.unwrap_or_default(),
        parent_step_id: w.parent_step_attempt_id.unwrap_or_default(),
    })
}

fn timestamp_from(dt: chrono::DateTime<chrono::Utc>) -> prost_types::Timestamp {
    prost_types::Timestamp {
        seconds: dt.timestamp(),
        nanos: dt.timestamp_subsec_nanos() as i32,
    }
}

fn merge_proto_policy(
    proto: Option<RetryPolicy>,
    fallback: Option<&kagzi_store::RetryPolicy>,
) -> Option<kagzi_store::RetryPolicy> {
    match (proto, fallback.cloned()) {
        (None, None) => None,
        (None, Some(base)) => Some(base),
        (Some(p), Some(mut base)) => {
            if p.maximum_attempts != 0 {
                base.maximum_attempts = p.maximum_attempts;
            }
            if p.initial_interval_ms != 0 {
                base.initial_interval_ms = p.initial_interval_ms;
            }
            if p.backoff_coefficient != 0.0 {
                base.backoff_coefficient = p.backoff_coefficient;
            }
            if p.maximum_interval_ms != 0 {
                base.maximum_interval_ms = p.maximum_interval_ms;
            }
            if !p.non_retryable_errors.is_empty() {
                base.non_retryable_errors = p.non_retryable_errors;
            }
            Some(base)
        }
        (Some(p), None) => Some(kagzi_store::RetryPolicy {
            maximum_attempts: if p.maximum_attempts == 0 {
                5
            } else {
                p.maximum_attempts
            },
            initial_interval_ms: if p.initial_interval_ms == 0 {
                1000
            } else {
                p.initial_interval_ms
            },
            backoff_coefficient: if p.backoff_coefficient == 0.0 {
                2.0
            } else {
                p.backoff_coefficient
            },
            maximum_interval_ms: if p.maximum_interval_ms == 0 {
                60000
            } else {
                p.maximum_interval_ms
            },
            non_retryable_errors: p.non_retryable_errors,
        }),
    }
}

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

fn workflow_status_to_string(status: WorkflowStatus) -> String {
    match status {
        WorkflowStatus::Pending => "PENDING",
        WorkflowStatus::Running => "RUNNING",
        WorkflowStatus::Sleeping => "SLEEPING",
        WorkflowStatus::Completed => "COMPLETED",
        WorkflowStatus::Failed => "FAILED",
        WorkflowStatus::Cancelled => "CANCELLED",
        WorkflowStatus::Unspecified => "UNSPECIFIED",
    }
    .to_string()
}

fn detail(
    code: ErrorCode,
    message: impl Into<String>,
    non_retryable: bool,
    retry_after_ms: i64,
    subject: impl Into<String>,
    subject_id: impl Into<String>,
) -> ErrorDetail {
    ErrorDetail {
        code: code as i32,
        message: message.into(),
        non_retryable,
        retry_after_ms,
        subject: subject.into(),
        subject_id: subject_id.into(),
        metadata: HashMap::new(),
    }
}

fn status_with_detail(code: Code, detail: ErrorDetail) -> Status {
    Status::with_details(code, detail.message.clone(), detail.encode_to_vec().into())
}

fn invalid_argument(message: impl Into<String>) -> Status {
    status_with_detail(
        Code::InvalidArgument,
        detail(ErrorCode::InvalidArgument, message, true, 0, "", ""),
    )
}

fn not_found(
    message: impl Into<String>,
    subject: impl Into<String>,
    id: impl Into<String>,
) -> Status {
    status_with_detail(
        Code::NotFound,
        detail(ErrorCode::NotFound, message, true, 0, subject, id),
    )
}

fn precondition_failed(message: impl Into<String>) -> Status {
    status_with_detail(
        Code::FailedPrecondition,
        detail(ErrorCode::PreconditionFailed, message, true, 0, "", ""),
    )
}

fn conflict(message: impl Into<String>) -> Status {
    status_with_detail(
        Code::Aborted,
        detail(ErrorCode::Conflict, message, false, 0, "", ""),
    )
}

fn unavailable(message: impl Into<String>) -> Status {
    status_with_detail(
        Code::Unavailable,
        detail(ErrorCode::Unavailable, message, false, 0, "", ""),
    )
}

fn deadline_exceeded(message: impl Into<String>) -> Status {
    status_with_detail(
        Code::DeadlineExceeded,
        detail(ErrorCode::Unavailable, message, false, 0, "", ""),
    )
}

fn permission_denied(message: impl Into<String>) -> Status {
    status_with_detail(
        Code::PermissionDenied,
        detail(ErrorCode::Unauthorized, message, true, 0, "", ""),
    )
}

fn internal(message: impl Into<String>) -> Status {
    status_with_detail(
        Code::Internal,
        detail(ErrorCode::Internal, message, true, 0, "", ""),
    )
}

fn string_error_detail(message: Option<String>) -> ErrorDetail {
    detail(
        ErrorCode::Unspecified,
        message.unwrap_or_default(),
        false,
        0,
        "",
        "",
    )
}

fn map_store_error(e: kagzi_store::StoreError) -> Status {
    match e {
        kagzi_store::StoreError::NotFound { entity, id } => {
            not_found(format!("{} not found", entity), entity, id)
        }
        kagzi_store::StoreError::InvalidArgument { message } => invalid_argument(message),
        kagzi_store::StoreError::InvalidState { message } => precondition_failed(message),
        kagzi_store::StoreError::AlreadyCompleted { message } => precondition_failed(message),
        kagzi_store::StoreError::LockConflict { message } => conflict(message),
        kagzi_store::StoreError::PreconditionFailed { message } => precondition_failed(message),
        kagzi_store::StoreError::Unauthorized { message } => permission_denied(message),
        kagzi_store::StoreError::Unavailable { message } => unavailable(message),
        kagzi_store::StoreError::Timeout { message } => deadline_exceeded(message),
        kagzi_store::StoreError::Database(e) => {
            tracing::error!("Database error: {:?}", e);
            internal("Database error")
        }
        kagzi_store::StoreError::Serialization(e) => {
            tracing::error!("Serialization error: {:?}", e);
            internal("Serialization error")
        }
    }
}

#[tonic::async_trait]
impl WorkflowService for WorkflowServiceImpl {
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

        if req.workflow_id.is_empty() {
            return Err(invalid_argument("workflow_id is required"));
        }

        let input_json = payload_to_json(req.input)?;
        let context_json = payload_to_optional_json(req.context)?;

        let namespace_id = if req.namespace_id.is_empty() {
            "default".to_string()
        } else {
            req.namespace_id
        };

        let workflows = self.store.workflows();

        if let Some(existing_id) = workflows
            .find_by_idempotency_key(&namespace_id, &req.workflow_id)
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

        let run_id = workflows
            .create(CreateWorkflow {
                business_id: req.workflow_id.clone(),
                task_queue: req.task_queue,
                workflow_type: req.workflow_type,
                input: input_json,
                namespace_id,
                idempotency_key: Some(req.workflow_id),
                context: context_json,
                deadline_at: req.deadline_at.map(|ts| {
                    chrono::DateTime::from_timestamp(ts.seconds, ts.nanos as u32)
                        .unwrap_or_default()
                }),
                version,
                retry_policy: merge_proto_policy(req.retry_policy, None),
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

        let result = self
            .store
            .workflows()
            .list(ListWorkflowsParams {
                namespace_id,
                filter_status,
                page_size,
                cursor,
            })
            .await
            .map_err(map_store_error)?;

        let next_page_token = result
            .next_cursor
            .map(|c| {
                let cursor_str = format!("{}:{}", c.created_at.timestamp_millis(), c.run_id);
                base64::engine::general_purpose::STANDARD.encode(cursor_str.as_bytes())
            })
            .unwrap_or_default();

        let workflows: Result<Vec<_>, Status> = result
            .workflows
            .into_iter()
            .map(workflow_to_proto)
            .collect();
        let workflows = workflows?;

        let response = Response::new(ListWorkflowsResponse {
            workflows,
            page: Some(PageInfo {
                next_page_token,
                has_more: result.has_more,
                total_count: 0, // not available in current store implementation
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
