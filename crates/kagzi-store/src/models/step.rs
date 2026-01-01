use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use strum::{AsRefStr, Display, EnumString};
use uuid::Uuid;

use super::RetryPolicy;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, EnumString, AsRefStr,
)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
pub enum StepKind {
    Function,
    Sleep,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, EnumString, AsRefStr,
)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
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
    pub input: Option<Vec<u8>>,
    pub output: Option<Vec<u8>>,
    pub error: Option<String>,
    pub child_workflow_run_id: Option<Uuid>,
    pub created_at: Option<DateTime<Utc>>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub retry_policy: Option<RetryPolicy>,
}

#[derive(Debug, Clone)]
pub struct BeginStepParams {
    pub run_id: Uuid,
    pub step_id: String,
    pub step_kind: StepKind,
    pub input: Option<Vec<u8>>,
    pub retry_policy: Option<RetryPolicy>,
}

#[derive(Debug, Clone)]
pub struct BeginStepResult {
    pub should_execute: bool,
    pub cached_output: Option<Vec<u8>>,
}

#[derive(Debug, Clone)]
pub struct FailStepParams {
    pub run_id: Uuid,
    pub step_id: String,
    pub error: String,
    pub non_retryable: bool,
    pub retry_after_ms: Option<i64>,
}

/// Result of failing a step.
///
/// If retry is scheduled, the workflow should be rescheduled with the
/// returned delay. The worker will replay the workflow and cached steps
/// will return instantly until the failed step is re-executed.
#[derive(Debug, Clone)]
pub struct FailStepResult {
    /// Whether a retry was scheduled
    pub scheduled_retry: bool,
    /// If scheduled_retry is true, this is the delay in milliseconds
    /// before the workflow should be rescheduled
    pub schedule_workflow_retry_ms: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct StepRetryInfo {
    pub attempt_number: i32,
    pub retry_policy: Option<RetryPolicy>,
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
