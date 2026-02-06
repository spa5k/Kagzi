//! Kagzi queue signaling (work-signal bus).
//!
//! This crate provides a pluggable, lossy wakeup distribution mechanism used by Kagzi workers to
//! reduce DB polling. Signals are never claims; the authoritative store remains the only gateway
//! to execution.
//!
//! - Default backend: Postgres LISTEN/NOTIFY (`PostgresNotifier`)
//! - Future backends (not implemented yet): Kafka / NATS JetStream / RabbitMQ

mod bus;
mod error;
mod postgres;

pub use bus::{WorkAvailable, WorkSignalBus};
pub use error::QueueError;
pub use postgres::PostgresNotifier;
