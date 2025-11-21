//! Worker implementation for executing workflows

use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use kagzi_core::{queries, Database, WorkflowStatus};

use crate::client::WorkflowFn;
use crate::context::WorkflowContext;

/// Worker configuration
#[derive(Debug, Clone)]
pub struct WorkerConfig {
    /// How often to poll for new workflows (milliseconds)
    pub poll_interval_ms: u64,
    /// How long to hold a worker lease (seconds)
    pub lease_duration_secs: i64,
    /// Worker ID (auto-generated if not provided)
    pub worker_id: Option<String>,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            poll_interval_ms: 100,
            lease_duration_secs: 60,
            worker_id: None,
        }
    }
}

/// Worker that polls and executes workflows
pub struct Worker {
    db: Arc<Database>,
    workflows: Arc<RwLock<HashMap<String, WorkflowFn<serde_json::Value>>>>,
    config: WorkerConfig,
    worker_id: String,
    running: Arc<RwLock<bool>>,
}

impl Worker {
    /// Create a new worker with default configuration
    pub fn new(
        db: Arc<Database>,
        workflows: Arc<RwLock<HashMap<String, WorkflowFn<serde_json::Value>>>>,
    ) -> Self {
        Self::with_config(db, workflows, WorkerConfig::default())
    }

    /// Create a new worker with custom configuration
    pub fn with_config(
        db: Arc<Database>,
        workflows: Arc<RwLock<HashMap<String, WorkflowFn<serde_json::Value>>>>,
        config: WorkerConfig,
    ) -> Self {
        let worker_id = config
            .worker_id
            .clone()
            .unwrap_or_else(|| format!("worker-{}", Uuid::new_v4()));

        info!("Created worker: {}", worker_id);

        Self {
            db,
            workflows,
            config,
            worker_id,
            running: Arc::new(RwLock::new(false)),
        }
    }

    /// Start the worker (blocking)
    ///
    /// This will run indefinitely, polling for workflows and executing them.
    pub async fn start(&self) -> Result<()> {
        {
            let mut running = self.running.write().await;
            if *running {
                return Err(anyhow::anyhow!("Worker is already running"));
            }
            *running = true;
        }

        info!("Worker {} starting...", self.worker_id);

        loop {
            // Check if we should stop
            {
                let running = self.running.read().await;
                if !*running {
                    info!("Worker {} stopping...", self.worker_id);
                    break;
                }
            }

            // Poll for next workflow
            match self.poll_and_execute().await {
                Ok(executed) => {
                    if !executed {
                        // No workflows available, sleep before polling again
                        tokio::time::sleep(Duration::from_millis(self.config.poll_interval_ms))
                            .await;
                    }
                }
                Err(e) => {
                    error!("Error in worker loop: {}", e);
                    tokio::time::sleep(Duration::from_millis(self.config.poll_interval_ms)).await;
                }
            }
        }

        info!("Worker {} stopped", self.worker_id);
        Ok(())
    }

    /// Stop the worker
    pub async fn stop(&self) {
        let mut running = self.running.write().await;
        *running = false;
        info!("Worker {} stop requested", self.worker_id);
    }

    /// Poll for a workflow and execute it
    ///
    /// Returns true if a workflow was executed, false if none were available
    async fn poll_and_execute(&self) -> Result<bool> {
        // Poll for next workflow
        let workflow_run = match queries::poll_next_workflow(self.db.pool()).await? {
            Some(run) => run,
            None => return Ok(false), // No workflows available
        };

        debug!(
            "Worker {} picked up workflow {}",
            self.worker_id, workflow_run.id
        );

        // Try to acquire lease
        match queries::acquire_worker_lease(
            self.db.pool(),
            workflow_run.id,
            &self.worker_id,
            self.config.lease_duration_secs,
        )
        .await
        {
            Ok(_) => {
                // Successfully acquired lease, execute workflow
                self.execute_workflow(workflow_run.id).await?;
                Ok(true)
            }
            Err(e) => {
                // Failed to acquire lease (another worker got it)
                debug!(
                    "Failed to acquire lease for workflow {}: {}",
                    workflow_run.id, e
                );
                Ok(false)
            }
        }
    }

    /// Execute a workflow
    async fn execute_workflow(&self, workflow_run_id: Uuid) -> Result<()> {
        info!("Executing workflow: {}", workflow_run_id);

        // Get workflow run details
        let workflow_run = queries::get_workflow_run(self.db.pool(), workflow_run_id).await?;

        // Update status to RUNNING
        queries::update_workflow_status(
            self.db.pool(),
            workflow_run_id,
            WorkflowStatus::Running,
            None,
        )
        .await?;

        // Look up workflow function
        let workflows = self.workflows.read().await;
        let workflow_fn = workflows.get(&workflow_run.workflow_name).ok_or_else(|| {
            anyhow::anyhow!(
                "Workflow function not registered: {}",
                workflow_run.workflow_name
            )
        })?;

        let workflow_fn = workflow_fn.clone();
        drop(workflows); // Release lock

        // Create workflow context
        let ctx = WorkflowContext::new(
            workflow_run_id,
            workflow_run.workflow_name.clone(),
            self.db.clone(),
        );

        // Execute workflow
        let result = workflow_fn(ctx, workflow_run.input.clone()).await;

        // Handle result
        match result {
            Ok(output) => {
                // Store output
                queries::update_workflow_output(self.db.pool(), workflow_run_id, output).await?;

                // Mark as completed
                queries::update_workflow_status(
                    self.db.pool(),
                    workflow_run_id,
                    WorkflowStatus::Completed,
                    None,
                )
                .await?;

                info!("Workflow {} completed successfully", workflow_run_id);
            }
            Err(e) => {
                let error_msg = e.to_string();

                // Check if this is a sleep signal
                if error_msg == "__SLEEP__" {
                    info!("Workflow {} is sleeping", workflow_run_id);
                    // Status already updated by sleep() method
                } else {
                    // Actual error
                    warn!("Workflow {} failed: {}", workflow_run_id, error_msg);

                    queries::update_workflow_status(
                        self.db.pool(),
                        workflow_run_id,
                        WorkflowStatus::Failed,
                        Some(error_msg),
                    )
                    .await?;
                }
            }
        }

        // Release lease
        queries::release_worker_lease(self.db.pool(), workflow_run_id).await?;

        Ok(())
    }

    /// Get worker ID
    pub fn worker_id(&self) -> &str {
        &self.worker_id
    }
}
