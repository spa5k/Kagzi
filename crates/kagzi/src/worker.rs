//! Worker implementation for executing workflows
//!
//! This module provides a production-ready worker with:
//! - Graceful shutdown (SIGTERM, SIGINT)
//! - Heartbeat mechanism
//! - Concurrent workflow limiting
//! - Worker lifecycle management
//! - Health monitoring

use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::signal;
use tokio::sync::{RwLock, Semaphore};
use tokio::time::interval;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use kagzi_core::{queries, Database, ErrorKind, StepError, WorkflowStatus};
use kagzi_core::models::{WorkerEventType, WorkerMetadata, WorkerStatus};

use crate::client::WorkflowFn;
use crate::context::WorkflowContext;
use crate::versioning::WorkflowRegistry;

/// Worker configuration
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WorkerConfig {
    /// How often to poll for new workflows (milliseconds)
    pub poll_interval_ms: u64,
    /// How long to hold a worker lease (seconds)
    pub lease_duration_secs: i64,
    /// Worker ID (auto-generated if not provided)
    pub worker_id: Option<String>,
    /// Maximum number of concurrent workflows
    pub max_concurrent_workflows: usize,
    /// Heartbeat interval (seconds)
    pub heartbeat_interval_secs: u64,
    /// Graceful shutdown timeout (seconds) - how long to wait for workflows to complete
    pub graceful_shutdown_timeout_secs: u64,
    /// Worker hostname (auto-detected if not provided)
    pub hostname: Option<String>,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            poll_interval_ms: 100,
            lease_duration_secs: 60,
            worker_id: None,
            max_concurrent_workflows: 10,
            heartbeat_interval_secs: 30,
            graceful_shutdown_timeout_secs: 300, // 5 minutes
            hostname: None,
        }
    }
}

/// Builder for creating workers with custom configuration
pub struct WorkerBuilder {
    db: Arc<Database>,
    workflows: Arc<RwLock<HashMap<String, WorkflowFn<serde_json::Value>>>>,
    registry: Arc<WorkflowRegistry>,
    config: WorkerConfig,
}

impl WorkerBuilder {
    /// Create a new worker builder
    pub fn new(
        db: Arc<Database>,
        workflows: Arc<RwLock<HashMap<String, WorkflowFn<serde_json::Value>>>>,
        registry: Arc<WorkflowRegistry>,
    ) -> Self {
        Self {
            db,
            workflows,
            registry,
            config: WorkerConfig::default(),
        }
    }

    /// Set the poll interval
    pub fn poll_interval_ms(mut self, ms: u64) -> Self {
        self.config.poll_interval_ms = ms;
        self
    }

    /// Set the lease duration
    pub fn lease_duration_secs(mut self, secs: i64) -> Self {
        self.config.lease_duration_secs = secs;
        self
    }

    /// Set the worker ID
    pub fn worker_id(mut self, id: String) -> Self {
        self.config.worker_id = Some(id);
        self
    }

    /// Set the maximum concurrent workflows
    pub fn max_concurrent_workflows(mut self, max: usize) -> Self {
        self.config.max_concurrent_workflows = max;
        self
    }

    /// Set the heartbeat interval
    pub fn heartbeat_interval_secs(mut self, secs: u64) -> Self {
        self.config.heartbeat_interval_secs = secs;
        self
    }

    /// Set the graceful shutdown timeout
    pub fn graceful_shutdown_timeout_secs(mut self, secs: u64) -> Self {
        self.config.graceful_shutdown_timeout_secs = secs;
        self
    }

    /// Set the hostname
    pub fn hostname(mut self, hostname: String) -> Self {
        self.config.hostname = Some(hostname);
        self
    }

    /// Build the worker
    pub fn build(self) -> Worker {
        Worker::with_config(self.db, self.workflows, self.registry, self.config)
    }
}

/// Worker that polls and executes workflows
pub struct Worker {
    db: Arc<Database>,
    workflows: Arc<RwLock<HashMap<String, WorkflowFn<serde_json::Value>>>>,
    registry: Arc<WorkflowRegistry>,
    config: WorkerConfig,
    worker_id: String,
    worker_db_id: Arc<RwLock<Option<Uuid>>>,
    running: Arc<RwLock<bool>>,
    semaphore: Arc<Semaphore>,
    active_workflow_count: Arc<RwLock<usize>>,
}

impl Worker {
    /// Create a new worker with default configuration
    pub fn new(
        db: Arc<Database>,
        workflows: Arc<RwLock<HashMap<String, WorkflowFn<serde_json::Value>>>>,
        registry: Arc<WorkflowRegistry>,
    ) -> Self {
        Self::with_config(db, workflows, registry, WorkerConfig::default())
    }

    /// Create a new worker with custom configuration
    pub fn with_config(
        db: Arc<Database>,
        workflows: Arc<RwLock<HashMap<String, WorkflowFn<serde_json::Value>>>>,
        registry: Arc<WorkflowRegistry>,
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
            registry,
            semaphore: Arc::new(Semaphore::new(config.max_concurrent_workflows)),
            config,
            worker_id,
            worker_db_id: Arc::new(RwLock::new(None)),
            running: Arc::new(RwLock::new(false)),
            active_workflow_count: Arc::new(RwLock::new(0)),
        }
    }

    /// Create a builder for configuring the worker
    pub fn builder(
        db: Arc<Database>,
        workflows: Arc<RwLock<HashMap<String, WorkflowFn<serde_json::Value>>>>,
        registry: Arc<WorkflowRegistry>,
    ) -> WorkerBuilder {
        WorkerBuilder::new(db, workflows, registry)
    }

    /// Start the worker (blocking)
    ///
    /// This will run indefinitely, polling for workflows and executing them.
    /// The worker will gracefully shutdown on SIGTERM or SIGINT.
    pub async fn start(&self) -> Result<()> {
        {
            let mut running = self.running.write().await;
            if *running {
                return Err(anyhow::anyhow!("Worker is already running"));
            }
            *running = true;
        }

        info!("Worker {} starting...", self.worker_id);

        // Register worker in database
        let worker_metadata = WorkerMetadata::current();
        let hostname = self.config.hostname.clone().unwrap_or_else(|| {
            worker_metadata.hostname.clone()
        });

        let worker = queries::register_worker(
            self.db.pool(),
            &self.worker_id,
            Some(hostname),
            Some(worker_metadata.process_id as i32),
            Some(serde_json::to_value(&self.config)?),
            Some(serde_json::to_value(&worker_metadata)?),
        )
        .await?;

        let db_id = worker.id;

        {
            let mut worker_db_id = self.worker_db_id.write().await;
            *worker_db_id = Some(db_id);
        }

        // Record STARTED event
        queries::record_worker_event(
            self.db.pool(),
            db_id,
            WorkerEventType::Started,
            None,
        )
        .await?;

        info!("Worker {} registered with ID: {}", self.worker_id, db_id);

        // Spawn heartbeat task
        let heartbeat_handle = self.spawn_heartbeat_task();

        // Spawn shutdown signal handler
        let shutdown_handle = self.spawn_shutdown_handler();

        // Main worker loop
        let result = self.run_worker_loop().await;

        // Wait for tasks to complete
        heartbeat_handle.abort();
        shutdown_handle.abort();

        // Update worker status to STOPPED
        queries::update_worker_status(
            self.db.pool(),
            db_id,
            WorkerStatus::Stopped,
        )
        .await?;

        // Record SHUTDOWN_COMPLETE event
        queries::record_worker_event(
            self.db.pool(),
            db_id,
            WorkerEventType::ShutdownComplete,
            None,
        )
        .await?;

        info!("Worker {} stopped", self.worker_id);

        result
    }

    /// Spawn a background task to handle shutdown signals
    fn spawn_shutdown_handler(&self) -> tokio::task::JoinHandle<()> {
        let running = self.running.clone();
        let worker_id = self.worker_id.clone();
        let db = self.db.clone();
        let worker_db_id = self.worker_db_id.clone();

        tokio::spawn(async move {
            // Wait for SIGTERM or SIGINT
            tokio::select! {
                _ = signal::ctrl_c() => {
                    info!("Worker {} received SIGINT, initiating graceful shutdown...", worker_id);
                }
                _ = Self::wait_for_sigterm() => {
                    info!("Worker {} received SIGTERM, initiating graceful shutdown...", worker_id);
                }
            }

            // Set running to false to stop polling
            {
                let mut running = running.write().await;
                *running = false;
            }

            // Update worker status to SHUTTING_DOWN
            if let Some(db_id) = *worker_db_id.read().await {
                if let Err(e) = queries::update_worker_status(
                    db.pool(),
                    db_id,
                    WorkerStatus::ShuttingDown,
                )
                .await
                {
                    error!("Failed to update worker status to SHUTTING_DOWN: {}", e);
                }

                // Record SHUTDOWN_REQUESTED event
                if let Err(e) = queries::record_worker_event(
                    db.pool(),
                    db_id,
                    WorkerEventType::ShutdownRequested,
                    None,
                )
                .await
                {
                    error!("Failed to record SHUTDOWN_REQUESTED event: {}", e);
                }
            }
        })
    }

    /// Wait for SIGTERM signal
    #[cfg(unix)]
    async fn wait_for_sigterm() {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigterm = signal(SignalKind::terminate()).unwrap();
        sigterm.recv().await;
    }

    /// Wait for SIGTERM signal (Windows fallback)
    #[cfg(not(unix))]
    async fn wait_for_sigterm() {
        // Windows doesn't have SIGTERM, so this will never complete
        // The worker will only respond to Ctrl+C (SIGINT)
        std::future::pending::<()>().await
    }

    /// Spawn a background task to send heartbeats
    fn spawn_heartbeat_task(&self) -> tokio::task::JoinHandle<()> {
        let db = self.db.clone();
        let worker_db_id = self.worker_db_id.clone();
        let running = self.running.clone();
        let heartbeat_interval_secs = self.config.heartbeat_interval_secs;
        let worker_id = self.worker_id.clone();

        tokio::spawn(async move {
            let mut interval_timer = interval(Duration::from_secs(heartbeat_interval_secs));

            loop {
                interval_timer.tick().await;

                // Check if we should stop
                {
                    let running = running.read().await;
                    if !*running {
                        debug!("Heartbeat task stopping for worker {}", worker_id);
                        break;
                    }
                }

                // Send heartbeat
                if let Some(db_id) = *worker_db_id.read().await {
                    match queries::update_worker_heartbeat(db.pool(), db_id).await {
                        Ok(_) => {
                            debug!("Worker {} sent heartbeat", worker_id);
                        }
                        Err(e) => {
                            error!("Failed to send heartbeat for worker {}: {}", worker_id, e);
                        }
                    }
                } else {
                    warn!("Worker {} not registered, skipping heartbeat", worker_id);
                }
            }
        })
    }

    /// Main worker loop
    async fn run_worker_loop(&self) -> Result<()> {
        loop {
            // Check if we should stop
            {
                let running = self.running.read().await;
                if !*running {
                    info!("Worker {} received shutdown signal, waiting for active workflows to complete...", self.worker_id);

                    // Wait for active workflows to complete (with timeout)
                    let shutdown_timeout = Duration::from_secs(self.config.graceful_shutdown_timeout_secs);
                    let start = std::time::Instant::now();

                    loop {
                        let active_count = *self.active_workflow_count.read().await;
                        if active_count == 0 {
                            info!("All workflows completed, worker {} shutting down cleanly", self.worker_id);
                            break;
                        }

                        if start.elapsed() > shutdown_timeout {
                            warn!(
                                "Shutdown timeout reached, {} workflows still active, forcing shutdown",
                                active_count
                            );
                            break;
                        }

                        debug!("Waiting for {} active workflows to complete...", active_count);
                        tokio::time::sleep(Duration::from_millis(500)).await;
                    }

                    break;
                }
            }

            // Try to acquire semaphore permit (concurrent workflow limiting)
            let permit = match self.semaphore.clone().try_acquire_owned() {
                Ok(permit) => permit,
                Err(_) => {
                    // At max capacity, sleep and try again
                    debug!(
                        "Worker {} at max capacity ({} concurrent workflows), waiting...",
                        self.worker_id, self.config.max_concurrent_workflows
                    );
                    tokio::time::sleep(Duration::from_millis(self.config.poll_interval_ms)).await;
                    continue;
                }
            };

            // Poll for next workflow
            match self.poll_and_execute_with_permit(permit).await {
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

        Ok(())
    }

    /// Stop the worker
    pub async fn stop(&self) {
        let mut running = self.running.write().await;
        *running = false;
        info!("Worker {} stop requested", self.worker_id);
    }

    /// Poll for a workflow and execute it (with semaphore permit)
    async fn poll_and_execute_with_permit(
        &self,
        permit: tokio::sync::OwnedSemaphorePermit,
    ) -> Result<bool> {
        // Poll for next workflow
        let workflow_run = match queries::poll_next_workflow(self.db.pool()).await? {
            Some(run) => run,
            None => {
                // No workflows available, release permit
                drop(permit);
                return Ok(false);
            }
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
                // Update worker lease name
                if let Err(e) = queries::update_worker_lease_name(
                    self.db.pool(),
                    workflow_run.id,
                    &self.worker_id,
                )
                .await
                {
                    warn!("Failed to update worker lease name: {}", e);
                }

                // Increment active workflow count
                {
                    let mut count = self.active_workflow_count.write().await;
                    *count += 1;
                }

                // Record WORKFLOW_STARTED event
                if let Some(db_id) = *self.worker_db_id.read().await {
                    let _ = queries::record_worker_event(
                        self.db.pool(),
                        db_id,
                        WorkerEventType::WorkflowStarted,
                        Some(serde_json::json!({
                            "workflow_run_id": workflow_run.id,
                            "workflow_name": workflow_run.workflow_name,
                        })),
                    )
                    .await;
                }

                // Execute workflow in background task
                let self_clone = self.clone_for_execution();
                let workflow_run_id = workflow_run.id;

                tokio::spawn(async move {
                    let result = self_clone.execute_workflow(workflow_run_id).await;

                    // Decrement active workflow count
                    {
                        let mut count = self_clone.active_workflow_count.write().await;
                        *count = count.saturating_sub(1);
                    }

                    // Record workflow completion event
                    if let Some(db_id) = *self_clone.worker_db_id.read().await {
                        let event_type = if result.is_ok() {
                            WorkerEventType::WorkflowCompleted
                        } else {
                            WorkerEventType::WorkflowFailed
                        };

                        let _ = queries::record_worker_event(
                            self_clone.db.pool(),
                            db_id,
                            event_type,
                            Some(serde_json::json!({
                                "workflow_run_id": workflow_run_id,
                            })),
                        )
                        .await;
                    }

                    // Release permit when done
                    drop(permit);

                    result
                });

                Ok(true)
            }
            Err(e) => {
                // Failed to acquire lease (another worker got it)
                debug!(
                    "Failed to acquire lease for workflow {}: {}",
                    workflow_run.id, e
                );
                drop(permit);
                Ok(false)
            }
        }
    }

    /// Clone the necessary fields for executing a workflow in a background task
    fn clone_for_execution(&self) -> Self {
        Self {
            db: self.db.clone(),
            workflows: self.workflows.clone(),
            registry: self.registry.clone(),
            config: self.config.clone(),
            worker_id: self.worker_id.clone(),
            worker_db_id: self.worker_db_id.clone(),
            running: self.running.clone(),
            semaphore: self.semaphore.clone(),
            active_workflow_count: self.active_workflow_count.clone(),
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

        // Look up workflow function from version registry
        let workflow_fn = self.registry
            .get_workflow(&workflow_run.workflow_name, workflow_run.workflow_version)
            .await
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Workflow function not registered: {} version {}",
                    workflow_run.workflow_name,
                    workflow_run.workflow_version
                )
            })?;

        let workflow_fn = workflow_fn.clone();

        // Create workflow context
        let ctx = WorkflowContext::new(
            workflow_run_id,
            workflow_run.workflow_name.clone(),
            self.db.clone(),
        );

        // Execute workflow
        let result = workflow_fn.call(ctx, workflow_run.input.clone()).await;

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

                    // Convert to structured error
                    let step_error = StepError::new(ErrorKind::Unknown, error_msg)
                        .with_source(format!("{:?}", e));
                    let error_json = serde_json::to_value(&step_error)?;

                    queries::update_workflow_status(
                        self.db.pool(),
                        workflow_run_id,
                        WorkflowStatus::Failed,
                        Some(error_json),
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

    /// Get number of active workflows
    pub async fn active_workflow_count(&self) -> usize {
        *self.active_workflow_count.read().await
    }

    /// Check if worker is running
    pub async fn is_running(&self) -> bool {
        *self.running.read().await
    }
}
