//! Retry configuration for workflows and steps.
//!
//! This module provides configurable retry policies with exponential backoff.

use std::time::Duration;

use kagzi_proto::kagzi::RetryPolicy as ProtoRetryPolicy;

/// Retry policy configuration for workflows and steps
#[derive(Clone, Debug)]
pub struct Retry {
    /// Number of retry attempts (0 = no retries, MAX = infinite)
    pub attempts: u32,

    /// Initial retry interval
    pub initial_interval: Duration,

    /// Maximum retry interval
    pub max_interval: Duration,

    /// Backoff coefficient (1.0 = linear, 2.0 = exponential)
    pub backoff_coefficient: f64,

    /// Error types that should not be retried
    pub non_retryable_errors: Vec<String>,
}

impl Default for Retry {
    fn default() -> Self {
        Self {
            attempts: 3,
            initial_interval: Duration::from_secs(1),
            max_interval: Duration::from_secs(60),
            backoff_coefficient: 2.0,
            non_retryable_errors: Vec::new(),
        }
    }
}

impl Retry {
    /// Create retry with N attempts and default exponential backoff
    pub fn attempts(n: u32) -> Self {
        Self {
            attempts: n,
            ..Default::default()
        }
    }

    /// Create retry with exponential backoff
    /// Default: initial=1s, max=60s, coefficient=2.0
    pub fn exponential(attempts: u32) -> Self {
        Self::attempts(attempts)
    }

    /// Create retry with fixed interval (no backoff)
    pub fn linear(attempts: u32, interval: Duration) -> Self {
        Self {
            attempts,
            initial_interval: interval,
            max_interval: interval,
            backoff_coefficient: 1.0,
            non_retryable_errors: Vec::new(),
        }
    }

    /// No retries - fail immediately
    pub fn none() -> Self {
        Self {
            attempts: 0,
            ..Default::default()
        }
    }

    /// Retry forever with exponential backoff
    pub fn forever() -> Self {
        Self {
            attempts: u32::MAX,
            ..Default::default()
        }
    }

    /// Set initial retry interval using Duration
    pub fn with_initial(mut self, duration: Duration) -> Self {
        self.initial_interval = duration;
        self
    }

    /// Set maximum retry interval using Duration
    pub fn with_max(mut self, duration: Duration) -> Self {
        self.max_interval = duration;
        self
    }

    /// Set backoff coefficient (default: 2.0)
    pub fn with_backoff(mut self, coefficient: f64) -> Self {
        self.backoff_coefficient = coefficient;
        self
    }

    /// Set error types that should not be retried
    pub fn with_non_retryable<I, S>(mut self, errors: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.non_retryable_errors = errors.into_iter().map(Into::into).collect();
        self
    }

    /// Validate this retry policy configuration
    ///
    /// # Errors
    /// Returns `anyhow::Error` if the configuration is invalid
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.initial_interval.as_nanos() == 0 {
            anyhow::bail!("initial_interval must be positive");
        }

        if self.max_interval.as_nanos() == 0 {
            anyhow::bail!("max_interval must be positive");
        }

        if self.backoff_coefficient < 1.0 {
            anyhow::bail!("backoff_coefficient must be >= 1.0");
        }

        if self.initial_interval > self.max_interval {
            anyhow::bail!("initial_interval cannot be greater than max_interval");
        }

        Ok(())
    }
}

// Convert to proto
impl From<Retry> for ProtoRetryPolicy {
    fn from(r: Retry) -> Self {
        Self {
            maximum_attempts: r.attempts as i32,
            initial_interval_ms: r.initial_interval.as_millis() as i64,
            backoff_coefficient: r.backoff_coefficient,
            maximum_interval_ms: r.max_interval.as_millis() as i64,
            non_retryable_errors: r.non_retryable_errors,
        }
    }
}
