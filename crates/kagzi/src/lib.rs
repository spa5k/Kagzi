use std::future::Future;
use std::pin::Pin;

pub mod tracing_utils;

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

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
