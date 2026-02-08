use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use strum::{AsRefStr, Display, EnumString};
use uuid::Uuid;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, EnumString, AsRefStr,
)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
pub enum WorkerStatus {
    Online,
    Draining,
    Offline,
}

#[derive(Debug, Clone)]
pub struct Worker {
    pub worker_id: Uuid,
    pub namespace: String,
    pub task_queue: String,
    pub status: WorkerStatus,

    pub hostname: Option<String>,
    pub pid: Option<i32>,
    pub version: Option<String>,

    pub workflow_types: Vec<String>,

    pub registered_at: DateTime<Utc>,
    pub last_heartbeat_at: DateTime<Utc>,
    pub deregistered_at: Option<DateTime<Utc>>,

    pub labels: serde_json::Value,
    pub queue_concurrency_limit: Option<i32>,
    pub workflow_type_concurrency: Vec<WorkflowTypeConcurrency>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowTypeConcurrency {
    pub workflow_type: String,
    pub max_concurrent: i32,
}

#[derive(Debug, Clone)]
pub struct RegisterWorkerParams {
    pub namespace: String,
    pub task_queue: String,
    pub workflow_types: Vec<String>,
    pub hostname: Option<String>,
    pub pid: Option<i32>,
    pub version: Option<String>,
    pub labels: serde_json::Value,
    pub queue_concurrency_limit: Option<i32>,
    pub workflow_type_concurrency: Vec<WorkflowTypeConcurrency>,
}

#[derive(Debug, Clone)]
pub struct WorkerHeartbeatParams {
    pub worker_id: Uuid,
}

#[derive(Debug, Clone, Default)]
pub struct ListWorkersParams {
    pub namespace: String,
    pub task_queue: Option<String>,
    pub filter_status: Option<WorkerStatus>,
    pub page_size: i32,
    pub cursor: Option<WorkerCursor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerCursor {
    pub worker_id: Uuid,
}
