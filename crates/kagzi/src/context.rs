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

use kagzi_core::{queries, CreateStepRun, Database, ErrorKind, StepError, StepStatus};

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
            } else if let Some(error_value) = step_run.error {
                // Try to deserialize as StepError, fallback to string message
                if let Ok(step_error) = serde_json::from_value::<StepError>(error_value.clone()) {
                    return Err(anyhow::anyhow!(
                        "Step previously failed: {} - {}",
                        step_error.kind,
                        step_error.message
                    ));
                } else if let Some(error_str) = error_value.as_str() {
                    // Legacy string error
                    return Err(anyhow::anyhow!("Step previously failed: {}", error_str));
                } else {
                    return Err(anyhow::anyhow!(
                        "Step previously failed with unknown error format"
                    ));
                }
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
                        parent_step_id: None,
                        parallel_group_id: None,
                    },
                )
                .await?;

                info!("Step '{}' completed successfully", step_id);
            }
            Err(ref e) => {
                // Convert anyhow::Error to StepError
                let step_error = StepError::new(ErrorKind::Unknown, e.to_string())
                    .with_source(format!("{:?}", e));

                // Serialize StepError to JSONB
                let error_json = serde_json::to_value(&step_error)?;

                queries::create_step_run(
                    self.db.pool(),
                    CreateStepRun {
                        workflow_run_id: self.workflow_run_id,
                        step_id: step_id.to_string(),
                        input_hash: None,
                        output: None,
                        error: Some(error_json),
                        status: StepStatus::Failed,
                        parent_step_id: None,
                        parallel_group_id: None,
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
                parent_step_id: None,
                parallel_group_id: None,
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
                parent_step_id: None,
                parallel_group_id: None,
            },
        )
        .await?;

        Err(anyhow::anyhow!("__SLEEP__"))
    }

    /// Create a step builder for advanced step configuration (retry policies, etc.)
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use kagzi::{WorkflowContext, RetryPolicy};
    /// # async fn example(ctx: WorkflowContext) -> anyhow::Result<()> {
    /// let result: String = ctx.step_builder("fetch-user")
    ///     .retry_policy(RetryPolicy::exponential())
    ///     .execute(async {
    ///         // This will retry automatically on transient failures
    ///         Ok::<_, anyhow::Error>("user data".to_string())
    ///     })
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn step_builder<'a>(&'a self, step_id: &'a str) -> StepBuilder<'a> {
        StepBuilder {
            ctx: self,
            step_id,
            retry_policy: None,
        }
    }

    /// Execute a dynamic list of steps in parallel
    ///
    /// All steps execute concurrently with full memoization support.
    /// Returns a vector of results in the same order as the input steps.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use kagzi::WorkflowContext;
    /// # async fn example(ctx: WorkflowContext) -> anyhow::Result<()> {
    /// // Execute multiple API calls in parallel
    /// let user_ids = vec!["user1", "user2", "user3"];
    /// // Note: Full implementation pending
    /// # Ok(())
    /// # }
    /// # ```
    pub async fn parallel_vec<T>(
        &self,
        _group_name: &str,
        _steps: Vec<(String, std::pin::Pin<Box<dyn Future<Output = Result<T>> + Send>>)>,
    ) -> Result<Vec<T>>
    where
        T: Serialize + for<'de> Deserialize<'de> + Send + 'static,
    {
        // TODO: Implement parallel execution logic
        Err(anyhow::anyhow!("Parallel execution not yet fully implemented"))
    }
}

/// Builder for configuring and executing workflow steps with advanced options
pub struct StepBuilder<'a> {
    ctx: &'a WorkflowContext,
    step_id: &'a str,
    retry_policy: Option<kagzi_core::RetryPolicy>,
}

impl<'a> StepBuilder<'a> {
    /// Set the retry policy for this step
    pub fn retry_policy(mut self, policy: kagzi_core::RetryPolicy) -> Self {
        self.retry_policy = Some(policy);
        self
    }

    /// Execute the step with the configured options
    pub async fn execute<Fut, T>(self, f: Fut) -> Result<T>
    where
        Fut: Future<Output = Result<T>>,
        T: Serialize + for<'de> Deserialize<'de>,
    {
        // Check if step already exists (memoization)
        if let Some(step_run) = queries::get_step_run(
            self.ctx.db.pool(),
            self.ctx.workflow_run_id,
            self.step_id,
        )
        .await?
        {
            // Check if this step is waiting for retry
            if step_run.should_retry() {
                let now = Utc::now();

                if !step_run.is_retry_ready(now) {
                    // Not ready to retry yet - signal worker to come back later
                    debug!(
                        "Step '{}' scheduled for retry at {:?}, workflow will sleep",
                        self.step_id, step_run.next_retry_at
                    );

                    if let Some(next_retry_at) = step_run.next_retry_at {
                        queries::set_workflow_sleep(
                            self.ctx.db.pool(),
                            self.ctx.workflow_run_id,
                            next_retry_at,
                        )
                        .await?;
                    }

                    return Err(anyhow::anyhow!("__RETRY__"));
                }

                // Ready to retry - fall through to execution
                info!(
                    "Retrying step '{}' (attempt {}/{})",
                    self.step_id,
                    step_run.attempts + 1,
                    step_run.get_retry_policy()
                        .map(|p| p.max_attempts())
                        .unwrap_or(0)
                );
            } else if matches!(step_run.status, StepStatus::Completed) {
                // Step completed successfully - return cached result
                debug!(
                    "Step '{}' already executed for workflow {}, returning cached result",
                    self.step_id, self.ctx.workflow_run_id
                );

                if let Some(output) = step_run.output {
                    let result: T = serde_json::from_value(output)?;
                    return Ok(result);
                }
            } else if matches!(step_run.status, StepStatus::Failed) && !step_run.should_retry() {
                // Step failed and won't retry - return error
                if let Some(error_value) = step_run.error {
                    if let Ok(step_error) = serde_json::from_value::<StepError>(error_value.clone())
                    {
                        return Err(anyhow::anyhow!(
                            "Step permanently failed: {} - {}",
                            step_error.kind,
                            step_error.message
                        ));
                    }
                }
                return Err(anyhow::anyhow!("Step permanently failed"));
            }
        }

        info!(
            "Executing step '{}' for workflow {}",
            self.step_id, self.ctx.workflow_run_id
        );

        // Execute the step
        let result = f.await;

        // Handle the result
        match result {
            Ok(ref value) => {
                // Success - store result and clear any retry schedule
                let output = serde_json::to_value(value)?;

                // Clear retry schedule if this was a retry
                let _ = queries::clear_step_retry_schedule(
                    self.ctx.db.pool(),
                    self.ctx.workflow_run_id,
                    self.step_id,
                )
                .await;

                queries::create_step_run(
                    self.ctx.db.pool(),
                    CreateStepRun {
                        workflow_run_id: self.ctx.workflow_run_id,
                        step_id: self.step_id.to_string(),
                        input_hash: None,
                        output: Some(output),
                        error: None,
                        status: StepStatus::Completed,
                        parent_step_id: None,
                        parallel_group_id: None,
                    },
                )
                .await?;

                info!("Step '{}' completed successfully", self.step_id);
            }
            Err(ref e) => {
                // Failure - determine if we should retry
                let step_error = StepError::new(ErrorKind::Unknown, e.to_string())
                    .with_source(format!("{:?}", e));

                // Get current attempt count
                let current_attempts = if let Some(step_run) = queries::get_step_run(
                    self.ctx.db.pool(),
                    self.ctx.workflow_run_id,
                    self.step_id,
                )
                .await?
                {
                    step_run.attempts
                } else {
                    0
                };

                let error_json = serde_json::to_value(&step_error)?;

                // Check if we should retry
                if let Some(ref policy) = self.retry_policy {
                    if step_error.retryable && policy.allows_retry() {
                        let next_attempt = current_attempts + 1;

                        if let Some(next_retry_at) = policy.next_retry_at(next_attempt as u32, Utc::now()) {
                            // Schedule retry
                            info!(
                                "Step '{}' failed (attempt {}), scheduling retry at {:?}",
                                self.step_id, next_attempt, next_retry_at
                            );

                            let policy_json = serde_json::to_value(policy)?;

                            queries::update_step_retry_state(
                                self.ctx.db.pool(),
                                self.ctx.workflow_run_id,
                                self.step_id,
                                next_attempt,
                                Some(next_retry_at),
                                Some(policy_json),
                                Some(error_json.clone()),
                            )
                            .await?;

                            // Set workflow to sleep until retry time
                            queries::set_workflow_sleep(
                                self.ctx.db.pool(),
                                self.ctx.workflow_run_id,
                                next_retry_at,
                            )
                            .await?;

                            return Err(anyhow::anyhow!("__RETRY__"));
                        }
                    }
                }

                // No retry - store as permanent failure
                info!("Step '{}' failed permanently: {}", self.step_id, e);

                queries::create_step_run(
                    self.ctx.db.pool(),
                    CreateStepRun {
                        workflow_run_id: self.ctx.workflow_run_id,
                        step_id: self.step_id.to_string(),
                        input_hash: None,
                        output: None,
                        error: Some(error_json),
                        status: StepStatus::Failed,
                        parent_step_id: None,
                        parallel_group_id: None,
                    },
                )
                .await?;
            }
        }

        result
    }
}
