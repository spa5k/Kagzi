use std::future::Future;
use std::pin::Pin;

mod client;
mod context;
mod errors;
mod retry;
mod worker;

pub use client::{Kagzi, ScheduleBuilder, StartWorkflowBuilder, WorkflowRun};
pub use context::{Context, StepBuilder};
pub use errors::{KagziError, WorkflowPaused};
pub use retry::Retry;
pub use worker::{Worker, WorkerBuilder};

/// A prelude module for convenient imports
pub mod prelude {
    pub use crate::{Context, Kagzi, Retry, Worker, WorkerBuilder};
}

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
