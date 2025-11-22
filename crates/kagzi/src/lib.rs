//! # Kagzi - Durable Workflow Engine
//!
//! Kagzi is a Rust-based durable workflow engine that allows you to build
//! resilient, long-running workflows that can survive crashes and restarts.
//!
//! ## Example
//!
//! ```no_run
//! use kagzi::{Kagzi, WorkflowContext};
//! use serde::{Deserialize, Serialize};
//!
//! #[derive(Serialize, Deserialize)]
//! struct MyInput {
//!     user_id: String,
//! }
//!
//! async fn my_workflow(ctx: WorkflowContext, input: MyInput) -> anyhow::Result<String> {
//!     // Steps are memoized - if workflow restarts, completed steps are skipped
//!     let result = ctx.step("fetch-data", async {
//!         Ok::<_, anyhow::Error>("data".to_string())
//!     }).await?;
//!
//!     Ok(result)
//! }
//! ```

pub mod client;
pub mod context;
pub mod worker;

// Re-exports
pub use client::Kagzi;
pub use context::{StepBuilder, WorkflowContext};
pub use worker::Worker;

// Re-export common types from kagzi-core
pub use kagzi_core::{RetryPolicy, RetryPredicate};
pub use uuid::Uuid;
