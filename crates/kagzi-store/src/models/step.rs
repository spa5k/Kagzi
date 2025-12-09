use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::Type;
use strum::{AsRefStr, Display, EnumString};
use uuid::Uuid;

use super::RetryPolicy;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Display, EnumString, AsRefStr,
)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
#[sqlx(type_name = "text", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum StepKind {
    Function,
    Sleep,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Display, EnumString, AsRefStr,
)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
#[sqlx(type_name = "text", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum StepStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone)]
pub struct StepRun {
    pub attempt_id: Uuid,
    pub run_id: Uuid,
    pub step_id: String,
    pub namespace_id: String,
    pub step_kind: StepKind,
    pub attempt_number: i32,
    pub status: StepStatus,
    pub input: Option<serde_json::Value>,
    pub output: Option<serde_json::Value>,
    pub error: Option<String>,
    pub child_workflow_run_id: Option<Uuid>,
    pub created_at: Option<DateTime<Utc>>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub retry_at: Option<DateTime<Utc>>,
    pub retry_policy: Option<RetryPolicy>,
}

#[derive(Debug, Clone)]
pub struct BeginStepParams {
    pub run_id: Uuid,
    pub step_id: String,
    pub step_kind: StepKind,
    pub input: Option<serde_json::Value>,
    pub retry_policy: Option<RetryPolicy>,
}

#[derive(Debug, Clone)]
pub struct BeginStepResult {
    pub should_execute: bool,
    pub cached_output: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct FailStepParams {
    pub run_id: Uuid,
    pub step_id: String,
    pub error: String,
    pub non_retryable: bool,
    pub retry_after_ms: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct FailStepResult {
    pub scheduled_retry: bool,
    pub retry_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct StepRetryInfo {
    pub attempt_number: i32,
    pub retry_policy: Option<RetryPolicy>,
}

#[derive(Debug, Clone)]
pub struct RetryTriggered {
    pub run_id: Uuid,
    pub step_id: String,
    pub attempt_number: i32,
}

#[derive(Debug, Clone, Default)]
pub struct ListStepsParams {
    pub run_id: Uuid,
    pub namespace_id: String,
    pub step_id: Option<String>,
    pub page_size: i32,
    pub cursor: Option<StepCursor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepCursor {
    pub created_at: DateTime<Utc>,
    pub attempt_id: Uuid,
}
