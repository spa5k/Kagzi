use crate::tracing_utils::{
    extract_or_generate_correlation_id, extract_or_generate_trace_id, log_grpc_request,
    log_grpc_response,
};
use crate::work_distributor::WorkDistributorHandle;
use cron::Schedule as CronSchedule;
use kagzi_proto::kagzi::workflow_service_server::WorkflowService;
use kagzi_proto::kagzi::{
    BeginStepRequest, BeginStepResponse, CancelWorkflowRunRequest, CompleteStepRequest,
    CompleteWorkflowRequest, CreateScheduleRequest, CreateScheduleResponse, DeleteScheduleRequest,
    DeregisterWorkerRequest, Empty, ErrorCode, ErrorDetail, FailStepRequest, FailWorkflowRequest,
    GetScheduleRequest, GetScheduleResponse, GetStepAttemptRequest, GetStepAttemptResponse,
    GetWorkerRequest, GetWorkerResponse, GetWorkflowRunRequest, GetWorkflowRunResponse,
    HealthCheckRequest, HealthCheckResponse, ListSchedulesRequest, ListSchedulesResponse,
    ListStepAttemptsRequest, ListStepAttemptsResponse, ListWorkersRequest, ListWorkersResponse,
    ListWorkflowRunsRequest, ListWorkflowRunsResponse, PollActivityRequest, PollActivityResponse,
    RegisterWorkerRequest, RegisterWorkerResponse, RetryPolicy, Schedule as ProtoSchedule,
    ScheduleSleepRequest, StartWorkflowRequest, StartWorkflowResponse, StepAttempt,
    StepAttemptStatus, StepKind, UpdateScheduleRequest, UpdateScheduleResponse, Worker,
    WorkerHeartbeatRequest, WorkerHeartbeatResponse, WorkflowRun, WorkflowStatus,
};
use kagzi_store::{
    BeginStepParams, CreateSchedule as StoreCreateSchedule, CreateWorkflow, FailStepParams,
    ListSchedulesParams, ListWorkersParams, ListWorkflowsParams, PgStore, RegisterWorkerParams,
    ScheduleRepository, StepRepository, UpdateSchedule as StoreUpdateSchedule,
    WorkerHeartbeatParams, WorkerRepository, WorkerStatus, WorkflowCursor, WorkflowRepository,
    WorkflowTypeConcurrency,
};
use prost::Message;
use std::collections::HashMap;
use std::str::FromStr;
use std::time::Duration;
use tonic::{Code, Request, Response, Status};
use tracing::{debug, info, instrument};

const MAX_QUEUE_CONCURRENCY: i32 = 10_000;
const MAX_TYPE_CONCURRENCY: i32 = 10_000;

/// Merge a protobuf `RetryPolicy` with an optional store-side fallback policy.
///
/// When `proto` is `Some`, fields with zero/empty values in the proto are treated as
/// "unspecified" and do not override corresponding fields from `fallback`. If `fallback`
/// is `None`, missing proto fields are populated with sensible defaults:
/// - `maximum_attempts` => 5
/// - `initial_interval_ms` => 1000
/// - `backoff_coefficient` => 2.0
/// - `maximum_interval_ms` => 60000
/// - `non_retryable_errors` => whatever is present in `proto` (empty list remains empty)
///
/// # Parameters
///
/// - `proto`: optional protobuf `RetryPolicy` carrying requested overrides.
/// - `fallback`: optional reference to an existing store `RetryPolicy` to use as base.
///
/// # Returns
///
/// `Some(kagzi_store::RetryPolicy)` containing the merged policy when either input is present, `None` if both are `None`.
///
/// # Examples
///
/// ```
/// // Example: proto provides overrides, fallback supplies defaults for unspecified fields.
/// let proto = kagzi_proto::RetryPolicy {
///     maximum_attempts: 3,
///     initial_interval_ms: 0, // treated as unspecified
///     backoff_coefficient: 1.5,
///     maximum_interval_ms: 0,
///     non_retryable_errors: vec!["fatal".to_string()],
/// };
/// let fallback = kagzi_store::RetryPolicy {
///     maximum_attempts: 10,
///     initial_interval_ms: 2000,
///     backoff_coefficient: 2.0,
///     maximum_interval_ms: 120_000,
///     non_retryable_errors: vec![],
/// };
/// let merged = merge_proto_policy(Some(proto), Some(&fallback)).unwrap();
/// assert_eq!(merged.maximum_attempts, 3); // overridden
/// assert_eq!(merged.initial_interval_ms, 2000); // taken from fallback
/// assert_eq!(merged.backoff_coefficient, 1.5); // overridden
/// assert_eq!(merged.maximum_interval_ms, 120_000); // taken from fallback
/// assert_eq!(merged.non_retryable_errors, vec!["fatal".to_string()]); // overridden
///
/// // Example: proto provided but no fallback — defaults fill unspecified fields.
/// let proto2 = kagzi_proto::RetryPolicy {
///     maximum_attempts: 0,
///     initial_interval_ms: 0,
///     backoff_coefficient: 0.0,
///     maximum_interval_ms: 0,
///     non_retryable_errors: vec![],
/// };
/// let merged2 = merge_proto_policy(Some(proto2), None).unwrap();
/// assert_eq!(merged2.maximum_attempts, 5);
/// assert_eq!(merged2.initial_interval_ms, 1000);
/// assert_eq!(merged2.backoff_coefficient, 2.0);
/// assert_eq!(merged2.maximum_interval_ms, 60000);
/// ```
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

/// Maps a store-level workflow status to its corresponding Protobuf `WorkflowStatus`.
///
/// # Returns
///
/// The `WorkflowStatus` enum value that corresponds to the provided `kagzi_store::WorkflowStatus`.
///
/// # Examples
///
/// ```
/// let proto = map_workflow_status(kagzi_store::WorkflowStatus::Running);
/// assert_eq!(proto, WorkflowStatus::Running);
/// ```
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

/// Convert a store `StepStatus` into its corresponding Protobuf `StepAttemptStatus`.
///
/// # Returns
///
/// `StepAttemptStatus` matching the provided store `StepStatus`.
///
/// # Examples
///
/// ```
/// let proto = map_step_status(kagzi_store::StepStatus::Running);
/// assert_eq!(proto, kagzi_proto::kagzi::StepAttemptStatus::Running);
/// ```
fn map_step_status(status: kagzi_store::StepStatus) -> StepAttemptStatus {
    match status {
        kagzi_store::StepStatus::Pending => StepAttemptStatus::Pending,
        kagzi_store::StepStatus::Running => StepAttemptStatus::Running,
        kagzi_store::StepStatus::Completed => StepAttemptStatus::Completed,
        kagzi_store::StepStatus::Failed => StepAttemptStatus::Failed,
    }
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

/// Create an ErrorDetail with the unspecified error code and the given message.
///
/// The message defaults to an empty string when `None`.
///
/// # Examples
///
/// ```
/// let d = string_error_detail(Some("oops".to_string()));
/// assert_eq!(d.message, "oops");
/// let empty = string_error_detail(None);
/// assert_eq!(empty.message, "");
/// ```
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

/// Convert a store-layer `WorkflowRun` into its Protobuf `WorkflowRun` representation.
///
/// Serializes the workflow input, optional output, and optional context to JSON bytes,
/// maps timestamps and status, and embeds any stored error detail. Returns a gRPC `Status`
/// error if JSON serialization of input/output/context fails.
///
/// # Examples
///
/// ```no_run
/// // Given a `kagzi_store::WorkflowRun` named `w`:
/// let proto = workflow_to_proto(w)?;
/// assert_eq!(proto.run_id, w.run_id.to_string());
/// # Ok::<(), tonic::Status>(())
/// ```
fn workflow_to_proto(w: kagzi_store::WorkflowRun) -> Result<WorkflowRun, Status> {
    let input_bytes = serde_json::to_vec(&w.input).map_err(|e| {
        tracing::error!("Failed to serialize workflow input: {:?}", e);
        internal("Failed to serialize workflow input")
    })?;

    let output_bytes = w
        .output
        .map(|o| {
            serde_json::to_vec(&o).map_err(|e| {
                tracing::error!("Failed to serialize workflow output: {:?}", e);
                internal("Failed to serialize workflow output")
            })
        })
        .transpose()?
        .unwrap_or_default();

    let context_bytes = w
        .context
        .map(|c| {
            serde_json::to_vec(&c).map_err(|e| {
                tracing::error!("Failed to serialize workflow context: {:?}", e);
                internal("Failed to serialize workflow context")
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
        error: Some(string_error_detail(w.error)),
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

/// Convert a store StepRun into its Protobuf StepAttempt representation.
///
/// The resulting `StepAttempt` mirrors the source `StepRun`: fields such as IDs,
/// timestamps, status, kind, serialized JSON output, error details, and namespace
/// are populated from the store model. Serialization failures for the step
/// output produce an internal gRPC `Status` error.
///
/// # Examples
///
/// ```ignore
/// // Given a `kagzi_store::StepRun` named `step_run`:
/// let proto = step_to_proto(step_run)?;
/// assert_eq!(proto.step_id, "example_step");
/// ```
///
/// # Returns
///
/// `Ok(StepAttempt)` with fields populated from the provided `StepRun`, or an
/// `Err(tonic::Status)` with code `Internal` if the step output fails to
/// serialize.
fn step_to_proto(s: kagzi_store::StepRun) -> Result<StepAttempt, Status> {
    let kind = if s.step_id.contains("sleep") || s.step_id.contains("wait") {
        StepKind::Sleep
    } else if s.step_id.contains("function") || s.step_id.contains("task") {
        StepKind::Function
    } else {
        StepKind::Unspecified
    };

    let output_bytes = serde_json::to_vec(&s.output).map_err(|e| {
        tracing::error!("Failed to serialize step output: {:?}", e);
        internal("Failed to serialize step output")
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
        error: Some(string_error_detail(s.error)),
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

/// Converts a store `kagzi_store::Worker` into its Protobuf `Worker` representation.
///
/// The returned proto has timestamps converted to `prost_types::Timestamp`, JSON labels
/// extracted into a string map, and optional fields populated with sensible defaults.
///
/// # Examples
///
/// ```no_run
/// // `store_worker` is a kagzi_store::Worker retrieved from the store.
/// let proto = worker_to_proto(store_worker);
/// assert_eq!(proto.worker_id, proto.worker_id);
/// ```
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
        queue_concurrency_limit: w.queue_concurrency_limit.unwrap_or(0),
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

/// Normalize a numeric limit to an allowed positive range.
///
/// Returns `None` when `raw` is less than or equal to zero; otherwise returns `Some` containing
/// `raw` capped to `max_allowed`.
///
/// # Examples
///
/// ```
/// assert_eq!(normalize_limit(5, 10), Some(5));
/// assert_eq!(normalize_limit(20, 10), Some(10));
/// assert_eq!(normalize_limit(0, 10), None);
/// ```
fn normalize_limit(raw: i32, max_allowed: i32) -> Option<i32> {
    if raw <= 0 {
        None
    } else {
        Some(raw.min(max_allowed))
    }
}

/// Maps a kagzi_store::StoreError into an appropriate gRPC `tonic::Status`.
///
/// The returned status encodes an equivalent gRPC error code and message for the provided
/// store-level error, so callers can return it directly from service handlers.
///
/// # Returns
///
/// A `tonic::Status` representing the corresponding gRPC error for `e`.
///
/// # Examples
///
/// ```
/// use tonic::Code;
/// let status = map_store_error(kagzi_store::StoreError::NotFound { entity: "workflow".into(), id: "r1".into() });
/// assert_eq!(status.code(), Code::NotFound);
/// ```
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

/// Clamp a max-catchup value to the allowed range.
///
/// Ensures the returned value is at least 1 and at most 10,000.
///
/// # Examples
///
/// ```
/// assert_eq!(clamp_max_catchup(0), 1);
/// assert_eq!(clamp_max_catchup(500), 500);
/// assert_eq!(clamp_max_catchup(20_000), 10_000);
/// ```
fn clamp_max_catchup(raw: i32) -> i32 {
    raw.clamp(1, 10_000)
}

/// Parses and validates a cron expression, returning a CronSchedule on success.
///
/// Returns an `InvalidArgument` gRPC `Status` if the expression is empty or fails to parse.
///
/// # Examples
///
/// ```
/// let ok = parse_cron_expr("*/5 * * * *").unwrap();
/// let err = parse_cron_expr("   ").unwrap_err();
/// assert!(err.code() == tonic::Code::InvalidArgument);
/// ```
fn parse_cron_expr(expr: &str) -> Result<CronSchedule, Status> {
    if expr.trim().is_empty() {
        return Err(invalid_argument("cron_expr cannot be empty"));
    }

    CronSchedule::from_str(expr).map_err(|e| invalid_argument(format!("Invalid cron: {}", e)))
}

/// Compute the next scheduled UTC time after `now` for a cron expression.
///
/// Parses `cron_expr` as a cron schedule and returns the next occurrence strictly after
/// `now`, converted to UTC.
///
/// # Errors
///
/// Returns a gRPC `Status` with `INVALID_ARGUMENT` if the cron expression is invalid
/// or if the schedule has no future occurrences.
///
/// # Examples
///
/// ```
/// use chrono::Utc;
/// // returns the next fire time after now for a cron that runs every minute
/// let next = next_fire_from_now("* * * * *", Utc::now()).unwrap();
/// assert!(next > Utc::now());
/// ```
fn next_fire_from_now(
    cron_expr: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<chrono::DateTime<chrono::Utc>, Status> {
    let schedule = parse_cron_expr(cron_expr)?;
    let next = schedule
        .after(&now)
        .next()
        .ok_or_else(|| invalid_argument("Cron expression has no future occurrences"))?;
    Ok(next.with_timezone(&chrono::Utc))
}

/// Convert a `chrono::DateTime<Utc>` into a `prost_types::Timestamp`.
///
/// # Examples
///
/// ```
/// use chrono::TimeZone;
/// let dt = chrono::Utc.ymd(2020, 1, 2).and_hms_nano(03, 04, 05, 6);
/// let ts = to_proto_timestamp(dt);
/// assert_eq!(ts.seconds, dt.timestamp());
/// assert_eq!(ts.nanos as u32, dt.timestamp_subsec_nanos());
/// ```
fn to_proto_timestamp(ts: chrono::DateTime<chrono::Utc>) -> prost_types::Timestamp {
    prost_types::Timestamp {
        seconds: ts.timestamp(),
        nanos: ts.timestamp_subsec_nanos() as i32,
    }
}

/// Convert a store `Schedule` into its Protobuf `Schedule` representation.
///
/// This serializes the schedule's `input` and optional `context` to JSON bytes and maps store fields
/// (IDs, task queue, cron expression, timestamps, flags and limits) into the Protobuf `Schedule`.
///
/// # Returns
///
/// `Ok(ProtoSchedule)` on success, or an `Err(Status)` with code `Internal` if JSON serialization of
/// the schedule's `input` or `context` fails.
///
/// # Examples
///
/// ```
/// // Given a `schedule` value from the store:
/// let proto = schedule_to_proto(schedule).unwrap();
/// assert_eq!(proto.schedule_id, schedule.schedule_id.to_string());
/// ```
fn schedule_to_proto(s: kagzi_store::Schedule) -> Result<ProtoSchedule, Status> {
    let input_bytes = serde_json::to_vec(&s.input).map_err(|e| {
        tracing::error!("Failed to serialize schedule input: {:?}", e);
        internal("Failed to serialize schedule input")
    })?;

    let context_bytes = s
        .context
        .map(|c| {
            serde_json::to_vec(&c).map_err(|e| {
                tracing::error!("Failed to serialize schedule context: {:?}", e);
                internal("Failed to serialize schedule context")
            })
        })
        .transpose()?
        .unwrap_or_default();

    Ok(ProtoSchedule {
        schedule_id: s.schedule_id.to_string(),
        namespace_id: s.namespace_id,
        task_queue: s.task_queue,
        workflow_type: s.workflow_type,
        cron_expr: s.cron_expr,
        input: input_bytes,
        context: context_bytes,
        enabled: s.enabled,
        max_catchup: s.max_catchup,
        next_fire_at: Some(to_proto_timestamp(s.next_fire_at)),
        last_fired_at: s.last_fired_at.map(to_proto_timestamp),
        version: s.version.unwrap_or_default(),
    })
}

#[derive(Clone)]
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
    /// Register a new worker and persist its metadata in the store.
    ///
    /// The request's empty `namespace_id` defaults to `"default"`. `workflow_types` must be non-empty.
    /// Optional fields (`hostname`, `pid`, `version`) are omitted when empty/zero. `max_concurrent` is
    /// normalized to at least 1. `labels` are stored as JSON. Queue and per-type concurrency limits
    /// are normalized and bounded by configured maximums.
    ///
    /// # Returns
    ///
    /// A `RegisterWorkerResponse` containing the assigned `worker_id` and the heartbeat interval in
    /// seconds (`heartbeat_interval_secs`).
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use tonic::Request;
    /// # // placeholder imports; replace with actual types when running
    /// # use kagzi_proto::kagzi::RegisterWorkerRequest;
    /// # use kagzi_service::MyWorkflowService;
    /// # async fn example(svc: &MyWorkflowService) {
    /// let req = RegisterWorkerRequest {
    ///     namespace_id: "".into(), // will default to "default"
    ///     task_queue: "default-queue".into(),
    ///     workflow_types: vec!["email_send".into()],
    ///     hostname: "".into(),
    ///     pid: 0,
    ///     version: "".into(),
    ///     max_concurrent: 4,
    ///     labels: std::collections::HashMap::new(),
    ///     queue_concurrency_limit: 0,
    ///     workflow_type_concurrency: vec![],
    /// };
    ///
    /// let resp = svc.register_worker(Request::new(req)).await.unwrap().into_inner();
    /// assert!(!resp.worker_id.is_empty());
    /// assert!(resp.heartbeat_interval_secs > 0);
    /// # }
    /// ```
    async fn register_worker(
        &self,
        request: Request<RegisterWorkerRequest>,
    ) -> Result<Response<RegisterWorkerResponse>, Status> {
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
                queue_concurrency_limit: normalize_limit(
                    req.queue_concurrency_limit,
                    MAX_QUEUE_CONCURRENCY,
                ),
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
            .map_err(|_| invalid_argument("Invalid worker_id"))?;

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
            .map_err(|_| invalid_argument("Invalid worker_id"))?;

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
                    .map_err(|_| invalid_argument("Invalid page_token"))?,
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
            .map_err(|_| invalid_argument("Invalid worker_id"))?;

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
            None => Err(not_found("Worker not found", "worker", req.worker_id)),
        }
    }

    /// Starts a new workflow run from the given request.
    ///
    /// The request's `input` is parsed as JSON (an empty `input` becomes `null`); `context` is parsed as JSON when present. If `namespace_id` is empty it defaults to `"default"`. If `version` is empty it defaults to `"1"`. When `idempotency_key` is provided and a matching workflow run already exists, the existing run id is returned instead of creating a new run. The request's retry policy is merged with any stored defaults before creation.
    ///
    /// # Returns
    ///
    /// A `StartWorkflowResponse` containing the created (or existing) workflow run id in `run_id`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use kagzi_proto::kagzi::StartWorkflowRequest;
    /// # use tonic::Request;
    /// # async fn example(svc: &crate::MyWorkflowService) {
    /// let req = StartWorkflowRequest {
    ///     workflow_id: "order-123".into(),
    ///     task_queue: "default".into(),
    ///     workflow_type: "process_order".into(),
    ///     input: serde_json::json!(null).to_string().into_bytes(),
    ///     ..Default::default()
    /// };
    /// let resp = svc.start_workflow(Request::new(req)).await.unwrap();
    /// println!("{}", resp.into_inner().run_id);
    /// # }
    /// ```
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
                .map_err(|e| invalid_argument(format!("Input must be valid JSON: {}", e)))?
        };

        let context_json: Option<serde_json::Value> = if req.context.is_empty() {
            None
        } else {
            Some(
                serde_json::from_slice(&req.context)
                    .map_err(|e| invalid_argument(format!("Context must be valid JSON: {}", e)))?,
            )
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
        }))
    }

    /// Creates a new cron-backed schedule from the RPC request and returns the created schedule.
    ///
    /// Validates required fields (`task_queue`, `workflow_type`, `cron_expr`), parses JSON `input` and
    /// optional `context`, applies defaults for `namespace_id`, `enabled`, `max_catchup`, and `version`,
    /// computes the schedule's next fire time from the provided cron expression, persists the schedule,
    /// and returns the created schedule converted to its protobuf representation.
    ///
    /// # Errors
    ///
    /// Returns `InvalidArgument` when required fields are missing or `input`/`context` are invalid JSON.
    /// Store-layer errors are mapped to appropriate gRPC `Status` values.
    ///
    /// # Examples
    ///
    /// ```
    /// # use kagzi_proto::kagzi::CreateScheduleRequest;
    /// # async fn example() -> Result<(), tonic::Status> {
    /// let req = CreateScheduleRequest {
    ///     namespace_id: "".to_string(), // will default to "default"
    ///     task_queue: "email".to_string(),
    ///     workflow_type: "send_reminder".to_string(),
    ///     cron_expr: "0 9 * * *".to_string(),
    ///     input: serde_json::json!({"user_id": 42}).to_string().into_bytes(),
    ///     context: vec![],
    ///     enabled: Some(true),
    ///     max_catchup: 0,
    ///     version: "".to_string(),
    /// };
    /// // service: &MyWorkflowService
    /// // let resp = service.create_schedule(tonic::Request::new(req)).await?;
    /// # Ok(())
    /// # }
    /// ```
    async fn create_schedule(
        &self,
        request: Request<CreateScheduleRequest>,
    ) -> Result<Response<CreateScheduleResponse>, Status> {
        let req = request.into_inner();

        if req.task_queue.is_empty() {
            return Err(invalid_argument("task_queue is required"));
        }
        if req.workflow_type.is_empty() {
            return Err(invalid_argument("workflow_type is required"));
        }
        if req.cron_expr.is_empty() {
            return Err(invalid_argument("cron_expr is required"));
        }

        let namespace_id = if req.namespace_id.is_empty() {
            "default".to_string()
        } else {
            req.namespace_id
        };

        let input_json: serde_json::Value = if req.input.is_empty() {
            serde_json::json!(null)
        } else {
            serde_json::from_slice(&req.input)
                .map_err(|e| invalid_argument(format!("Input must be valid JSON: {}", e)))?
        };

        let context_json: Option<serde_json::Value> = if req.context.is_empty() {
            None
        } else {
            Some(
                serde_json::from_slice(&req.context)
                    .map_err(|e| invalid_argument(format!("Context must be valid JSON: {}", e)))?,
            )
        };

        let enabled = req.enabled.unwrap_or(true);
        let max_catchup = if req.max_catchup == 0 {
            100
        } else {
            clamp_max_catchup(req.max_catchup)
        };

        let next_fire_at = next_fire_from_now(&req.cron_expr, chrono::Utc::now())?;

        let schedule_id = self
            .store
            .schedules()
            .create(StoreCreateSchedule {
                namespace_id: namespace_id.clone(),
                task_queue: req.task_queue,
                workflow_type: req.workflow_type,
                cron_expr: req.cron_expr,
                input: input_json,
                context: context_json,
                enabled,
                max_catchup,
                next_fire_at,
                version: if req.version.is_empty() {
                    None
                } else {
                    Some(req.version)
                },
            })
            .await
            .map_err(map_store_error)?;

        let schedule = self
            .store
            .schedules()
            .find_by_id(schedule_id, &namespace_id)
            .await
            .map_err(map_store_error)?;

        let schedule = schedule
            .map(schedule_to_proto)
            .transpose()?
            .ok_or_else(|| not_found("Schedule not found", "schedule", schedule_id.to_string()))?;

        Ok(Response::new(CreateScheduleResponse {
            schedule: Some(schedule),
        }))
    }

    /// Retrieves a schedule by its ID within the specified namespace.
    ///
    /// If `namespace_id` is empty, the namespace defaults to `"default"`.
    /// Returns an `InvalidArgument` error when `schedule_id` is not a valid UUID,
    /// and a `NotFound` error when no schedule exists with the given ID in the
    /// resolved namespace.
    ///
    /// # Examples
    ///
    /// ```
    /// # use tonic::Request;
    /// # use kagzi_proto::kagzi::GetScheduleRequest;
    /// # async fn _example(svc: &crate::MyWorkflowService) {
    /// let req = GetScheduleRequest {
    ///     schedule_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
    ///     namespace_id: "".to_string(), // will use "default"
    /// };
    /// let resp = svc.get_schedule(Request::new(req)).await.unwrap();
    /// let schedule = resp.into_inner().schedule.unwrap();
    /// # }
    /// ```
    async fn get_schedule(
        &self,
        request: Request<GetScheduleRequest>,
    ) -> Result<Response<GetScheduleResponse>, Status> {
        let req = request.into_inner();
        let schedule_id = uuid::Uuid::parse_str(&req.schedule_id)
            .map_err(|_| invalid_argument("Invalid schedule_id"))?;

        let namespace_id = if req.namespace_id.is_empty() {
            "default".to_string()
        } else {
            req.namespace_id
        };

        let schedule = self
            .store
            .schedules()
            .find_by_id(schedule_id, &namespace_id)
            .await
            .map_err(map_store_error)?;

        let schedule = schedule
            .map(schedule_to_proto)
            .transpose()?
            .ok_or_else(|| not_found("Schedule not found", "schedule", req.schedule_id))?;

        Ok(Response::new(GetScheduleResponse {
            schedule: Some(schedule),
        }))
    }

    /// Lists schedules in a namespace, optionally filtered by task queue.
    ///
    /// If `namespace_id` is empty the namespace defaults to `"default"`. The requested `limit` defaults to 100 when non-positive and is clamped to a maximum of 500. If `task_queue` is empty no task-queue filter is applied. The response contains the matching schedules converted to their protobuf representation.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use kagzi_proto::kagzi::ListSchedulesRequest;
    /// // `svc` is a `MyWorkflowService` instance behind a gRPC server; this shows the request shape.
    /// let req = ListSchedulesRequest { namespace_id: "".into(), task_queue: "".into(), limit: 0 };
    /// // let resp = svc.list_schedules(tonic::Request::new(req)).await.unwrap();
    /// ```
    async fn list_schedules(
        &self,
        request: Request<ListSchedulesRequest>,
    ) -> Result<Response<ListSchedulesResponse>, Status> {
        let req = request.into_inner();

        let namespace_id = if req.namespace_id.is_empty() {
            "default".to_string()
        } else {
            req.namespace_id
        };

        let limit = if req.limit <= 0 {
            100
        } else {
            req.limit.min(500)
        };

        let schedules = self
            .store
            .schedules()
            .list(ListSchedulesParams {
                namespace_id,
                task_queue: if req.task_queue.is_empty() {
                    None
                } else {
                    Some(req.task_queue)
                },
                limit: Some(limit as i64),
            })
            .await
            .map_err(map_store_error)?;

        let mut proto_schedules = Vec::with_capacity(schedules.len());
        for s in schedules {
            proto_schedules.push(schedule_to_proto(s)?);
        }

        Ok(Response::new(ListSchedulesResponse {
            schedules: proto_schedules,
        }))
    }

    /// Updates an existing schedule with the provided fields and returns the updated schedule.
    ///
    /// The request may modify the task queue, workflow type, cron expression, serialized input/context,
    /// enabled flag, max catch-up, explicit next fire time, and optimistic `version`. When a new cron
    /// expression is supplied the method computes the next firing time relative to now; if that time
    /// equals the schedule's current `next_fire_at`, it advances to the subsequent occurrence to avoid
    /// duplicating the same instant. JSON `input` and `context` bytes are validated and must be valid JSON.
    ///
    /// # Errors
    ///
    /// May return a gRPC `Status` with:
    /// - `InvalidArgument` when IDs, cron, or JSON payloads are malformed or the cron has no future occurrences;
    /// - `NotFound` if the schedule does not exist;
    /// - Other mapped store errors (`FailedPrecondition`, `Conflict`, `Unavailable`, `Internal`, etc.) as produced by the backing store.
    ///
    /// # Returns
    ///
    /// An `UpdateScheduleResponse` containing the updated schedule.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use tonic::Request;
    /// use kagzi_proto::kagzi::UpdateScheduleRequest;
    ///
    /// # async fn doc_example() -> Result<(), Box<dyn std::error::Error>> {
    /// let svc: MyWorkflowService = /* obtain service instance */ unimplemented!();
    /// let req = UpdateScheduleRequest {
    ///     schedule_id: "00000000-0000-0000-0000-000000000000".to_string(),
    ///     namespace_id: "default".to_string(),
    ///     task_queue: "queue".to_string(),
    ///     workflow_type: "my_workflow".to_string(),
    ///     cron_expr: Some("0 0 * * *".to_string()),
    ///     input: None,
    ///     context: None,
    ///     enabled: true,
    ///     max_catchup: Some(10),
    ///     next_fire_at: None,
    ///     version: None,
    /// };
    ///
    /// let _resp = svc.update_schedule(Request::new(req)).await?;
    /// # Ok(()) }
    /// ```
    async fn update_schedule(
        &self,
        request: Request<UpdateScheduleRequest>,
    ) -> Result<Response<UpdateScheduleResponse>, Status> {
        let req = request.into_inner();

        let schedule_id = uuid::Uuid::parse_str(&req.schedule_id)
            .map_err(|_| invalid_argument("Invalid schedule_id"))?;

        let namespace_id = if req.namespace_id.is_empty() {
            "default".to_string()
        } else {
            req.namespace_id
        };

        let current_schedule = self
            .store
            .schedules()
            .find_by_id(schedule_id, &namespace_id)
            .await
            .map_err(map_store_error)?
            .ok_or_else(|| not_found("Schedule not found", "schedule", req.schedule_id.clone()))?;

        let cron_expr = req.cron_expr.clone();
        let parsed_cron = if let Some(ref expr) = cron_expr {
            Some(parse_cron_expr(expr)?)
        } else {
            None
        };

        let next_fire_at = if let Some(cron) = parsed_cron.as_ref() {
            // Cron was updated; compute next fire from now.
            let candidate = cron
                .after(&chrono::Utc::now())
                .next()
                .ok_or_else(|| invalid_argument("Cron expression has no future occurrences"))?
                .with_timezone(&chrono::Utc);

            // If the computed time equals the existing next_fire_at (e.g., same cron),
            // advance to the subsequent occurrence to avoid scheduling the same instant.
            if candidate == current_schedule.next_fire_at {
                cron.after(&candidate)
                    .next()
                    .map(|dt| dt.with_timezone(&chrono::Utc))
            } else {
                Some(candidate)
            }
        } else if let Some(ts) = req.next_fire_at {
            // Explicit next_fire_at override without cron change.
            chrono::DateTime::from_timestamp(ts.seconds, ts.nanos as u32)
        } else {
            None
        };

        let input_json = match req.input {
            Some(bytes) => Some(
                serde_json::from_slice(&bytes)
                    .map_err(|e| invalid_argument(format!("Input must be valid JSON: {}", e)))?,
            ),
            None => None,
        };

        let context_json = match req.context {
            Some(bytes) => Some(
                serde_json::from_slice(&bytes)
                    .map_err(|e| invalid_argument(format!("Context must be valid JSON: {}", e)))?,
            ),
            None => None,
        };

        let max_catchup = req.max_catchup.map(clamp_max_catchup);

        self.store
            .schedules()
            .update(
                schedule_id,
                &namespace_id,
                StoreUpdateSchedule {
                    task_queue: req.task_queue,
                    workflow_type: req.workflow_type,
                    cron_expr,
                    input: input_json,
                    context: context_json,
                    enabled: req.enabled,
                    max_catchup,
                    next_fire_at,
                    version: req.version,
                },
            )
            .await
            .map_err(map_store_error)?;

        let schedule = self
            .store
            .schedules()
            .find_by_id(schedule_id, &namespace_id)
            .await
            .map_err(map_store_error)?;

        let schedule = schedule
            .map(schedule_to_proto)
            .transpose()?
            .ok_or_else(|| not_found("Schedule not found", "schedule", req.schedule_id))?;

        Ok(Response::new(UpdateScheduleResponse {
            schedule: Some(schedule),
        }))
    }

    /// Deletes a schedule identified by `schedule_id` in the given namespace.
    ///
    /// If `namespace_id` is empty the default namespace ("default") is used. Returns an empty response when
    /// the schedule was successfully deleted.
    ///
    /// # Errors
    ///
    /// - Returns `InvalidArgument` if `schedule_id` is not a valid UUID.
    /// - Returns `NotFound` if no schedule with the given ID exists in the namespace.
    /// - Other store-related failures are returned as appropriate gRPC error statuses.
    ///
    /// # Examples
    ///
    /// ```
    /// # // This is illustrative; in real usage `svc` is an initialized service and call is awaited in async context.
    /// use kagzi_proto::kagzi::DeleteScheduleRequest;
    /// let req = DeleteScheduleRequest { schedule_id: "00000000-0000-0000-0000-000000000000".into(), namespace_id: "".into() };
    /// // let resp = svc.delete_schedule(tonic::Request::new(req)).await;
    /// ```
    async fn delete_schedule(
        &self,
        request: Request<DeleteScheduleRequest>,
    ) -> Result<Response<Empty>, Status> {
        let req = request.into_inner();
        let schedule_id = uuid::Uuid::parse_str(&req.schedule_id)
            .map_err(|_| invalid_argument("Invalid schedule_id"))?;

        let namespace_id = if req.namespace_id.is_empty() {
            "default".to_string()
        } else {
            req.namespace_id
        };

        let deleted = self
            .store
            .schedules()
            .delete(schedule_id, &namespace_id)
            .await
            .map_err(map_store_error)?;

        if !deleted {
            return Err(not_found("Schedule not found", "schedule", req.schedule_id));
        }

        Ok(Response::new(Empty {}))
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
                let status = not_found(
                    format!(
                        "Workflow run not found: run_id={}, namespace_id={}",
                        run_id, namespace_id
                    ),
                    "workflow_run",
                    run_id.to_string(),
                );

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

    /// Lists workflow runs in a namespace with optional status filtering and cursor-based pagination.
    ///
    /// The RPC accepts an optional `namespace_id` (defaults to `"default"` when empty),
    /// a `filter_status` to restrict results, a `page_size` clamped to a maximum of 100
    /// (and a default of 20 when the provided value is zero or negative), and a `page_token`
    /// that encodes a cursor for pagination.
    ///
    /// The `page_token` is expected to be a Base64-encoded string of the form
    /// `"{timestamp_millis}:{run_id}"`. The response includes `next_page_token` encoded
    /// with the same format when more results exist, and a `has_more` flag indicating
    /// whether additional pages are available.
    ///
    /// # Examples
    ///
    /// ```
    /// # use kagzi_proto::kagzi::{ListWorkflowRunsRequest, ListWorkflowRunsResponse};
    /// # use tonic::Request;
    /// # async fn example(svc: &super::MyWorkflowService) {
    /// let req = ListWorkflowRunsRequest {
    ///     namespace_id: "default".to_string(),
    ///     filter_status: String::new(),
    ///     page_size: 25,
    ///     page_token: String::new(),
    /// };
    /// let resp = svc.list_workflow_runs(Request::new(req)).await.unwrap();
    /// let ListWorkflowRunsResponse { workflow_runs, has_more, .. } = resp.into_inner();
    /// // use `workflow_runs` and `has_more`
    /// # }
    /// ```
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

        let cursor: Option<WorkflowCursor> = if req.page_token.is_empty() {
            None
        } else {
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(&req.page_token)
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

        let next_page_token = result
            .next_cursor
            .map(|c| {
                let cursor_str = format!("{}:{}", c.created_at.timestamp_millis(), c.run_id);
                base64::engine::general_purpose::STANDARD.encode(cursor_str.as_bytes())
            })
            .unwrap_or_default();

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

    /// Cancels a workflow run identified by run_id in the specified namespace.
    ///
    /// Attempts to transition a workflow run to the cancelled state. The request's
    /// `namespace_id` defaults to `"default"` when empty.
    ///
    /// Errors:
    /// - `InvalidArgument` if `run_id` is not a valid UUID.
    /// - `NotFound` if the workflow run does not exist in the namespace.
    /// - `FailedPrecondition` if the workflow exists but cannot be cancelled due to its current status.
    /// - Other store-related errors are mapped to appropriate gRPC `Status` codes.
    ///
    /// # Returns
    ///
    /// `Empty` on success.
    ///
    /// # Examples
    ///
    /// ```
    /// use kagzi_proto::kagzi::CancelWorkflowRunRequest;
    /// use tonic::Request;
    ///
    /// // Build a cancel request (defaults namespace to "default" when empty)
    /// let req = CancelWorkflowRunRequest {
    ///     run_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
    ///     namespace_id: "".to_string(),
    /// };
    ///
    /// // Call the service method with `Request::new(req)` and handle the `Result`.
    /// // let resp = service.cancel_workflow_run(Request::new(req)).await;
    /// ```
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
                "CancelWorkflowRun",
                &correlation_id,
                &trace_id,
                Status::code(&Status::ok("")),
                None,
            );

            Ok(Response::new(Empty {}))
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
                        "Workflow run not found: run_id={}, namespace_id={}",
                        run_id, namespace_id
                    ),
                    "workflow_run",
                    run_id.to_string(),
                )
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

    /// Claims the next available workflow run for a worker and returns the workflow's input when work is available.
    ///
    /// Returns an empty response when no work is available within the configured poll timeout.
    ///
    /// # Errors
    ///
    /// Returns a gRPC `InvalidArgument` error when `worker_id` is not a valid UUID or
    /// when `supported_workflow_types` is empty.
    ///
    /// Returns a gRPC `FailedPrecondition` error when the worker is not registered, is offline,
    /// is draining, is registered for a different namespace/task_queue, or is not registered for
    /// any of the requested workflow types.
    ///
    /// # Returns
    ///
    /// A `PollActivityResponse` containing:
    /// - `run_id`: the workflow run id, or an empty string if no work was returned.
    /// - `workflow_type`: the workflow type, or an empty string if no work was returned.
    /// - `workflow_input`: serialized JSON bytes of the workflow input, or an empty vec if no work was returned.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use kagzi_proto::kagzi::PollActivityRequest;
    /// # use tonic::Request;
    /// # async fn example(svc: &MyWorkflowService) {
    /// let req = PollActivityRequest {
    ///     worker_id: "00000000-0000-0000-0000-000000000000".to_string(),
    ///     task_queue: "default".to_string(),
    ///     supported_workflow_types: vec!["email".to_string()],
    ///     namespace_id: "".to_string(),
    /// };
    /// let resp = svc.poll_activity(Request::new(req)).await;
    /// # }
    /// ```
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
            .map_err(|_| invalid_argument("Invalid worker_id"))?;

        if req.supported_workflow_types.is_empty() {
            return Err(invalid_argument("supported_workflow_types cannot be empty"));
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
                precondition_failed("Worker not registered or offline. Call RegisterWorker first.")
            })?;

        if worker.namespace_id != namespace_id || worker.task_queue != req.task_queue {
            return Err(precondition_failed(
                "Worker not registered for the requested namespace/task_queue",
            ));
        }

        if worker.status == WorkerStatus::Offline {
            return Err(precondition_failed(
                "Worker not registered or offline. Call RegisterWorker first.",
            ));
        }

        if worker.status == WorkerStatus::Draining {
            return Err(precondition_failed(
                "Worker is draining and not accepting new work",
            ));
        }

        let effective_types: Vec<String> = worker
            .workflow_types
            .iter()
            .filter(|t| req.supported_workflow_types.iter().any(|r| r == *t))
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

            let input_bytes = serde_json::to_vec(&work_item.input).map_err(|e| {
                tracing::error!("Failed to serialize workflow input: {:?}", e);
                internal("Failed to serialize workflow input")
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
                    internal("Failed to serialize workflow input")
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
            .map_err(|_| invalid_argument("Invalid step_attempt_id"))?;

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
                let status = not_found(
                    "Step attempt not found",
                    "step_attempt",
                    req.step_attempt_id,
                );

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
            .map_err(|_| invalid_argument("Invalid workflow_run_id"))?;

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

    /// Begins execution of a step for a workflow run and returns whether the caller should execute it along with any cached result.
    ///
    /// Returns a `BeginStepResponse` containing `should_execute` and `cached_result` (serialized bytes) for the requested step. The request's `run_id` and `step_id` identify the step; any provided retry policy is merged with the workflow's stored retry policy when determining behavior.
    ///
    /// # Examples
    ///
    /// ```
    /// # use tonic::Request;
    /// # use kagzi_proto::kagzi::BeginStepRequest;
    /// # async fn example(svc: &crate::MyWorkflowService) {
    /// let req = BeginStepRequest {
    ///     run_id: "00000000-0000-0000-0000-000000000000".into(),
    ///     step_id: "step-1".into(),
    ///     input: Vec::new(),
    ///     retry_policy: None,
    /// };
    /// let resp = svc.begin_step(Request::new(req)).await.unwrap().into_inner();
    /// println!("should_execute={}", resp.should_execute);
    /// # }
    /// ```
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

        let run_id =
            uuid::Uuid::parse_str(&req.run_id).map_err(|_| invalid_argument("Invalid run_id"))?;

        let workflow_retry = self
            .store
            .workflows()
            .get_retry_policy(run_id)
            .await
            .map_err(map_store_error)?;

        let input: Option<serde_json::Value> = if !req.input.is_empty() {
            Some(
                serde_json::from_slice(&req.input)
                    .map_err(|e| invalid_argument(format!("Input must be valid JSON: {}", e)))?,
            )
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
                retry_policy: merge_proto_policy(req.retry_policy, workflow_retry.as_ref()),
            })
            .await
            .map_err(map_store_error)?;

        let cached_result = result
            .cached_output
            .map(|o| serde_json::to_vec(&o))
            .transpose()
            .map_err(|e| {
                tracing::error!("Failed to serialize cached step output: {:?}", e);
                internal("Failed to serialize cached step output")
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

        let run_id =
            uuid::Uuid::parse_str(&req.run_id).map_err(|_| invalid_argument("Invalid run_id"))?;

        let output_json: serde_json::Value = serde_json::from_slice(&req.output)
            .map_err(|e| invalid_argument(format!("Output must be valid JSON: {}", e)))?;

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
            .map_err(|_| invalid_argument("Invalid run_id: must be a valid UUID"))?;

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

        let run_id =
            uuid::Uuid::parse_str(&req.run_id).map_err(|_| invalid_argument("Invalid run_id"))?;

        let output_json: serde_json::Value = serde_json::from_slice(&req.output)
            .map_err(|e| invalid_argument(format!("Output must be valid JSON: {}", e)))?;

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

        let run_id =
            uuid::Uuid::parse_str(&req.run_id).map_err(|_| invalid_argument("Invalid run_id"))?;

        let error_detail = req.error.unwrap_or_else(|| ErrorDetail {
            code: ErrorCode::Unspecified as i32,
            message: String::new(),
            non_retryable: false,
            retry_after_ms: 0,
            subject: String::new(),
            subject_id: String::new(),
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

        let run_id =
            uuid::Uuid::parse_str(&req.run_id).map_err(|_| invalid_argument("Invalid run_id"))?;

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