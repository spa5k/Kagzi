use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

pub fn clamp_max_catchup(max_catchup: i32) -> i32 {
    max_catchup.clamp(1, 10_000)
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Schedule {
    pub schedule_id: Uuid,
    pub namespace_id: String,
    pub task_queue: String,
    pub workflow_type: String,
    pub cron_expr: String,
    pub input: serde_json::Value,
    pub context: Option<serde_json::Value>,
    pub enabled: bool,
    pub max_catchup: i32,
    pub next_fire_at: DateTime<Utc>,
    pub last_fired_at: Option<DateTime<Utc>>,
    pub version: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleFiring {
    pub schedule_id: Uuid,
    pub fire_at: DateTime<Utc>,
    pub run_id: Uuid,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CreateSchedule {
    pub namespace_id: String,
    pub task_queue: String,
    pub workflow_type: String,
    pub cron_expr: String,
    pub input: serde_json::Value,
    pub context: Option<serde_json::Value>,
    pub enabled: bool,
    pub max_catchup: i32,
    pub next_fire_at: DateTime<Utc>,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateSchedule {
    pub task_queue: Option<String>,
    pub workflow_type: Option<String>,
    pub cron_expr: Option<String>,
    pub input: Option<serde_json::Value>,
    pub context: Option<serde_json::Value>,
    pub enabled: Option<bool>,
    pub max_catchup: Option<i32>,
    pub next_fire_at: Option<DateTime<Utc>>,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ListSchedulesParams {
    pub namespace_id: String,
    pub task_queue: Option<String>,
    pub page_size: i32,
    pub cursor: Option<ScheduleCursor>,
}

#[derive(Debug, Clone)]
pub struct ScheduleCursor {
    pub created_at: DateTime<Utc>,
    pub schedule_id: Uuid,
}
