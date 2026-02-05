use std::future::Future;
use std::pin::Pin;

mod client;
mod context;
mod errors;
mod propagation;
mod retry;
mod worker;

pub use client::{Kagzi, ScheduleBuilder, StartWorkflowBuilder, WorkflowRun};
pub use client::{QueueBuilder, TaskQueueHandle};
pub use context::{Context, StepBuilder};
pub use errors::{KagziError, WorkflowPaused};
pub use retry::Retry;
pub use worker::{SignalBackend, Worker, WorkerBuilder};

/// A prelude module for convenient imports
pub mod prelude {
    pub use crate::{
        Context, Kagzi, QueueBuilder, Retry, SignalBackend, TaskQueueHandle, Worker, WorkerBuilder,
    };
}

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
