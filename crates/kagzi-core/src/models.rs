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
    pub workflow_version: String,
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
    pub workflow_run_id: Uuid,
    pub step_id: String,
    pub input_hash: Option<String>,
    pub output: Option<serde_json::Value>,
    pub error: Option<serde_json::Value>,
    pub status: StepStatus,
    pub attempts: i32,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
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

/// Input for creating a new workflow run
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateWorkflowRun {
    pub workflow_name: String,
    pub workflow_version: Option<String>,
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
            workflow_version: Some("v1".to_string()),
            input: input.clone(),
        };

        assert_eq!(create_run.workflow_name, "test-workflow");
        assert_eq!(create_run.workflow_version, Some("v1".to_string()));
        assert_eq!(create_run.input, input);
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
        };

        assert_eq!(create_step.workflow_run_id, run_id);
        assert_eq!(create_step.step_id, "step-1");
        assert_eq!(create_step.input_hash, Some("hash123".to_string()));
        assert_eq!(create_step.output, Some(output));
        assert_eq!(create_step.error, None);
        assert_eq!(create_step.status, StepStatus::Completed);
    }
}
