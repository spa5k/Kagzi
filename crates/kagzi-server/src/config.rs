use config::{Config, ConfigError, Environment};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum QueueBackend {
    #[default]
    Postgres,
    Nats,
    Kafka,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Settings {
    pub database_url: String,
    #[serde(default)]
    pub server: ServerSettings,
    #[serde(default)]
    pub coordinator: CoordinatorSettings,
    #[serde(default)]
    pub worker: WorkerSettings,
    #[serde(default)]
    pub payload: PayloadSettings,
    #[serde(default)]
    pub queue: QueueSettings,
    #[serde(default)]
    pub telemetry: TelemetrySettings,
    #[serde(default)]
    pub worker_telemetry: WorkerTelemetrySettings,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerSettings {
    pub host: String,
    pub port: u16,
    pub db_max_connections: u32,
}

impl Default for ServerSettings {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".to_string(),
            port: 50051,
            db_max_connections: 50,
        }
    }
}

/// Settings for the coordinator background task.
///
/// The coordinator handles:
/// - Firing due cron schedules
/// - Marking stale workers as offline
#[derive(Debug, Clone, Deserialize)]
pub struct CoordinatorSettings {
    /// Interval between coordinator ticks in seconds
    pub interval_secs: u64,
    /// Maximum number of cron schedules to fire per tick
    pub batch_size: i32,
    /// How long a worker can go without heartbeat before being marked offline
    pub worker_stale_threshold_secs: i64,
    /// Default max_catchup for new schedules (controls how many missed runs to backfill)
    /// Similar to Temporal's catchup window concept
    /// Value 0 = never backfill, skip to current time
    pub default_max_catchup: i32,
    /// Maximum number of schedule instances to create per second (global rate limit)
    /// Prevents resource exhaustion during large backfills
    pub max_backfill_per_second: i32,
}

impl Default for CoordinatorSettings {
    fn default() -> Self {
        Self {
            interval_secs: 5,
            batch_size: 100,
            worker_stale_threshold_secs: 30,
            default_max_catchup: 50, // Catch up to 50 missed runs by default
            max_backfill_per_second: 10, // Max 10 schedule instances per second globally
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkerSettings {
    /// How long to wait for work before returning empty from poll
    pub poll_timeout_secs: u64,
    /// How often workers should send heartbeats
    pub heartbeat_interval_secs: u32,
    /// Initial visibility timeout when claiming a workflow.
    /// Workflows become available to other workers after this time unless extended.
    pub visibility_timeout_secs: i64,
    /// How many seconds to extend visibility on each heartbeat.
    /// Should be greater than heartbeat_interval to allow buffer for network delays.
    pub heartbeat_extension_secs: i64,
}

impl Default for WorkerSettings {
    fn default() -> Self {
        Self {
            poll_timeout_secs: 60,
            heartbeat_interval_secs: 10,
            visibility_timeout_secs: 60, // 1 minute - fast orphan recovery, extended by heartbeats
            heartbeat_extension_secs: 45, // Must be > heartbeat_interval to allow buffer for network delays
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct PayloadSettings {
    pub warn_threshold_bytes: usize,
    pub max_size_bytes: usize,
}

impl Default for PayloadSettings {
    fn default() -> Self {
        Self {
            warn_threshold_bytes: 1024 * 1024, // 1 MB
            max_size_bytes: 2 * 1024 * 1024,   // 2 MB
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct QueueSettings {
    pub backend: QueueBackend,
    /// Broadcast channel capacity for queue notifications
    pub channel_capacity: usize,
    /// Interval in seconds for cleaning up stale notification channels
    pub cleanup_interval_secs: u64,
    /// Maximum jitter in milliseconds to add when workers wake from notifications
    pub poll_jitter_ms: u64,
    /// Maximum time in seconds to retry reconnecting the queue listener before giving up
    pub max_reconnect_secs: u64,

    /// NATS connection URL (used when backend=nats)
    pub nats_url: String,
    /// NATS subject prefix (used when backend=nats)
    pub nats_subject_prefix: String,
    /// NATS queue group for worker direct-subscribe (used when backend=nats)
    pub nats_queue_group: String,

    /// Kafka bootstrap brokers (used when backend=kafka)
    pub kafka_brokers: String,
    /// Kafka topic for work wakeups (used when backend=kafka)
    pub kafka_topic: String,
    /// Kafka consumer group id prefix (used when backend=kafka)
    pub kafka_group_id_prefix: String,
}

impl Default for QueueSettings {
    fn default() -> Self {
        Self {
            backend: QueueBackend::Postgres,
            channel_capacity: 64,       // Broadcast channel capacity per queue
            cleanup_interval_secs: 300, // 5 minutes
            poll_jitter_ms: 100,        // 100ms max jitter
            max_reconnect_secs: 300,    // 5 minutes max reconnection attempts

            nats_url: "localhost:4222".to_string(),
            nats_subject_prefix: "kagzi.work".to_string(),
            nats_queue_group: "kagzi-workers".to_string(),

            kafka_brokers: "localhost:9094".to_string(),
            kafka_topic: "kagzi-work".to_string(),
            kafka_group_id_prefix: "kagzi-workers".to_string(),
        }
    }
}

/// Settings for telemetry (tracing, logs, metrics).
#[derive(Debug, Clone, Deserialize)]
pub struct TelemetrySettings {
    /// Whether OpenTelemetry exporters are enabled (default: false)
    /// When false, only tracing logs are emitted (cleaner for development)
    /// When true, OpenTelemetry spans/metrics are exported to stdout
    pub enabled: bool,
    /// Service name for telemetry (default: "kagzi-server")
    pub service_name: String,
    /// Log level filter (default: "info")
    pub log_level: String,
    /// Log format: "pretty" or "json" (default: "pretty")
    pub log_format: String,
}

impl Default for TelemetrySettings {
    fn default() -> Self {
        Self {
            enabled: false,
            service_name: "kagzi-server".to_string(),
            log_level: "info".to_string(),
            log_format: "pretty".to_string(),
        }
    }
}

/// Settings for worker telemetry ingestion and retention (UI/analytics).
#[derive(Debug, Clone, Deserialize)]
pub struct WorkerTelemetrySettings {
    /// Whether ingestion APIs are enabled.
    pub enabled: bool,
    /// How many days of telemetry events to retain (pruned on an interval).
    pub events_retention_days: i64,
    /// How often to prune telemetry events (seconds).
    pub prune_interval_secs: u64,
    /// Max events accepted per ReportWorkerEvents call.
    pub max_events_per_report: usize,
}

impl Default for WorkerTelemetrySettings {
    fn default() -> Self {
        Self {
            enabled: true,
            events_retention_days: 14,
            prune_interval_secs: 60,
            max_events_per_report: 500,
        }
    }
}

impl Settings {
    pub fn new() -> Result<Self, ConfigError> {
        let mut builder = Config::builder()
            .add_source(config::File::with_name("config/default").required(false))
            .add_source(Environment::with_prefix("KAGZI").separator("_"));

        if let Ok(url) = std::env::var("KAGZI_DB_URL") {
            builder = builder.set_override("database_url", url)?;
        }

        builder.build()?.try_deserialize()
    }
}
