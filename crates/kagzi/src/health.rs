//! Worker health check module
//!
//! Provides health monitoring for worker instances

use chrono::{DateTime, Utc};
use kagzi_core::models::{Worker, WorkerStatus};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Health status of a worker
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HealthStatus {
    /// Worker is healthy and processing workflows
    Healthy,
    /// Worker is degraded (e.g., slow heartbeat, high error rate)
    Degraded,
    /// Worker is unhealthy and should be restarted
    Unhealthy,
}

impl std::fmt::Display for HealthStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HealthStatus::Healthy => write!(f, "healthy"),
            HealthStatus::Degraded => write!(f, "degraded"),
            HealthStatus::Unhealthy => write!(f, "unhealthy"),
        }
    }
}

/// Health check configuration
#[derive(Debug, Clone)]
pub struct HealthCheckConfig {
    /// Maximum age of heartbeat before considering worker unhealthy (seconds)
    pub heartbeat_timeout_secs: i64,
    /// Maximum age of heartbeat before considering worker degraded (seconds)
    pub heartbeat_warning_secs: i64,
}

impl Default for HealthCheckConfig {
    fn default() -> Self {
        Self {
            heartbeat_timeout_secs: 120,  // 2 minutes
            heartbeat_warning_secs: 60,   // 1 minute
        }
    }
}

/// Worker health information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerHealth {
    /// Worker ID
    pub worker_id: uuid::Uuid,
    /// Worker name
    pub worker_name: String,
    /// Current health status
    pub status: HealthStatus,
    /// Last heartbeat timestamp
    pub last_heartbeat: DateTime<Utc>,
    /// Time since last heartbeat
    pub heartbeat_age_secs: i64,
    /// Worker uptime
    pub uptime_secs: i64,
    /// Current worker status (RUNNING, SHUTTING_DOWN, STOPPED)
    pub worker_status: WorkerStatus,
    /// Number of workflows currently being processed
    pub active_workflows: i64,
    /// Optional error message if unhealthy
    pub error_message: Option<String>,
}

impl WorkerHealth {
    /// Create health info from a worker record
    pub fn from_worker(worker: &Worker, active_workflows: i64) -> Self {
        let now = Utc::now();
        let heartbeat_age = now - worker.last_heartbeat;
        let uptime = worker.uptime();

        Self {
            worker_id: worker.id,
            worker_name: worker.worker_name.clone(),
            status: HealthStatus::Healthy, // Will be calculated
            last_heartbeat: worker.last_heartbeat,
            heartbeat_age_secs: heartbeat_age.num_seconds(),
            uptime_secs: uptime.num_seconds(),
            worker_status: worker.status.clone(),
            active_workflows,
            error_message: None,
        }
    }

    /// Check if worker is healthy
    pub fn is_healthy(&self) -> bool {
        matches!(self.status, HealthStatus::Healthy)
    }

    /// Check if worker is degraded
    pub fn is_degraded(&self) -> bool {
        matches!(self.status, HealthStatus::Degraded)
    }

    /// Check if worker is unhealthy
    pub fn is_unhealthy(&self) -> bool {
        matches!(self.status, HealthStatus::Unhealthy)
    }
}

/// Health checker for workers
pub struct HealthChecker {
    config: HealthCheckConfig,
    db: Arc<kagzi_core::Database>,
}

impl HealthChecker {
    /// Create a new health checker
    pub fn new(db: Arc<kagzi_core::Database>) -> Self {
        Self::with_config(db, HealthCheckConfig::default())
    }

    /// Create a new health checker with custom configuration
    pub fn with_config(db: Arc<kagzi_core::Database>, config: HealthCheckConfig) -> Self {
        Self { config, db }
    }

    /// Check the health of a specific worker
    pub async fn check_worker_health(
        &self,
        worker: &Worker,
        active_workflows: i64,
    ) -> WorkerHealth {
        let mut health = WorkerHealth::from_worker(worker, active_workflows);

        // Check heartbeat age
        if health.heartbeat_age_secs > self.config.heartbeat_timeout_secs {
            health.status = HealthStatus::Unhealthy;
            health.error_message = Some(format!(
                "Heartbeat timeout: {} seconds since last heartbeat (threshold: {})",
                health.heartbeat_age_secs, self.config.heartbeat_timeout_secs
            ));
        } else if health.heartbeat_age_secs > self.config.heartbeat_warning_secs {
            health.status = HealthStatus::Degraded;
            health.error_message = Some(format!(
                "Slow heartbeat: {} seconds since last heartbeat (warning threshold: {})",
                health.heartbeat_age_secs, self.config.heartbeat_warning_secs
            ));
        }

        // Check if worker is stopped
        if worker.is_stopped() {
            health.status = HealthStatus::Unhealthy;
            health.error_message = Some("Worker is stopped".to_string());
        }

        health
    }

    /// Check database connectivity
    pub async fn check_database_health(&self) -> anyhow::Result<()> {
        // Simple query to check if database is accessible
        kagzi_core::queries::get_active_workers(self.db.pool())
            .await?;
        Ok(())
    }

    /// Perform a comprehensive health check
    pub async fn comprehensive_check(&self) -> HealthCheckResult {
        let mut result = HealthCheckResult::default();

        // Check database health
        match self.check_database_health().await {
            Ok(_) => {
                result.database_healthy = true;
            }
            Err(e) => {
                result.database_healthy = false;
                result.errors.push(format!("Database unhealthy: {}", e));
            }
        }

        result
    }
}

/// Comprehensive health check result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthCheckResult {
    /// Is the database healthy?
    pub database_healthy: bool,
    /// List of errors encountered
    pub errors: Vec<String>,
    /// Timestamp of the health check
    pub checked_at: DateTime<Utc>,
}

impl Default for HealthCheckResult {
    fn default() -> Self {
        Self {
            database_healthy: false,
            errors: Vec::new(),
            checked_at: Utc::now(),
        }
    }
}

impl HealthCheckResult {
    /// Check if all systems are healthy
    pub fn is_healthy(&self) -> bool {
        self.database_healthy && self.errors.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};
    use kagzi_core::models::{Worker, WorkerStatus};

    fn create_test_worker(heartbeat_offset_secs: i64) -> Worker {
        Worker {
            id: uuid::Uuid::new_v4(),
            worker_name: "test-worker".to_string(),
            hostname: Some("localhost".to_string()),
            process_id: Some(1234),
            started_at: Utc::now() - Duration::seconds(3600),
            last_heartbeat: Utc::now() - Duration::seconds(heartbeat_offset_secs),
            status: WorkerStatus::Running,
            config: None,
            metadata: None,
            stopped_at: None,
        }
    }

    // Note: These tests are commented out because they require a database connection
    // They can be run as integration tests with a test database

    /*
    #[tokio::test]
    async fn test_healthy_worker() {
        let config = HealthCheckConfig {
            heartbeat_timeout_secs: 120,
            heartbeat_warning_secs: 60,
        };

        // This would need a real database connection
        // let db = Arc::new(kagzi_core::Database::from_pool(pool));
        // let checker = HealthChecker::with_config(db, config);
        // let worker = create_test_worker(10); // 10 seconds ago
        // let health = checker.check_worker_health(&worker, 0).await;

        // assert!(health.is_healthy());
        // assert_eq!(health.status, HealthStatus::Healthy);
        // assert!(health.error_message.is_none());
    }
    */

    #[test]
    fn test_health_status_display() {
        assert_eq!(HealthStatus::Healthy.to_string(), "healthy");
        assert_eq!(HealthStatus::Degraded.to_string(), "degraded");
        assert_eq!(HealthStatus::Unhealthy.to_string(), "unhealthy");
    }
}
