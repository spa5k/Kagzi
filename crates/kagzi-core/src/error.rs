//! Error types for Kagzi Core

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

/// Kagzi core error type for framework operations
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

/// Classification of errors for retry logic and error handling
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorKind {
    // Transient errors - should retry
    NetworkError,
    ServiceUnavailable,
    RateLimited,
    Timeout,
    DatabaseConnectionFailed,

    // Permanent errors - don't retry
    ValidationFailed,
    NotFound,
    PermissionDenied,
    InvalidInput,
    BusinessLogicError,

    // Special
    Cancelled,
    Unknown,
}

impl ErrorKind {
    /// Returns true if this error kind is typically retryable
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            ErrorKind::NetworkError
                | ErrorKind::ServiceUnavailable
                | ErrorKind::RateLimited
                | ErrorKind::Timeout
                | ErrorKind::DatabaseConnectionFailed
        )
    }
}

impl std::fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ErrorKind::NetworkError => write!(f, "NetworkError"),
            ErrorKind::ServiceUnavailable => write!(f, "ServiceUnavailable"),
            ErrorKind::RateLimited => write!(f, "RateLimited"),
            ErrorKind::Timeout => write!(f, "Timeout"),
            ErrorKind::DatabaseConnectionFailed => write!(f, "DatabaseConnectionFailed"),
            ErrorKind::ValidationFailed => write!(f, "ValidationFailed"),
            ErrorKind::NotFound => write!(f, "NotFound"),
            ErrorKind::PermissionDenied => write!(f, "PermissionDenied"),
            ErrorKind::InvalidInput => write!(f, "InvalidInput"),
            ErrorKind::BusinessLogicError => write!(f, "BusinessLogicError"),
            ErrorKind::Cancelled => write!(f, "Cancelled"),
            ErrorKind::Unknown => write!(f, "Unknown"),
        }
    }
}

/// Structured error information for workflow steps
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepError {
    /// Classification of the error
    pub kind: ErrorKind,
    /// Human-readable error message
    pub message: String,
    /// Error chain/source information
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Additional context as key-value pairs
    #[serde(skip_serializing_if = "HashMap::is_empty", default)]
    pub context: HashMap<String, serde_json::Value>,
    /// Whether this error should trigger a retry
    pub retryable: bool,
    /// When the error occurred
    pub occurred_at: DateTime<Utc>,
}

impl StepError {
    /// Create a new StepError with the given kind and message
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        let retryable = kind.is_retryable();
        Self {
            kind,
            message: message.into(),
            source: None,
            context: HashMap::new(),
            retryable,
            occurred_at: Utc::now(),
        }
    }

    /// Add source error information
    pub fn with_source(mut self, source: impl std::fmt::Display) -> Self {
        self.source = Some(source.to_string());
        self
    }

    /// Add context key-value pair
    pub fn with_context(
        mut self,
        key: impl Into<String>,
        value: impl Serialize,
    ) -> Result<Self> {
        self.context
            .insert(key.into(), serde_json::to_value(value)?);
        Ok(self)
    }

    /// Set retryable flag (overrides default from ErrorKind)
    pub fn with_retryable(mut self, retryable: bool) -> Self {
        self.retryable = retryable;
        self
    }
}

impl std::fmt::Display for StepError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.message)
    }
}

impl std::error::Error for StepError {}

/// Convert from anyhow::Error to StepError
impl From<anyhow::Error> for StepError {
    fn from(e: anyhow::Error) -> Self {
        // Try to downcast to known error types, otherwise treat as unknown
        StepError::new(ErrorKind::Unknown, e.to_string()).with_source(format!("{:?}", e))
    }
}

/// Convert from sqlx::Error to StepError
impl From<sqlx::Error> for StepError {
    fn from(e: sqlx::Error) -> Self {
        let kind = match &e {
            sqlx::Error::PoolTimedOut | sqlx::Error::PoolClosed => {
                ErrorKind::DatabaseConnectionFailed
            }
            sqlx::Error::Io(_) => ErrorKind::NetworkError,
            sqlx::Error::RowNotFound => ErrorKind::NotFound,
            _ => ErrorKind::Unknown,
        };

        StepError::new(kind, e.to_string()).with_source(format!("{:?}", e))
    }
}

/// Workflow-level errors
#[derive(Debug, Error, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WorkflowError {
    #[error("Step failed: {step_id}")]
    StepFailed {
        step_id: String,
        #[serde(flatten)]
        error: StepError,
    },

    #[error("Timeout after {duration_ms}ms")]
    Timeout { duration_ms: u64 },

    #[error("Workflow cancelled")]
    Cancelled,
}

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

    #[test]
    fn test_error_kind_is_retryable() {
        // Transient errors should be retryable
        assert!(ErrorKind::NetworkError.is_retryable());
        assert!(ErrorKind::ServiceUnavailable.is_retryable());
        assert!(ErrorKind::RateLimited.is_retryable());
        assert!(ErrorKind::Timeout.is_retryable());
        assert!(ErrorKind::DatabaseConnectionFailed.is_retryable());

        // Permanent errors should not be retryable
        assert!(!ErrorKind::ValidationFailed.is_retryable());
        assert!(!ErrorKind::NotFound.is_retryable());
        assert!(!ErrorKind::PermissionDenied.is_retryable());
        assert!(!ErrorKind::InvalidInput.is_retryable());
        assert!(!ErrorKind::BusinessLogicError.is_retryable());

        // Special errors should not be retryable
        assert!(!ErrorKind::Cancelled.is_retryable());
        assert!(!ErrorKind::Unknown.is_retryable());
    }

    #[test]
    fn test_step_error_new() {
        let err = StepError::new(ErrorKind::NetworkError, "Connection failed");
        assert_eq!(err.kind, ErrorKind::NetworkError);
        assert_eq!(err.message, "Connection failed");
        assert!(err.retryable);
        assert!(err.source.is_none());
        assert!(err.context.is_empty());
    }

    #[test]
    fn test_step_error_with_source() {
        let err = StepError::new(ErrorKind::NetworkError, "Connection failed")
            .with_source("TCP connection refused");
        assert_eq!(err.source, Some("TCP connection refused".to_string()));
    }

    #[test]
    fn test_step_error_with_context() {
        let err = StepError::new(ErrorKind::RateLimited, "Too many requests")
            .with_context("retry_after", 60)
            .unwrap();
        assert_eq!(err.context.get("retry_after").unwrap(), &serde_json::json!(60));
    }

    #[test]
    fn test_step_error_with_retryable() {
        // Override default retryable behavior
        let err = StepError::new(ErrorKind::Unknown, "Custom error").with_retryable(true);
        assert!(err.retryable);

        let err = StepError::new(ErrorKind::NetworkError, "Network error").with_retryable(false);
        assert!(!err.retryable);
    }

    #[test]
    fn test_step_error_serialization() {
        let err = StepError::new(ErrorKind::NetworkError, "Connection timeout")
            .with_source("TCP timeout");

        let json = serde_json::to_string(&err).unwrap();
        let deserialized: StepError = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.kind, ErrorKind::NetworkError);
        assert_eq!(deserialized.message, "Connection timeout");
        assert_eq!(deserialized.source, Some("TCP timeout".to_string()));
    }

    #[test]
    fn test_step_error_from_anyhow() {
        let anyhow_err = anyhow::anyhow!("Something went wrong");
        let step_err: StepError = anyhow_err.into();

        assert_eq!(step_err.kind, ErrorKind::Unknown);
        assert!(step_err.message.contains("Something went wrong"));
    }

    #[test]
    fn test_step_error_from_sqlx() {
        // Test RowNotFound
        let sqlx_err = sqlx::Error::RowNotFound;
        let step_err: StepError = sqlx_err.into();
        assert_eq!(step_err.kind, ErrorKind::NotFound);

        // Test PoolTimedOut
        let sqlx_err = sqlx::Error::PoolTimedOut;
        let step_err: StepError = sqlx_err.into();
        assert_eq!(step_err.kind, ErrorKind::DatabaseConnectionFailed);
    }

    #[test]
    fn test_error_kind_serialization() {
        let kind = ErrorKind::NetworkError;
        let json = serde_json::to_string(&kind).unwrap();
        assert_eq!(json, r#""NETWORK_ERROR""#);

        let deserialized: ErrorKind = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, ErrorKind::NetworkError);
    }

    #[test]
    fn test_workflow_error_serialization() {
        let step_error = StepError::new(ErrorKind::NetworkError, "Failed to fetch data");
        let workflow_error = WorkflowError::StepFailed {
            step_id: "fetch-user".to_string(),
            error: step_error,
        };

        let json = serde_json::to_string(&workflow_error).unwrap();
        let deserialized: WorkflowError = serde_json::from_str(&json).unwrap();

        match deserialized {
            WorkflowError::StepFailed { step_id, error } => {
                assert_eq!(step_id, "fetch-user");
                assert_eq!(error.kind, ErrorKind::NetworkError);
                assert_eq!(error.message, "Failed to fetch data");
            }
            _ => panic!("Expected StepFailed variant"),
        }
    }
}
