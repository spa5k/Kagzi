use std::time::Duration;

use kagzi_proto::kagzi::RetryPolicy as ProtoRetryPolicy;

#[derive(Clone, Debug)]
pub struct Retry {
    pub attempts: u32,
    pub initial_interval: Duration,
    pub max_interval: Duration,
    pub backoff_coefficient: f64,
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
    pub fn linear(attempts: u32, interval: &str) -> Self {
        let d = humantime::parse_duration(interval).expect("Invalid duration format");
        Self {
            attempts,
            initial_interval: d,
            max_interval: d,
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

    /// Set initial retry interval (e.g., "1s", "500ms", "1m")
    pub fn initial(mut self, duration: &str) -> Self {
        self.initial_interval =
            humantime::parse_duration(duration).expect("Invalid duration format");
        self
    }

    /// Set maximum retry interval (e.g., "60s", "5m")
    pub fn max(mut self, duration: &str) -> Self {
        self.max_interval = humantime::parse_duration(duration).expect("Invalid duration format");
        self
    }

    /// Set backoff coefficient (default: 2.0)
    pub fn backoff(mut self, coefficient: f64) -> Self {
        self.backoff_coefficient = coefficient;
        self
    }

    /// Set error types that should not be retried
    pub fn non_retryable<I, S>(mut self, errors: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.non_retryable_errors = errors.into_iter().map(Into::into).collect();
        self
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
