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

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn test_error_display() {
        let id = Uuid::new_v4();
        let err = Error::WorkflowNotFound(id);
        assert_eq!(err.to_string(), format!("Workflow not found: {}", id));

        let err = Error::StepNotFound(id, "step-1".to_string());
        assert_eq!(
            err.to_string(),
            format!("Step not found: workflow_run_id={}, step_id=step-1", id)
        );

        let err = Error::WorkflowAlreadyExists(id);
        assert_eq!(err.to_string(), format!("Workflow already exists: {}", id));

        let err = Error::InvalidStatusTransition {
            from: "PENDING".to_string(),
            to: "COMPLETED".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "Invalid workflow status transition: from PENDING to COMPLETED"
        );

        let err = Error::LeaseNotAcquired;
        assert_eq!(err.to_string(), "Worker lease not acquired");

        let err = Error::Migration("test migration error".to_string());
        assert_eq!(err.to_string(), "Migration error: test migration error");

        let err = Error::Other("test error".to_string());
        assert_eq!(err.to_string(), "test error");
    }

    #[test]
    fn test_error_from_serde() {
        let json_err = serde_json::from_str::<serde_json::Value>("invalid json").unwrap_err();
        let err: Error = json_err.into();
        assert!(matches!(err, Error::Serialization(_)));
    }
}
