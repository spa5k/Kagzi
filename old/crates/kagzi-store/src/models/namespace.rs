use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Namespace represents a multi-tenancy isolation boundary.
/// Namespaces are enabled/disabled, not deleted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Namespace {
    pub id: Uuid,
    pub namespace: String,
    pub display_name: String,
    pub description: Option<String>,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// CreateNamespace contains parameters for creating a new namespace.
#[derive(Debug, Clone)]
pub struct CreateNamespace {
    pub namespace: String,
    pub display_name: String,
    pub description: Option<String>,
}

/// UpdateNamespace contains parameters for updating a namespace.
#[derive(Debug, Clone)]
pub struct UpdateNamespace {
    pub display_name: Option<String>,
    pub description: Option<String>,
}

/// NamespaceStats contains statistics for a namespace.
#[derive(Debug, Clone, Default)]
pub struct NamespaceStats {
    pub total_workflows: i64,
    pub pending_workflows: i64,
    pub running_workflows: i64,
    pub completed_workflows: i64,
    pub failed_workflows: i64,
    pub total_workers: i64,
    pub online_workers: i64,
    pub total_schedules: i64,
    pub enabled_schedules: i64,
    pub workflows_by_status: Vec<WorkflowStatusCount>,
    pub workflows_by_type: Vec<WorkflowTypeCount>,
}

/// WorkflowStatusCount represents count of workflows by status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowStatusCount {
    pub status: String,
    pub count: i64,
}

/// WorkflowTypeCount represents count of workflows by type.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowTypeCount {
    pub workflow_type: String,
    pub total_runs: i64,
    pub active_runs: i64,
    pub task_queues: Vec<String>,
}

/// WorkflowTypeInfo represents detailed information about a workflow type.
#[derive(Debug, Clone)]
pub struct WorkflowTypeInfo {
    pub workflow_type: String,
    pub total_runs: i64,
    pub active_runs: i64,
    pub task_queues: Vec<String>,
}
