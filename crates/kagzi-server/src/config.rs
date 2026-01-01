use config::{Config, ConfigError, Environment};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Settings {
    pub database_url: String,
    pub server: ServerSettings,
    pub coordinator: CoordinatorSettings,
    pub worker: WorkerSettings,
    pub payload: PayloadSettings,
    pub queue: QueueSettings,
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

/// Settings for the coordinator background task.
///
/// The coordinator handles:
/// - Firing due cron schedules
/// - Marking stale workers as offline
#[derive(Debug, Clone, Deserialize)]
pub struct CoordinatorSettings {
    /// Interval between coordinator ticks in seconds
    #[serde(default = "default_coordinator_interval_secs")]
    pub interval_secs: u64,
    /// Maximum number of cron schedules to fire per tick
    #[serde(default = "default_coordinator_batch_size")]
    pub batch_size: i32,
    /// How long a worker can go without heartbeat before being marked offline
    #[serde(default = "default_worker_stale_threshold_secs")]
    pub worker_stale_threshold_secs: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkerSettings {
    /// How long to wait for work before returning empty from poll
    #[serde(default = "default_poll_timeout_secs")]
    pub poll_timeout_secs: u64,
    /// How often workers should send heartbeats
    #[serde(default = "default_heartbeat_interval_secs")]
    pub heartbeat_interval_secs: u32,
    /// Initial visibility timeout when claiming a workflow.
    /// Workflows become available to other workers after this time unless extended.
    #[serde(default = "default_visibility_timeout_secs")]
    pub visibility_timeout_secs: i64,
    /// How many seconds to extend visibility on each heartbeat.
    /// Should be greater than heartbeat_interval to allow buffer for network delays.
    #[serde(default = "default_heartbeat_extension_secs")]
    pub heartbeat_extension_secs: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PayloadSettings {
    #[serde(default = "default_payload_warn_threshold_bytes")]
    pub warn_threshold_bytes: usize,
    #[serde(default = "default_payload_max_size_bytes")]
    pub max_size_bytes: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct QueueSettings {
    /// Interval in seconds for cleaning up stale notification channels
    #[serde(default = "default_queue_cleanup_interval_secs")]
    pub cleanup_interval_secs: u64,
    /// Maximum jitter in milliseconds to add when workers wake from notifications
    #[serde(default = "default_queue_poll_jitter_ms")]
    pub poll_jitter_ms: u64,
    /// Maximum time in seconds to retry reconnecting the queue listener before giving up
    #[serde(default = "default_queue_max_reconnect_secs")]
    pub max_reconnect_secs: u64,
}

impl Settings {
    pub fn new() -> Result<Self, ConfigError> {
        let mut builder = Config::builder()
            // Built-in defaults to avoid missing-section errors
            .set_default("server.host", default_host())?
            .set_default("server.port", default_port() as i64)?
            .set_default(
                "server.db_max_connections",
                default_db_max_connections() as i64,
            )?
            .set_default(
                "coordinator.interval_secs",
                default_coordinator_interval_secs() as i64,
            )?
            .set_default(
                "coordinator.batch_size",
                default_coordinator_batch_size() as i64,
            )?
            .set_default(
                "coordinator.worker_stale_threshold_secs",
                default_worker_stale_threshold_secs(),
            )?
            .set_default(
                "worker.poll_timeout_secs",
                default_poll_timeout_secs() as i64,
            )?
            .set_default(
                "worker.heartbeat_interval_secs",
                default_heartbeat_interval_secs() as i64,
            )?
            .set_default(
                "worker.visibility_timeout_secs",
                default_visibility_timeout_secs(),
            )?
            .set_default(
                "worker.heartbeat_extension_secs",
                default_heartbeat_extension_secs(),
            )?
            .set_default(
                "payload.warn_threshold_bytes",
                default_payload_warn_threshold_bytes() as i64,
            )?
            .set_default(
                "payload.max_size_bytes",
                default_payload_max_size_bytes() as i64,
            )?
            .set_default(
                "queue.cleanup_interval_secs",
                default_queue_cleanup_interval_secs() as i64,
            )?
            .set_default(
                "queue.poll_jitter_ms",
                default_queue_poll_jitter_ms() as i64,
            )?
            .set_default(
                "queue.max_reconnect_secs",
                default_queue_max_reconnect_secs() as i64,
            )?
            // Single canonical source: KAGZI_* (flat, single underscore)
            .add_source(Environment::with_prefix("KAGZI").separator("_"));

        if let Ok(db_url) = std::env::var("KAGZI_DB_URL") {
            builder = builder.set_override("database_url", db_url)?;
        }

        builder.build()?.try_deserialize()
    }
}

impl Default for CoordinatorSettings {
    fn default() -> Self {
        Self {
            interval_secs: default_coordinator_interval_secs(),
            batch_size: default_coordinator_batch_size(),
            worker_stale_threshold_secs: default_worker_stale_threshold_secs(),
        }
    }
}

impl Default for WorkerSettings {
    fn default() -> Self {
        Self {
            poll_timeout_secs: default_poll_timeout_secs(),
            heartbeat_interval_secs: default_heartbeat_interval_secs(),
            visibility_timeout_secs: default_visibility_timeout_secs(),
            heartbeat_extension_secs: default_heartbeat_extension_secs(),
        }
    }
}

impl Default for PayloadSettings {
    fn default() -> Self {
        Self {
            warn_threshold_bytes: default_payload_warn_threshold_bytes(),
            max_size_bytes: default_payload_max_size_bytes(),
        }
    }
}

impl Default for QueueSettings {
    fn default() -> Self {
        Self {
            cleanup_interval_secs: default_queue_cleanup_interval_secs(),
            poll_jitter_ms: default_queue_poll_jitter_ms(),
            max_reconnect_secs: default_queue_max_reconnect_secs(),
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

fn default_coordinator_interval_secs() -> u64 {
    5
}

fn default_coordinator_batch_size() -> i32 {
    100
}

fn default_worker_stale_threshold_secs() -> i64 {
    30
}

fn default_poll_timeout_secs() -> u64 {
    60
}

fn default_heartbeat_interval_secs() -> u32 {
    10
}

fn default_visibility_timeout_secs() -> i64 {
    60 // 1 minute - fast orphan recovery, extended by heartbeats
}

fn default_heartbeat_extension_secs() -> i64 {
    45 // Must be > heartbeat_interval to allow buffer for network delays
}

fn default_payload_warn_threshold_bytes() -> usize {
    1024 * 1024
}

fn default_payload_max_size_bytes() -> usize {
    2 * 1024 * 1024
}

fn default_queue_cleanup_interval_secs() -> u64 {
    300 // 5 minutes
}

fn default_queue_poll_jitter_ms() -> u64 {
    100 // 100ms max jitter
}

fn default_queue_max_reconnect_secs() -> u64 {
    300 // 5 minutes max reconnection attempts
}
