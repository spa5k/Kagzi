//! Shared proto conversion functions for gRPC services.
//!
//! This module provides consistent conversion between store models and proto messages,
//! eliminating duplication across service implementations.

use std::collections::HashMap;

use kagzi_proto::kagzi::{
    Payload, Step, StepKind, StepStatus, Worker, WorkerStatus, Workflow, WorkflowStatus,
};
use kagzi_store::{
    StepKind as StoreStepKind, StepRun, StepStatus as StoreStepStatus, Worker as StoreWorker,
    WorkerStatus as StoreWorkerStatus, WorkflowRun, WorkflowStatus as StoreWorkflowStatus,
};
use tonic::Status;

use crate::helpers::{bytes_to_payload, invalid_argument_error, string_error_detail};

/// Convert chrono DateTime to proto Timestamp.
pub fn timestamp_from(dt: chrono::DateTime<chrono::Utc>) -> prost_types::Timestamp {
    prost_types::Timestamp {
        seconds: dt.timestamp(),
        nanos: dt.timestamp_subsec_nanos() as i32,
    }
}

/// Convert store WorkflowStatus to proto WorkflowStatus.
pub fn map_workflow_status(status: StoreWorkflowStatus) -> WorkflowStatus {
    match status {
        StoreWorkflowStatus::Pending => WorkflowStatus::Pending,
        StoreWorkflowStatus::Running => WorkflowStatus::Running,
        StoreWorkflowStatus::Sleeping => WorkflowStatus::Sleeping,
        StoreWorkflowStatus::Completed => WorkflowStatus::Completed,
        StoreWorkflowStatus::Failed => WorkflowStatus::Failed,
        StoreWorkflowStatus::Cancelled => WorkflowStatus::Cancelled,
        StoreWorkflowStatus::Scheduled => WorkflowStatus::Scheduled,
        StoreWorkflowStatus::Paused => WorkflowStatus::Paused,
    }
}

/// Convert proto WorkflowStatus to status string for store filtering.
pub fn workflow_status_to_string(status: WorkflowStatus) -> String {
    match status {
        WorkflowStatus::Pending => "PENDING",
        WorkflowStatus::Running => "RUNNING",
        WorkflowStatus::Sleeping => "SLEEPING",
        WorkflowStatus::Completed => "COMPLETED",
        WorkflowStatus::Failed => "FAILED",
        WorkflowStatus::Cancelled => "CANCELLED",
        WorkflowStatus::Scheduled => "SCHEDULED",
        WorkflowStatus::Paused => "PAUSED",
        WorkflowStatus::Unspecified => "UNSPECIFIED",
    }
    .to_string()
}

/// Convert store StepStatus to proto StepStatus.
pub fn map_step_status(status: StoreStepStatus) -> StepStatus {
    match status {
        StoreStepStatus::Pending => StepStatus::Pending,
        StoreStepStatus::Running => StepStatus::Running,
        StoreStepStatus::Completed => StepStatus::Completed,
        StoreStepStatus::Failed => StepStatus::Failed,
    }
}

/// Convert store StepKind to proto StepKind.
pub fn map_step_kind(kind: StoreStepKind) -> StepKind {
    match kind {
        StoreStepKind::Function => StepKind::Function,
        StoreStepKind::Sleep => StepKind::Sleep,
    }
}

/// Convert proto StepKind to store StepKind.
pub fn map_proto_step_kind(kind: i32) -> Result<StoreStepKind, Status> {
    let kind =
        StepKind::try_from(kind).map_err(|_| invalid_argument_error("step kind is required"))?;

    match kind {
        StepKind::Function => Ok(StoreStepKind::Function),
        StepKind::Sleep => Ok(StoreStepKind::Sleep),
        StepKind::ChildWorkflow => Ok(StoreStepKind::Function), // Map child workflow to function for now
        StepKind::Unspecified => Err(invalid_argument_error("step kind is required")),
    }
}

/// Convert store WorkerStatus to proto WorkerStatus.
pub fn map_worker_status(status: StoreWorkerStatus) -> WorkerStatus {
    match status {
        StoreWorkerStatus::Online => WorkerStatus::Online,
        StoreWorkerStatus::Draining => WorkerStatus::Draining,
        StoreWorkerStatus::Offline => WorkerStatus::Offline,
    }
}

/// Convert store WorkflowRun to proto Workflow.
pub fn workflow_to_proto(w: WorkflowRun) -> Result<Workflow, Status> {
    let error = w.error.map(|msg| string_error_detail(Some(msg)));

    Ok(Workflow {
        run_id: w.run_id.to_string(),
        external_id: w.external_id,
        namespace: w.namespace,
        task_queue: w.task_queue,
        workflow_type: w.workflow_type,
        status: map_workflow_status(w.status) as i32,
        input: Some(bytes_to_payload(Some(w.input))),
        output: Some(bytes_to_payload(w.output)),
        error,
        attempts: w.attempts,
        created_at: Some(timestamp_from(
            w.created_at.unwrap_or_else(chrono::Utc::now),
        )),
        started_at: w.started_at.map(timestamp_from),
        finished_at: w.finished_at.map(timestamp_from),
        wake_up_at: w.available_at.map(timestamp_from),
        worker_id: w.locked_by,
        version: w.version.unwrap_or_default(),
        parent_step_id: w.parent_step_attempt_id,
        parent_run_id: None,
        priority: None,
        timeout_ms: None,
        cron_expr: w.cron_expr,
        schedule_id: w.schedule_id.map(|id| id.to_string()),
    })
}

/// Convert store StepRun to proto Step.
pub fn step_to_proto(s: StepRun) -> Result<Step, Status> {
    let input = bytes_to_payload(s.input);
    let output = bytes_to_payload(s.output);
    let error = s.error.map(|msg| string_error_detail(Some(msg)));

    let step_id = s.step_id;

    Ok(Step {
        step_id: step_id.clone(),
        run_id: s.run_id.to_string(),
        namespace: s.namespace,
        name: step_id,
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

/// Convert store Worker to proto Worker.
pub fn worker_to_proto(w: StoreWorker) -> Worker {
    let labels = match w.labels {
        serde_json::Value::Object(map) => map
            .into_iter()
            .filter_map(|(k, v)| v.as_str().map(|s| (k, s.to_string())))
            .collect(),
        _ => HashMap::new(),
    };

    Worker {
        worker_id: w.worker_id.to_string(),
        task_queue: w.task_queue,
        status: map_worker_status(w.status) as i32,
        hostname: w.hostname.unwrap_or_default(),
        pid: w.pid.unwrap_or(0),
        version: w.version.unwrap_or_default(),
        workflow_types: w.workflow_types,
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
        active_workflow_count: 0,
        capabilities: HashMap::new(),
    }
}

/// Helper to create empty Payload.
pub fn empty_payload() -> Payload {
    Payload {
        data: Vec::new(),
        metadata: HashMap::new(),
    }
}
