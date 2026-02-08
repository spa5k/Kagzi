//! Telemetry module for Kagzi server.
//!
//! Provides OpenTelemetry-based observability with:
//! - Tracing (distributed traces)
//! - Logs (structured logging via tracing)
//! - Metrics (counters, histograms)
//!
//! Uses W3C Trace Context for cross-service context propagation,
//! enabling distributed tracing with any SDK (Rust, Python, Go, etc.).

mod init;
mod propagation;

pub use init::{TelemetryGuard, init_telemetry};
pub use propagation::{extract_context, inject_context};
