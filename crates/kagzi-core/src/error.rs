//! Error types for Kagzi Core

use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Workflow not found: {0}")]
    WorkflowNotFound(uuid::Uuid),

    #[error("Step not found: workflow_run_id={0}, step_id={1}")]
    StepNotFound(uuid::Uuid, String),

    #[error("Workflow already exists: {0}")]
    WorkflowAlreadyExists(uuid::Uuid),

    #[error("Invalid workflow status transition: from {from} to {to}")]
    InvalidStatusTransition { from: String, to: String },

    #[error("Worker lease not acquired")]
    LeaseNotAcquired,

    #[error("Migration error: {0}")]
    Migration(String),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;
