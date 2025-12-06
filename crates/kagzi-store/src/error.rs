use thiserror::Error;

#[derive(Error, Debug)]
pub enum StoreError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Invalid argument: {message}")]
    InvalidArgument { message: String },

    #[error("{entity} not found: {id}")]
    NotFound { entity: &'static str, id: String },

    #[error("Invalid state: {message}")]
    InvalidState { message: String },

    #[error("Already completed: {message}")]
    AlreadyCompleted { message: String },

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Lock conflict: {message}")]
    LockConflict { message: String },

    #[error("Precondition failed: {message}")]
    PreconditionFailed { message: String },

    #[error("Unauthorized: {message}")]
    Unauthorized { message: String },

    #[error("Service unavailable: {message}")]
    Unavailable { message: String },

    #[error("Operation timed out: {message}")]
    Timeout { message: String },
}

impl StoreError {
    pub fn invalid_argument(message: impl Into<String>) -> Self {
        Self::InvalidArgument {
            message: message.into(),
        }
    }

    pub fn not_found(entity: &'static str, id: impl Into<String>) -> Self {
        Self::NotFound {
            entity,
            id: id.into(),
        }
    }

    pub fn invalid_state(message: impl Into<String>) -> Self {
        Self::InvalidState {
            message: message.into(),
        }
    }

    pub fn already_completed(message: impl Into<String>) -> Self {
        Self::AlreadyCompleted {
            message: message.into(),
        }
    }

    pub fn lock_conflict(message: impl Into<String>) -> Self {
        Self::LockConflict {
            message: message.into(),
        }
    }

    pub fn precondition_failed(message: impl Into<String>) -> Self {
        Self::PreconditionFailed {
            message: message.into(),
        }
    }

    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::Unauthorized {
            message: message.into(),
        }
    }

    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::Unavailable {
            message: message.into(),
        }
    }

    pub fn timeout(message: impl Into<String>) -> Self {
        Self::Timeout {
            message: message.into(),
        }
    }
}
