use std::time::Duration;

use backon::{BackoffBuilder, ExponentialBuilder};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use strum::{AsRefStr, Display, EnumString};
use uuid::Uuid;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, EnumString, AsRefStr,
)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
pub enum WorkflowStatus {
    Pending,
    Running,
    Sleeping,
    Completed,
    Failed,
    Cancelled,
    Scheduled,
    Paused,
}

impl WorkflowStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }

    pub fn can_cancel(&self) -> bool {
        matches!(self, Self::Pending | Self::Running | Self::Sleeping)
    }

    pub fn is_schedule_template(&self) -> bool {
        matches!(self, Self::Scheduled | Self::Paused)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryPolicy {
    pub maximum_attempts: i32,
    pub initial_interval_ms: i64,
    pub backoff_coefficient: f64,
    pub maximum_interval_ms: i64,
    #[serde(default)]
    pub non_retryable_errors: Vec<String>,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            maximum_attempts: 5,
            initial_interval_ms: 1000,
            backoff_coefficient: 2.0,
            maximum_interval_ms: 60000,
            non_retryable_errors: vec![],
        }
    }
}

impl RetryPolicy {
    pub fn calculate_delay_ms(&self, attempt: i32) -> i64 {
        let mut backoff = ExponentialBuilder::default()
            .with_min_delay(Duration::from_millis(self.initial_interval_ms as u64))
            .with_max_delay(Duration::from_millis(self.maximum_interval_ms as u64))
            .with_jitter()
            .build();

        // Advance to requested attempt number (attempt 1 uses initial interval).
        for _ in 0..attempt.saturating_sub(1) {
            backoff.next();
        }

        let delay = backoff
            .next()
            .unwrap_or(Duration::from_millis(self.maximum_interval_ms as u64));
        delay.as_millis() as i64
    }

    pub fn is_non_retryable(&self, error: &str) -> bool {
        self.non_retryable_errors
            .iter()
            .any(|e| error.starts_with(e))
    }

    pub fn should_retry(&self, current_attempt: i32) -> bool {
        self.maximum_attempts < 0 || current_attempt < self.maximum_attempts
    }
}

#[derive(Debug, Clone)]
pub struct WorkflowRun {
    pub run_id: Uuid,
    pub namespace_id: String,
    pub external_id: String,
    pub task_queue: String,
    pub workflow_type: String,
    pub status: WorkflowStatus,
    pub input: Vec<u8>,
    pub output: Option<Vec<u8>>,
    pub locked_by: Option<String>,
    pub attempts: i32,
    pub error: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    /// When this workflow becomes available for claiming.
    /// Used for scheduling sleep, retry backoff, and visibility timeout.
    pub available_at: Option<DateTime<Utc>>,
    pub version: Option<String>,
    pub parent_step_attempt_id: Option<String>,
    pub retry_policy: Option<RetryPolicy>,
    pub cron_expr: Option<String>,
    pub schedule_id: Option<Uuid>,
    /// Last time this scheduled workflow fired.
    /// Used to calculate missed runs for backfill-aware scheduling.
    /// None for first-time schedules.
    pub last_fired_at: Option<DateTime<Utc>>,
    /// Maximum number of missed runs to catch up on during backfill.
    /// - 0: Never backfill, skip all missed runs to current time
    /// - N: Catch up at most N missed runs (e.g., 50 = catch up to 50 most recent runs)
    ///
    /// This prevents resource exhaustion when a schedule has been down for a long time.
    /// For example, with a 1-minute schedule down for a day (1440 missed runs),
    /// max_catchup=50 ensures only the 50 most recent runs execute.
    pub max_catchup: i32,
}

#[derive(Debug, Clone)]
pub struct CreateWorkflow {
    pub run_id: Uuid,
    pub external_id: String,
    pub task_queue: String,
    pub workflow_type: String,
    pub input: Vec<u8>,
    pub namespace_id: String,
    pub version: String,
    pub retry_policy: Option<RetryPolicy>,
    pub cron_expr: Option<String>,
    pub schedule_id: Option<Uuid>,
}

#[derive(Debug, Clone, Default)]
pub struct ListWorkflowsParams {
    pub namespace_id: String,
    pub filter_status: Option<String>,
    pub page_size: i32,
    pub cursor: Option<WorkflowCursor>,
    pub schedule_id: Option<Uuid>,
}

#[derive(Debug, Clone)]
pub struct WorkflowCursor {
    pub created_at: DateTime<Utc>,
    pub run_id: Uuid,
}

#[derive(Debug, Clone)]
pub struct ClaimedWorkflow {
    pub run_id: Uuid,
    pub workflow_type: String,
    pub input: Vec<u8>,
    pub locked_by: Option<String>,
}

#[derive(Debug, Clone)]
pub struct WorkflowExistsResult {
    pub exists: bool,
    pub status: Option<WorkflowStatus>,
    pub locked_by: Option<String>,
}

#[derive(Debug, Clone)]
pub struct WorkCandidate {
    pub run_id: Uuid,
    pub workflow_type: String,
    pub available_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct WorkflowPayload {
    pub run_id: Uuid,
    pub input: Vec<u8>,
    pub output: Option<Vec<u8>>,
    pub context: Option<serde_json::Value>,
}
