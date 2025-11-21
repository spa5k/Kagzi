//! Workflow execution context
//!
//! Provides the runtime context for workflow execution, including step memoization
//! and durable sleep functionality.

use anyhow::Result;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::sync::Arc;
use tracing::{debug, info};
use uuid::Uuid;

use kagzi_core::{queries, CreateStepRun, Database, StepStatus};

/// Workflow execution context
///
/// This is passed to every workflow function and provides methods for:
/// - Step execution with automatic memoization
/// - Durable sleep
/// - Accessing workflow metadata
#[derive(Clone)]
pub struct WorkflowContext {
    pub(crate) workflow_run_id: Uuid,
    pub(crate) workflow_name: String,
    pub(crate) db: Arc<Database>,
}

impl WorkflowContext {
    /// Create a new workflow context
    pub fn new(workflow_run_id: Uuid, workflow_name: String, db: Arc<Database>) -> Self {
        Self {
            workflow_run_id,
            workflow_name,
            db,
        }
    }

    /// Get the workflow run ID
    pub fn workflow_run_id(&self) -> Uuid {
        self.workflow_run_id
    }

    /// Get the workflow name
    pub fn workflow_name(&self) -> &str {
        &self.workflow_name
    }

    /// Execute a step with memoization
    ///
    /// If the step has been executed before (found in database), returns the cached result.
    /// Otherwise, executes the provided future and stores the result.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use kagzi::WorkflowContext;
    /// # async fn example(ctx: WorkflowContext) -> anyhow::Result<()> {
    /// let user_data: String = ctx.step("fetch-user", async {
    ///     // This only runs once - subsequent calls return cached result
    ///     Ok::<_, anyhow::Error>("user data".to_string())
    /// }).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn step<Fut, T>(&self, step_id: &str, f: Fut) -> Result<T>
    where
        Fut: Future<Output = Result<T>>,
        T: Serialize + for<'de> Deserialize<'de>,
    {
        // Check if step already exists (memoization)
        if let Some(step_run) =
            queries::get_step_run(self.db.pool(), self.workflow_run_id, step_id).await?
        {
            debug!(
                "Step '{}' already executed for workflow {}, returning cached result",
                step_id, self.workflow_run_id
            );

            // Return cached result
            if let Some(output) = step_run.output {
                let result: T = serde_json::from_value(output)?;
                return Ok(result);
            } else if let Some(error) = step_run.error {
                return Err(anyhow::anyhow!("Step previously failed: {}", error));
            }
        }

        info!(
            "Executing step '{}' for workflow {}",
            step_id, self.workflow_run_id
        );

        // Execute the step
        let result = f.await;

        // Store the result
        match result {
            Ok(ref value) => {
                let output = serde_json::to_value(value)?;
                queries::create_step_run(
                    self.db.pool(),
                    CreateStepRun {
                        workflow_run_id: self.workflow_run_id,
                        step_id: step_id.to_string(),
                        input_hash: None,
                        output: Some(output),
                        error: None,
                        status: StepStatus::Completed,
                    },
                )
                .await?;

                info!("Step '{}' completed successfully", step_id);
            }
            Err(ref e) => {
                queries::create_step_run(
                    self.db.pool(),
                    CreateStepRun {
                        workflow_run_id: self.workflow_run_id,
                        step_id: step_id.to_string(),
                        input_hash: None,
                        output: None,
                        error: Some(e.to_string()),
                        status: StepStatus::Failed,
                    },
                )
                .await?;

                info!("Step '{}' failed: {}", step_id, e);
            }
        }

        result
    }

    /// Sleep for a duration (durable - survives restarts)
    ///
    /// This puts the workflow to sleep without holding a worker slot.
    /// The workflow will be resumed after the specified duration.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use kagzi::WorkflowContext;
    /// # use std::time::Duration;
    /// # async fn example(ctx: WorkflowContext) -> anyhow::Result<()> {
    /// // Sleep for 1 hour
    /// ctx.sleep("wait-1h", Duration::from_secs(3600)).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn sleep(&self, step_id: &str, duration: std::time::Duration) -> Result<()> {
        // Check if this sleep has already completed (memoization)
        if let Some(step_run) =
            queries::get_step_run(self.db.pool(), self.workflow_run_id, step_id).await?
        {
            if matches!(step_run.status, StepStatus::Completed) {
                debug!(
                    "Sleep step '{}' already completed for workflow {}, skipping",
                    step_id, self.workflow_run_id
                );
                return Ok(());
            }
        }

        let sleep_until = Utc::now()
            + ChronoDuration::from_std(duration)
                .map_err(|e| anyhow::anyhow!("Invalid duration: {}", e))?;

        info!(
            "Workflow {} sleeping until {} (step: {})",
            self.workflow_run_id, sleep_until, step_id
        );

        // Set workflow to sleeping status
        queries::set_workflow_sleep(self.db.pool(), self.workflow_run_id, sleep_until).await?;

        // Mark this sleep step as completed so we don't repeat it on resume
        queries::create_step_run(
            self.db.pool(),
            CreateStepRun {
                workflow_run_id: self.workflow_run_id,
                step_id: step_id.to_string(),
                input_hash: None,
                output: Some(serde_json::json!({"slept_until": sleep_until})),
                error: None,
                status: StepStatus::Completed,
            },
        )
        .await?;

        // Signal that we need to stop execution and let worker pick up later
        Err(anyhow::anyhow!("__SLEEP__"))
    }

    /// Sleep until a specific time
    pub async fn sleep_until(&self, step_id: &str, wake_time: DateTime<Utc>) -> Result<()> {
        // Check if this sleep has already completed
        if let Some(step_run) =
            queries::get_step_run(self.db.pool(), self.workflow_run_id, step_id).await?
        {
            if matches!(step_run.status, StepStatus::Completed) {
                debug!(
                    "Sleep step '{}' already completed for workflow {}, skipping",
                    step_id, self.workflow_run_id
                );
                return Ok(());
            }
        }

        info!(
            "Workflow {} sleeping until {} (step: {})",
            self.workflow_run_id, wake_time, step_id
        );

        queries::set_workflow_sleep(self.db.pool(), self.workflow_run_id, wake_time).await?;

        queries::create_step_run(
            self.db.pool(),
            CreateStepRun {
                workflow_run_id: self.workflow_run_id,
                step_id: step_id.to_string(),
                input_hash: None,
                output: Some(serde_json::json!({"slept_until": wake_time})),
                error: None,
                status: StepStatus::Completed,
            },
        )
        .await?;

        Err(anyhow::anyhow!("__SLEEP__"))
    }
}
