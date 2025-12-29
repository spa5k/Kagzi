use std::time::Duration;

use kagzi_proto::kagzi::RetryPolicy as ProtoRetryPolicy;

#[derive(Clone, Default)]
pub struct RetryPolicy {
    pub maximum_attempts: Option<i32>,
    pub initial_interval: Option<Duration>,
    pub backoff_coefficient: Option<f64>,
    pub maximum_interval: Option<Duration>,
    pub non_retryable_errors: Vec<String>,
}

impl From<RetryPolicy> for ProtoRetryPolicy {
    fn from(p: RetryPolicy) -> Self {
        Self {
            maximum_attempts: p.maximum_attempts.unwrap_or(0),
            initial_interval_ms: p
                .initial_interval
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0),
            backoff_coefficient: p.backoff_coefficient.unwrap_or(0.0),
            maximum_interval_ms: p
                .maximum_interval
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0),
            non_retryable_errors: p.non_retryable_errors,
        }
    }
}
