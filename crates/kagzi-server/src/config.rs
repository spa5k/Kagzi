use config::{Config, ConfigError, Environment};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Settings {
    pub database_url: String,
    pub server: ServerSettings,
    pub scheduler: SchedulerSettings,
    pub watchdog: WatchdogSettings,
    pub worker: WorkerSettings,
    pub payload: PayloadSettings,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerSettings {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_db_max_connections")]
    pub db_max_connections: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SchedulerSettings {
    #[serde(default = "default_scheduler_interval_secs")]
    pub interval_secs: u64,
    #[serde(default = "default_scheduler_batch_size")]
    pub batch_size: i32,
    #[serde(default = "default_max_workflows_per_tick")]
    pub max_workflows_per_tick: i32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WatchdogSettings {
    #[serde(default = "default_watchdog_interval_secs")]
    pub interval_secs: u64,
    #[serde(default = "default_worker_stale_threshold_secs")]
    pub worker_stale_threshold_secs: i64,
    #[serde(default = "default_counter_reconcile_interval_secs")]
    pub counter_reconcile_interval_secs: u64,
    #[serde(default = "default_wake_sleeping_batch_size")]
    pub wake_sleeping_batch_size: i32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkerSettings {
    #[serde(default = "default_poll_timeout_secs")]
    pub poll_timeout_secs: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PayloadSettings {
    #[serde(default = "default_payload_warn_threshold_bytes")]
    pub warn_threshold_bytes: usize,
    #[serde(default = "default_payload_max_size_bytes")]
    pub max_size_bytes: usize,
}

impl Settings {
    pub fn new() -> Result<Self, ConfigError> {
        Config::builder()
            // Unprefixed env (e.g., DATABASE_URL)
            .add_source(Environment::default())
            // Legacy single-underscore prefix support: KAGZI_SCHEDULER_INTERVAL_SECS
            .add_source(Environment::with_prefix("KAGZI").separator("_"))
            // Preferred double-underscore hierarchical prefix: KAGZI__SCHEDULER__INTERVAL_SECS
            .add_source(Environment::with_prefix("KAGZI").separator("__"))
            .build()?
            .try_deserialize()
    }
}

impl Default for WorkerSettings {
    fn default() -> Self {
        Self {
            poll_timeout_secs: default_poll_timeout_secs(),
        }
    }
}

fn default_host() -> String {
    "0.0.0.0".to_string()
}

fn default_port() -> u16 {
    50051
}

fn default_db_max_connections() -> u32 {
    50
}

fn default_scheduler_interval_secs() -> u64 {
    5
}

fn default_scheduler_batch_size() -> i32 {
    100
}

fn default_max_workflows_per_tick() -> i32 {
    1000
}

fn default_watchdog_interval_secs() -> u64 {
    1
}

fn default_worker_stale_threshold_secs() -> i64 {
    30
}

fn default_counter_reconcile_interval_secs() -> u64 {
    300
}

fn default_poll_timeout_secs() -> u64 {
    60
}

fn default_wake_sleeping_batch_size() -> i32 {
    100
}

fn default_payload_warn_threshold_bytes() -> usize {
    1024 * 1024
}

fn default_payload_max_size_bytes() -> usize {
    2 * 1024 * 1024
}
