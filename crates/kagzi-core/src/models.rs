//! Database models for Kagzi workflows

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

/// Status of a workflow run
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "text")]
pub enum WorkflowStatus {
    #[sqlx(rename = "PENDING")]
    Pending,
    #[sqlx(rename = "RUNNING")]
    Running,
    #[sqlx(rename = "COMPLETED")]
    Completed,
    #[sqlx(rename = "FAILED")]
    Failed,
    #[sqlx(rename = "SLEEPING")]
    Sleeping,
    #[sqlx(rename = "CANCELLED")]
    Cancelled,
}

impl std::fmt::Display for WorkflowStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WorkflowStatus::Pending => write!(f, "PENDING"),
            WorkflowStatus::Running => write!(f, "RUNNING"),
            WorkflowStatus::Completed => write!(f, "COMPLETED"),
            WorkflowStatus::Failed => write!(f, "FAILED"),
            WorkflowStatus::Sleeping => write!(f, "SLEEPING"),
            WorkflowStatus::Cancelled => write!(f, "CANCELLED"),
        }
    }
}

/// Status of a step run
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "text")]
pub enum StepStatus {
    #[sqlx(rename = "COMPLETED")]
    Completed,
    #[sqlx(rename = "FAILED")]
    Failed,
}

impl std::fmt::Display for StepStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StepStatus::Completed => write!(f, "COMPLETED"),
            StepStatus::Failed => write!(f, "FAILED"),
        }
    }
}

/// A workflow run represents a single execution of a workflow
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct WorkflowRun {
    pub id: Uuid,
    pub workflow_name: String,
    pub workflow_version: i32,
    pub input: serde_json::Value,
    pub output: Option<serde_json::Value>,
    pub status: WorkflowStatus,
    pub error: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub sleep_until: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
}

impl WorkflowRun {
    /// Get the error as a structured StepError if available
    pub fn get_step_error(&self) -> Option<crate::error::StepError> {
        self.error
            .as_ref()
            .and_then(|e| serde_json::from_value(e.clone()).ok())
    }

    /// Get the error message as a string
    pub fn error_message(&self) -> Option<String> {
        self.error.as_ref().and_then(|e| {
            if let Some(msg) = e.get("message") {
                msg.as_str().map(|s| s.to_string())
            } else {
                // Fallback for legacy string errors
                e.as_str().map(|s| s.to_string())
            }
        })
    }
}

/// A step run represents the memoized result of a single step in a workflow
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct StepRun {
    pub id: Option<i64>,
    pub workflow_run_id: Uuid,
    pub step_id: String,
    pub input_hash: Option<String>,
    pub output: Option<serde_json::Value>,
    pub error: Option<serde_json::Value>,
    pub status: StepStatus,
    pub attempts: i32,
    pub next_retry_at: Option<DateTime<Utc>>,
    pub retry_policy: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub parent_step_id: Option<String>,
    pub parallel_group_id: Option<Uuid>,
}

impl StepRun {
    /// Get the error as a structured StepError if available
    pub fn get_step_error(&self) -> Option<crate::error::StepError> {
        self.error
            .as_ref()
            .and_then(|e| serde_json::from_value(e.clone()).ok())
    }

    /// Get the error message as a string
    pub fn error_message(&self) -> Option<String> {
        self.error.as_ref().and_then(|e| {
            if let Some(msg) = e.get("message") {
                msg.as_str().map(|s| s.to_string())
            } else {
                // Fallback for legacy string errors
                e.as_str().map(|s| s.to_string())
            }
        })
    }

    /// Get the retry policy as a structured RetryPolicy if available
    pub fn get_retry_policy(&self) -> Option<crate::retry::RetryPolicy> {
        self.retry_policy
            .as_ref()
            .and_then(|p| serde_json::from_value(p.clone()).ok())
    }

    /// Check if this step should be retried
    pub fn should_retry(&self) -> bool {
        self.next_retry_at.is_some()
    }

    /// Check if retry is ready (next_retry_at has passed)
    pub fn is_retry_ready(&self, now: DateTime<Utc>) -> bool {
        self.next_retry_at
            .map(|retry_at| retry_at <= now)
            .unwrap_or(false)
    }

    /// Check if this step is part of a parallel group
    pub fn is_parallel(&self) -> bool {
        self.parallel_group_id.is_some()
    }

    /// Check if this step has a parent (nested parallel)
    pub fn has_parent(&self) -> bool {
        self.parent_step_id.is_some()
    }

    /// Check if this is a top-level step (no parent)
    pub fn is_top_level(&self) -> bool {
        self.parent_step_id.is_none()
    }
}

/// A step attempt represents a single retry attempt for a step
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct StepAttempt {
    pub id: i64,
    pub step_run_id: i64,
    pub attempt_number: i32,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub error: Option<serde_json::Value>,
    pub status: String,
}

impl StepAttempt {
    /// Get the error as a structured StepError if available
    pub fn get_step_error(&self) -> Option<crate::error::StepError> {
        self.error
            .as_ref()
            .and_then(|e| serde_json::from_value(e.clone()).ok())
    }

    /// Check if this attempt is still running
    pub fn is_running(&self) -> bool {
        self.status == "running"
    }

    /// Check if this attempt succeeded
    pub fn is_succeeded(&self) -> bool {
        self.status == "succeeded"
    }

    /// Check if this attempt failed
    pub fn is_failed(&self) -> bool {
        self.status == "failed"
    }
}

/// A worker lease ensures only one worker processes a workflow at a time
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct WorkerLease {
    pub workflow_run_id: Uuid,
    pub worker_id: String,
    pub acquired_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub heartbeat_at: DateTime<Utc>,
}

/// A workflow version tracks available versions of a workflow definition
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct WorkflowVersion {
    pub workflow_name: String,
    pub version: i32,
    pub is_default: bool,
    pub created_at: DateTime<Utc>,
    pub deprecated_at: Option<DateTime<Utc>>,
    pub description: Option<String>,
}

impl WorkflowVersion {
    /// Check if this version is active (not deprecated)
    pub fn is_active(&self) -> bool {
        self.deprecated_at.is_none()
    }

    /// Check if this version is deprecated
    pub fn is_deprecated(&self) -> bool {
        self.deprecated_at.is_some()
    }
}

/// Input for creating a new workflow run
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateWorkflowRun {
    pub workflow_name: String,
    pub workflow_version: Option<i32>,
    pub input: serde_json::Value,
}

/// Input for creating a step run
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateStepRun {
    pub workflow_run_id: Uuid,
    pub step_id: String,
    pub input_hash: Option<String>,
    pub output: Option<serde_json::Value>,
    pub error: Option<serde_json::Value>,
    pub status: StepStatus,
    pub parent_step_id: Option<String>,
    pub parallel_group_id: Option<Uuid>,
}

/// Worker status
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "text")]
pub enum WorkerStatus {
    #[sqlx(rename = "RUNNING")]
    Running,
    #[sqlx(rename = "SHUTTING_DOWN")]
    ShuttingDown,
    #[sqlx(rename = "STOPPED")]
    Stopped,
}

impl std::fmt::Display for WorkerStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WorkerStatus::Running => write!(f, "RUNNING"),
            WorkerStatus::ShuttingDown => write!(f, "SHUTTING_DOWN"),
            WorkerStatus::Stopped => write!(f, "STOPPED"),
        }
    }
}

/// Worker instance tracking
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Worker {
    pub id: Uuid,
    pub worker_name: String,
    pub hostname: Option<String>,
    pub process_id: Option<i32>,
    pub started_at: DateTime<Utc>,
    pub last_heartbeat: DateTime<Utc>,
    pub status: WorkerStatus,
    pub config: Option<serde_json::Value>,
    pub metadata: Option<serde_json::Value>,
    pub stopped_at: Option<DateTime<Utc>>,
}

impl Worker {
    /// Check if the worker is currently active (running)
    pub fn is_active(&self) -> bool {
        matches!(self.status, WorkerStatus::Running)
    }

    /// Check if the worker is shutting down
    pub fn is_shutting_down(&self) -> bool {
        matches!(self.status, WorkerStatus::ShuttingDown)
    }

    /// Check if the worker has stopped
    pub fn is_stopped(&self) -> bool {
        matches!(self.status, WorkerStatus::Stopped)
    }

    /// Check if the worker heartbeat is stale (older than given duration)
    pub fn is_heartbeat_stale(&self, max_age: chrono::Duration) -> bool {
        Utc::now() - self.last_heartbeat > max_age
    }

    /// Get the worker uptime
    pub fn uptime(&self) -> chrono::Duration {
        if let Some(stopped_at) = self.stopped_at {
            stopped_at - self.started_at
        } else {
            Utc::now() - self.started_at
        }
    }
}

/// Worker event type
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "text")]
pub enum WorkerEventType {
    #[sqlx(rename = "STARTED")]
    Started,
    #[sqlx(rename = "HEARTBEAT")]
    Heartbeat,
    #[sqlx(rename = "SHUTDOWN_REQUESTED")]
    ShutdownRequested,
    #[sqlx(rename = "SHUTDOWN_COMPLETE")]
    ShutdownComplete,
    #[sqlx(rename = "CRASHED")]
    Crashed,
    #[sqlx(rename = "WORKFLOW_STARTED")]
    WorkflowStarted,
    #[sqlx(rename = "WORKFLOW_COMPLETED")]
    WorkflowCompleted,
    #[sqlx(rename = "WORKFLOW_FAILED")]
    WorkflowFailed,
}

impl std::fmt::Display for WorkerEventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WorkerEventType::Started => write!(f, "STARTED"),
            WorkerEventType::Heartbeat => write!(f, "HEARTBEAT"),
            WorkerEventType::ShutdownRequested => write!(f, "SHUTDOWN_REQUESTED"),
            WorkerEventType::ShutdownComplete => write!(f, "SHUTDOWN_COMPLETE"),
            WorkerEventType::Crashed => write!(f, "CRASHED"),
            WorkerEventType::WorkflowStarted => write!(f, "WORKFLOW_STARTED"),
            WorkerEventType::WorkflowCompleted => write!(f, "WORKFLOW_COMPLETED"),
            WorkerEventType::WorkflowFailed => write!(f, "WORKFLOW_FAILED"),
        }
    }
}

/// Worker lifecycle event
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct WorkerEvent {
    pub id: Uuid,
    pub worker_id: Uuid,
    pub event_type: WorkerEventType,
    pub event_data: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
}

/// Worker metadata for tracking deployment information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerMetadata {
    pub hostname: String,
    pub process_id: u32,
    pub rust_version: String,
    pub kagzi_version: String,
    pub started_at: DateTime<Utc>,
}

impl WorkerMetadata {
    /// Create worker metadata for the current process
    pub fn current() -> Self {
        Self {
            hostname: hostname::get()
                .ok()
                .and_then(|h| h.into_string().ok())
                .unwrap_or_else(|| "unknown".to_string()),
            process_id: std::process::id(),
            rust_version: std::env!("CARGO_PKG_RUST_VERSION", "unknown").to_string(),
            kagzi_version: env!("CARGO_PKG_VERSION").to_string(),
            started_at: Utc::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workflow_status_display() {
        assert_eq!(WorkflowStatus::Pending.to_string(), "PENDING");
        assert_eq!(WorkflowStatus::Running.to_string(), "RUNNING");
        assert_eq!(WorkflowStatus::Completed.to_string(), "COMPLETED");
        assert_eq!(WorkflowStatus::Failed.to_string(), "FAILED");
        assert_eq!(WorkflowStatus::Sleeping.to_string(), "SLEEPING");
        assert_eq!(WorkflowStatus::Cancelled.to_string(), "CANCELLED");
    }

    #[test]
    fn test_step_status_display() {
        assert_eq!(StepStatus::Completed.to_string(), "COMPLETED");
        assert_eq!(StepStatus::Failed.to_string(), "FAILED");
    }

    #[test]
    fn test_workflow_status_equality() {
        assert_eq!(WorkflowStatus::Pending, WorkflowStatus::Pending);
        assert_ne!(WorkflowStatus::Pending, WorkflowStatus::Running);
    }

    #[test]
    fn test_step_status_equality() {
        assert_eq!(StepStatus::Completed, StepStatus::Completed);
        assert_ne!(StepStatus::Completed, StepStatus::Failed);
    }

    #[test]
    fn test_workflow_status_serialization() {
        let status = WorkflowStatus::Pending;
        let json = serde_json::to_string(&status).unwrap();
        let deserialized: WorkflowStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(status, deserialized);
    }

    #[test]
    fn test_step_status_serialization() {
        let status = StepStatus::Completed;
        let json = serde_json::to_string(&status).unwrap();
        let deserialized: StepStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(status, deserialized);
    }

    #[test]
    fn test_create_workflow_run() {
        let input = serde_json::json!({"user_id": "123"});
        let create_run = CreateWorkflowRun {
            workflow_name: "test-workflow".to_string(),
            workflow_version: Some(1),
            input: input.clone(),
        };

        assert_eq!(create_run.workflow_name, "test-workflow");
        assert_eq!(create_run.workflow_version, Some(1));
        assert_eq!(create_run.input, input);
    }

    #[test]
    fn test_workflow_version_is_active() {
        let active_version = WorkflowVersion {
            workflow_name: "test-workflow".to_string(),
            version: 1,
            is_default: true,
            created_at: Utc::now(),
            deprecated_at: None,
            description: None,
        };

        assert!(active_version.is_active());
        assert!(!active_version.is_deprecated());
    }

    #[test]
    fn test_workflow_version_is_deprecated() {
        let deprecated_version = WorkflowVersion {
            workflow_name: "test-workflow".to_string(),
            version: 1,
            is_default: false,
            created_at: Utc::now(),
            deprecated_at: Some(Utc::now()),
            description: None,
        };

        assert!(!deprecated_version.is_active());
        assert!(deprecated_version.is_deprecated());
    }

    #[test]
    fn test_create_step_run() {
        let run_id = Uuid::new_v4();
        let output = serde_json::json!({"result": "success"});

        let create_step = CreateStepRun {
            workflow_run_id: run_id,
            step_id: "step-1".to_string(),
            input_hash: Some("hash123".to_string()),
            output: Some(output.clone()),
            error: None,
            status: StepStatus::Completed,
            parent_step_id: None,
            parallel_group_id: None,
        };

        assert_eq!(create_step.workflow_run_id, run_id);
        assert_eq!(create_step.step_id, "step-1");
        assert_eq!(create_step.input_hash, Some("hash123".to_string()));
        assert_eq!(create_step.output, Some(output));
        assert_eq!(create_step.error, None);
        assert_eq!(create_step.status, StepStatus::Completed);
        assert_eq!(create_step.parent_step_id, None);
        assert_eq!(create_step.parallel_group_id, None);
    }
}
