use thiserror::Error;

#[derive(Error, Debug)]
pub enum StoreError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("{entity} not found: {id}")]
    NotFound { entity: &'static str, id: String },

    #[error("Invalid state: {message}")]
    InvalidState { message: String },

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Lock conflict: {message}")]
    LockConflict { message: String },

    #[error("Precondition failed: {message}")]
    PreconditionFailed { message: String },
}

impl StoreError {
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
}
