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
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub sleep_until: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
}

/// A step run represents the memoized result of a single step in a workflow
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct StepRun {
    pub workflow_run_id: Uuid,
    pub step_id: String,
    pub input_hash: Option<String>,
    pub output: Option<serde_json::Value>,
    pub error: Option<String>,
    pub status: StepStatus,
    pub attempts: i32,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
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
    pub error: Option<String>,
    pub status: StepStatus,
}
