use thiserror::Error;

#[derive(Error, Debug)]
pub enum StoreError {
    #[error("Database error: {0}")]
    Database(sqlx::Error),

    #[error("Conflict: {message}")]
    Conflict { message: String },

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

    pub fn is_unique_violation(&self) -> bool {
        matches!(self, Self::Database(sqlx::Error::Database(db_err)) if db_err.code().is_some_and(|c| c == "23505"))
    }
}

impl From<sqlx::Error> for StoreError {
    fn from(e: sqlx::Error) -> Self {
        if let sqlx::Error::Database(db_err) = &e {
            match db_err.code().as_deref() {
                Some("23505") => {
                    return StoreError::Conflict {
                        message: db_err.message().to_string(),
                    };
                }
                Some("23503") => {
                    return StoreError::InvalidArgument {
                        message: db_err.message().to_string(),
                    };
                }
                _ => {}
            }
        }
        StoreError::Database(e)
    }
}
