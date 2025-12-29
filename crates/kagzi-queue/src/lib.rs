//! Queue notification system for Kagzi server.
//!
//! Provides a `QueueNotifier` trait for signaling when work is available,
//! with a PostgreSQL implementation using NOTIFY/LISTEN.

mod error;
mod postgres;
mod traits;

pub use error::QueueError;
pub use postgres::PostgresNotifier;
pub use traits::QueueNotifier;
