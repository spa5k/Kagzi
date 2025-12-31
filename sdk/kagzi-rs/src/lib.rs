use std::future::Future;
use std::pin::Pin;

mod client;
mod errors;
mod retry;
mod worker;
mod workflow_context;

pub use client::{Client, WorkflowBuilder, WorkflowScheduleBuilder};
pub use errors::{KagziError, WorkflowPaused};
pub use retry::RetryPolicy;
pub use worker::{Worker, WorkerBuilder};
pub use workflow_context::WorkflowContext;

/// A prelude module for convenient imports
pub mod prelude {
    pub use crate::{Client, KagziError, RetryPolicy, Worker, WorkerBuilder, WorkflowContext};
}

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
