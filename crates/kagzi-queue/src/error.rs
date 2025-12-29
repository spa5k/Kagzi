use thiserror::Error;

#[derive(Debug, Error)]
pub enum QueueError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Listener not started")]
    NotStarted,

    #[error("Queue error: {0}")]
    Other(String),
}
