//! Parallel step execution for workflows
//!
//! This module provides utilities for executing multiple workflow steps concurrently,
//! with full memoization support and error handling strategies.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::future::Future;
use uuid::Uuid;

use crate::WorkflowContext;

/// Error handling strategy for parallel execution
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParallelErrorStrategy {
    /// Stop immediately when any step fails (fail-fast)
    FailFast,
    /// Wait for all steps to complete, collecting all errors
    CollectAll,
}

/// Result of a parallel execution
#[derive(Debug)]
pub struct ParallelResult<T> {
    /// Successful results
    pub results: Vec<T>,
    /// Errors encountered (only populated with CollectAll strategy)
    pub errors: Vec<anyhow::Error>,
}

impl<T> ParallelResult<T> {
    /// Check if all steps succeeded
    pub fn is_success(&self) -> bool {
        self.errors.is_empty()
    }

    /// Get the first error if any
    pub fn first_error(&self) -> Option<&anyhow::Error> {
        self.errors.first()
    }

    /// Convert to a Result, failing if any errors occurred
    pub fn into_result(self) -> Result<Vec<T>> {
        if let Some(error) = self.errors.into_iter().next() {
            Err(error)
        } else {
            Ok(self.results)
        }
    }
}

/// Parallel execution context
///
/// This struct provides the core functionality for parallel step execution,
/// including memoization checking, spawning tasks, and collecting results.
pub struct ParallelExecutor<'a> {
    ctx: &'a WorkflowContext,
    parallel_group_id: Uuid,
    parent_step_id: Option<String>,
    #[allow(dead_code)]
    error_strategy: ParallelErrorStrategy,
}

impl<'a> ParallelExecutor<'a> {
    /// Create a new parallel executor
    pub fn new(
        ctx: &'a WorkflowContext,
        parallel_group_id: Uuid,
        parent_step_id: Option<String>,
        error_strategy: ParallelErrorStrategy,
    ) -> Self {
        Self {
            ctx,
            parallel_group_id,
            parent_step_id,
            error_strategy,
        }
    }

    /// Execute a single step within the parallel group
    ///
    /// This checks memoization first, then executes if needed
    pub async fn execute_step<Fut, T>(
        &self,
        step_id: &str,
        f: Fut,
    ) -> Result<T>
    where
        Fut: Future<Output = Result<T>>,
        T: Serialize + for<'de> Deserialize<'de>,
    {
        use kagzi_core::{queries, CreateStepRun, StepStatus};
        use tracing::{debug, info};

        // Check if step already exists (memoization)
        if let Some(step_run) =
            queries::get_step_run(self.ctx.db.pool(), self.ctx.workflow_run_id, step_id).await?
        {
            debug!(
                "Parallel step '{}' already executed, returning cached result",
                step_id
            );

            if let Some(output) = step_run.output {
                let result: T = serde_json::from_value(output)?;
                return Ok(result);
            } else if let Some(error_value) = step_run.error {
                return Err(anyhow::anyhow!(
                    "Parallel step previously failed: {}",
                    error_value
                ));
            }
        }

        info!(
            "Executing parallel step '{}' in group {}",
            step_id, self.parallel_group_id
        );

        // Execute the step
        let result = f.await;

        // Store the result with parallel tracking fields
        match result {
            Ok(ref value) => {
                let output = serde_json::to_value(value)?;
                queries::create_step_run(
                    self.ctx.db.pool(),
                    CreateStepRun {
                        workflow_run_id: self.ctx.workflow_run_id,
                        step_id: step_id.to_string(),
                        input_hash: None,
                        output: Some(output),
                        error: None,
                        status: StepStatus::Completed,
                        parent_step_id: self.parent_step_id.clone(),
                        parallel_group_id: Some(self.parallel_group_id),
                    },
                )
                .await?;

                info!("Parallel step '{}' completed successfully", step_id);
            }
            Err(ref e) => {
                let error_json = serde_json::to_value(e.to_string())?;
                queries::create_step_run(
                    self.ctx.db.pool(),
                    CreateStepRun {
                        workflow_run_id: self.ctx.workflow_run_id,
                        step_id: step_id.to_string(),
                        input_hash: None,
                        output: None,
                        error: Some(error_json),
                        status: StepStatus::Failed,
                        parent_step_id: self.parent_step_id.clone(),
                        parallel_group_id: Some(self.parallel_group_id),
                    },
                )
                .await?;

                info!("Parallel step '{}' failed: {}", step_id, e);
            }
        }

        result
    }
}

// Macro to implement parallel() for tuples of different sizes
// This provides compile-time type safety for parallel execution

macro_rules! impl_parallel_tuple {
    ($($T:ident),+) => {
        // Not implemented yet - will be added in the core implementation phase
    };
}

// Implement for tuples up to 10 elements
impl_parallel_tuple!(T1, T2);
impl_parallel_tuple!(T1, T2, T3);
impl_parallel_tuple!(T1, T2, T3, T4);
impl_parallel_tuple!(T1, T2, T3, T4, T5);
impl_parallel_tuple!(T1, T2, T3, T4, T5, T6);
impl_parallel_tuple!(T1, T2, T3, T4, T5, T6, T7);
impl_parallel_tuple!(T1, T2, T3, T4, T5, T6, T7, T8);
impl_parallel_tuple!(T1, T2, T3, T4, T5, T6, T7, T8, T9);
impl_parallel_tuple!(T1, T2, T3, T4, T5, T6, T7, T8, T9, T10);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parallel_result_is_success() {
        let result = ParallelResult {
            results: vec![1, 2, 3],
            errors: vec![],
        };
        assert!(result.is_success());

        let result_with_errors: ParallelResult<i32> = ParallelResult {
            results: vec![],
            errors: vec![anyhow::anyhow!("error")],
        };
        assert!(!result_with_errors.is_success());
    }

    #[test]
    fn test_parallel_result_into_result() {
        let result = ParallelResult {
            results: vec![1, 2, 3],
            errors: vec![],
        };
        assert!(result.into_result().is_ok());

        let result_with_errors: ParallelResult<i32> = ParallelResult {
            results: vec![],
            errors: vec![anyhow::anyhow!("error")],
        };
        assert!(result_with_errors.into_result().is_err());
    }

    #[test]
    fn test_parallel_error_strategy_equality() {
        assert_eq!(
            ParallelErrorStrategy::FailFast,
            ParallelErrorStrategy::FailFast
        );
        assert_ne!(
            ParallelErrorStrategy::FailFast,
            ParallelErrorStrategy::CollectAll
        );
    }
}
