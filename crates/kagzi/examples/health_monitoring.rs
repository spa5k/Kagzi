//! Example: Worker Health Monitoring
//!
//! This example demonstrates V2 health monitoring features:
//! - Worker health checks
//! - Heartbeat monitoring
//! - Health status reporting
//! - Database health checks
//!
//! Run with: cargo run --example health_monitoring

use kagzi::{
    HealthCheckConfig, HealthChecker, HealthStatus, Kagzi, WorkflowContext,
};
use serde::{Deserialize, Serialize};
use std::env;
use std::time::Duration;
use tokio::time::interval;
use tracing::info;

#[derive(Debug, Serialize, Deserialize)]
struct MonitoringInput {
    task_count: i32,
}

#[derive(Debug, Serialize, Deserialize)]
struct MonitoringOutput {
    tasks_completed: i32,
}

/// Simple workflow for monitoring demonstration
async fn monitored_workflow(
    ctx: WorkflowContext,
    input: MonitoringInput,
) -> anyhow::Result<MonitoringOutput> {
    info!("Starting monitored workflow with {} tasks", input.task_count);

    for i in 1..=input.task_count {
        ctx.step(&format!("task-{}", i), async move {
            info!("  Processing task {}", i);
            tokio::time::sleep(Duration::from_secs(2)).await;
            Ok::<_, anyhow::Error>(())
        })
        .await?;
    }

    Ok(MonitoringOutput {
        tasks_completed: input.task_count,
    })
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("info,kagzi=debug,sqlx=warn")
        .init();

    let database_url = env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://kagzi:kagzi_dev_password@localhost:5432/kagzi".to_string());

    let kagzi = Kagzi::connect(&database_url).await?;

    kagzi
        .register_workflow("monitored-task", monitored_workflow)
        .await;

    info!("🏥 Starting health monitoring example\n");

    // Configure health checker with custom thresholds
    let health_config = HealthCheckConfig {
        heartbeat_timeout_secs: 60,  // Consider unhealthy after 60s
        heartbeat_warning_secs: 30,  // Warn after 30s
    };

    let health_checker = HealthChecker::with_config(kagzi.db_handle(), health_config);

    // Check database health
    info!("🔍 Checking database health...");
    match health_checker.check_database_health().await {
        Ok(()) => info!("  ✓ Database is healthy\n"),
        Err(e) => {
            info!("  ✗ Database health check failed: {}", e);
            return Err(e);
        }
    }

    // Start some workflows
    info!("📋 Starting monitored workflows...");
    for i in 1..=3 {
        let input = MonitoringInput { task_count: 2 };
        let handle = kagzi.start_workflow("monitored-task", input).await?;
        info!("  Started workflow {}: {}", i, handle.run_id());
    }
    info!("");

    // Start worker with health monitoring
    info!("🚀 Starting worker with health monitoring enabled...");
    let worker = kagzi
        .create_worker_builder()
        .worker_id(format!("monitored-worker-{}", uuid::Uuid::new_v4()))
        .heartbeat_interval_secs(10) // Heartbeat every 10 seconds
        .max_concurrent_workflows(2)
        .build();

    // Spawn worker
    let worker_handle = tokio::spawn(async move {
        worker.start().await.expect("Worker failed");
    });

    // Monitor worker health periodically
    let health_monitor = tokio::spawn({
        let kagzi = kagzi.clone();
        async move {
            let mut health_interval = interval(Duration::from_secs(15));
            let health_checker = HealthChecker::with_config(
                kagzi.db_handle(),
                HealthCheckConfig {
                    heartbeat_timeout_secs: 60,
                    heartbeat_warning_secs: 30,
                },
            );

            for iteration in 1..=5 {
                health_interval.tick().await;

                info!("\n🏥 Health Check #{}", iteration);
                info!("================");

                // Get all active workers
                match kagzi_core::queries::get_active_workers(kagzi.db_handle().pool()).await {
                    Ok(workers) => {
                        info!("Active workers: {}", workers.len());

                        for worker in workers {
                            // Count active workflows for this worker
                            let active_workflows = match kagzi_core::queries::count_active_workflows_for_worker(
                                kagzi.db_handle().pool(),
                                worker.id,
                            )
                            .await
                            {
                                Ok(count) => count,
                                Err(e) => {
                                    info!("  Error counting workflows: {}", e);
                                    0
                                }
                            };

                            let worker_health = health_checker
                                .check_worker_health(&worker, active_workflows)
                                .await;

                            info!("\n  Worker: {}", worker_health.worker_name);
                            info!("    Status: {}", format_health_status(&worker_health.status));
                            info!("    Uptime: {}s", worker_health.uptime_secs);
                            info!(
                                "    Last heartbeat: {}s ago",
                                worker_health.heartbeat_age_secs
                            );
                            info!("    Active workflows: {}", worker_health.active_workflows);
                            info!("    Worker status: {:?}", worker_health.worker_status);

                            if let Some(error) = &worker_health.error_message {
                                info!("    ⚠ Warning: {}", error);
                            }
                        }
                    }
                    Err(e) => {
                        info!("  ✗ Failed to get active workers: {}", e);
                    }
                }

                // Comprehensive health check
                let comprehensive = health_checker.comprehensive_check().await;
                info!("\n  Comprehensive Health:");
                info!("    Database: {}", if comprehensive.database_healthy { "✓" } else { "✗" });
                info!("    Overall: {}", if comprehensive.is_healthy() { "✓ Healthy" } else { "✗ Unhealthy" });

                if !comprehensive.errors.is_empty() {
                    info!("    Errors:");
                    for error in &comprehensive.errors {
                        info!("      - {}", error);
                    }
                }
            }

            info!("\n✅ Health monitoring completed");
        }
    });

    // Wait for monitoring to complete
    let _ = tokio::join!(health_monitor);

    // Signal worker to stop
    drop(worker_handle);

    info!("\n💡 Key Features Demonstrated:");
    info!("  • Worker health monitoring with configurable thresholds");
    info!("  • Heartbeat age tracking");
    info!("  • Database health checks");
    info!("  • Active workflow counting");
    info!("  • Comprehensive system health reporting");

    Ok(())
}

fn format_health_status(status: &HealthStatus) -> String {
    match status {
        HealthStatus::Healthy => "✓ Healthy".to_string(),
        HealthStatus::Degraded => "⚠ Degraded".to_string(),
        HealthStatus::Unhealthy => "✗ Unhealthy".to_string(),
    }
}
