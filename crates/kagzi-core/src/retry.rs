//! Retry policies and backoff strategies for workflow steps
//!
//! This module provides automatic retry capabilities with configurable policies:
//! - Exponential backoff with jitter
//! - Fixed interval retries
//! - No retry (fail immediately)
//!
//! Retry policies can be configured per-step and work with error classification
//! to determine whether errors are retryable.

use crate::error::{ErrorKind, StepError};
use chrono::{DateTime, Duration, Utc};
use rand::Rng;
use serde::{Deserialize, Serialize};

/// Retry policy configuration for step execution
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RetryPolicy {
    /// No retry - fail immediately on error
    None,

    /// Exponential backoff with optional jitter
    Exponential {
        /// Initial delay in milliseconds
        initial_interval_ms: u64,
        /// Maximum delay in milliseconds (caps exponential growth)
        max_interval_ms: u64,
        /// Backoff multiplier (e.g., 2.0 for doubling)
        multiplier: f64,
        /// Maximum number of retry attempts (0 = no retries)
        max_attempts: u32,
        /// Whether to add random jitter to prevent thundering herd
        use_jitter: bool,
    },

    /// Fixed interval retries
    Fixed {
        /// Delay between retries in milliseconds
        interval_ms: u64,
        /// Maximum number of retry attempts (0 = no retries)
        max_attempts: u32,
    },
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self::exponential()
    }
}

impl RetryPolicy {
    /// Create exponential backoff policy with sensible defaults
    ///
    /// - Initial interval: 1 second
    /// - Max interval: 1 hour
    /// - Multiplier: 2.0 (doubles each retry)
    /// - Max attempts: 5
    /// - Jitter: enabled
    pub fn exponential() -> Self {
        Self::Exponential {
            initial_interval_ms: 1000,      // 1 second
            max_interval_ms: 3_600_000,     // 1 hour
            multiplier: 2.0,
            max_attempts: 5,
            use_jitter: true,
        }
    }

    /// Create exponential backoff with custom settings
    pub fn exponential_with(
        initial_interval_ms: u64,
        max_interval_ms: u64,
        multiplier: f64,
        max_attempts: u32,
        use_jitter: bool,
    ) -> Self {
        Self::Exponential {
            initial_interval_ms,
            max_interval_ms,
            multiplier,
            max_attempts,
            use_jitter,
        }
    }

    /// Create fixed interval retry policy
    ///
    /// - Interval: 5 seconds
    /// - Max attempts: 3
    pub fn fixed() -> Self {
        Self::Fixed {
            interval_ms: 5000,  // 5 seconds
            max_attempts: 3,
        }
    }

    /// Create fixed interval retry with custom settings
    pub fn fixed_with(interval_ms: u64, max_attempts: u32) -> Self {
        Self::Fixed {
            interval_ms,
            max_attempts,
        }
    }

    /// No retry policy - fail immediately
    pub fn none() -> Self {
        Self::None
    }

    /// Calculate the next retry time based on the current attempt number
    ///
    /// Returns None if no more retries should be attempted
    pub fn next_retry_at(&self, attempt: u32, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        match self {
            RetryPolicy::None => None,

            RetryPolicy::Exponential {
                initial_interval_ms,
                max_interval_ms,
                multiplier,
                max_attempts,
                use_jitter,
            } => {
                if attempt >= *max_attempts {
                    return None;
                }

                let delay_ms = calculate_exponential_backoff(
                    *initial_interval_ms,
                    *max_interval_ms,
                    *multiplier,
                    attempt,
                    *use_jitter,
                );

                Some(now + Duration::milliseconds(delay_ms as i64))
            }

            RetryPolicy::Fixed {
                interval_ms,
                max_attempts,
            } => {
                if attempt >= *max_attempts {
                    return None;
                }

                Some(now + Duration::milliseconds(*interval_ms as i64))
            }
        }
    }

    /// Check if this policy allows retries
    pub fn allows_retry(&self) -> bool {
        !matches!(self, RetryPolicy::None)
    }

    /// Get the maximum number of attempts for this policy
    pub fn max_attempts(&self) -> u32 {
        match self {
            RetryPolicy::None => 0,
            RetryPolicy::Exponential { max_attempts, .. } => *max_attempts,
            RetryPolicy::Fixed { max_attempts, .. } => *max_attempts,
        }
    }
}

/// Calculate exponential backoff delay with optional jitter
fn calculate_exponential_backoff(
    initial_interval_ms: u64,
    max_interval_ms: u64,
    multiplier: f64,
    attempt: u32,
    use_jitter: bool,
) -> u64 {
    // Calculate base delay: initial * multiplier^attempt
    let delay = (initial_interval_ms as f64) * multiplier.powi(attempt as i32);

    // Cap at max interval
    let delay = delay.min(max_interval_ms as f64) as u64;

    // Apply jitter if enabled (±25% randomness)
    if use_jitter {
        apply_jitter(delay)
    } else {
        delay
    }
}

/// Apply random jitter to a delay (±25%)
fn apply_jitter(delay_ms: u64) -> u64 {
    let mut rng = rand::thread_rng();
    let jitter_range = (delay_ms as f64) * 0.25;
    let jitter = rng.gen_range(-jitter_range..=jitter_range);
    ((delay_ms as f64) + jitter).max(0.0) as u64
}

/// Predicate for determining if a step should be retried
#[derive(Debug, Clone, PartialEq, Default)]
pub enum RetryPredicate {
    /// Retry only if the error is classified as retryable
    #[default]
    OnRetryableError,
    /// Retry on all errors
    OnAnyError,
    /// Never retry
    Never,
    /// Custom predicate based on error kind
    OnErrorKind(Vec<ErrorKind>),
}

impl RetryPredicate {
    /// Check if a step should be retried based on the error
    pub fn should_retry(&self, error: &StepError) -> bool {
        match self {
            RetryPredicate::OnRetryableError => error.retryable,
            RetryPredicate::OnAnyError => true,
            RetryPredicate::Never => false,
            RetryPredicate::OnErrorKind(kinds) => kinds.contains(&error.kind),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_retry_policy_default() {
        let policy = RetryPolicy::default();
        assert!(matches!(policy, RetryPolicy::Exponential { .. }));
    }

    #[test]
    fn test_exponential_backoff_calculation() {
        // Test exponential growth
        let delay0 = calculate_exponential_backoff(1000, 60_000, 2.0, 0, false);
        let delay1 = calculate_exponential_backoff(1000, 60_000, 2.0, 1, false);
        let delay2 = calculate_exponential_backoff(1000, 60_000, 2.0, 2, false);

        assert_eq!(delay0, 1000);  // 1s
        assert_eq!(delay1, 2000);  // 2s
        assert_eq!(delay2, 4000);  // 4s
    }

    #[test]
    fn test_exponential_backoff_max_cap() {
        // Test that delay is capped at max_interval
        let delay = calculate_exponential_backoff(1000, 5000, 2.0, 10, false);
        assert_eq!(delay, 5000);  // Capped at 5s instead of 1024s
    }

    #[test]
    fn test_exponential_backoff_with_jitter() {
        // Test that jitter produces values within expected range
        let base_delay = 1000u64;

        for _ in 0..100 {
            let delay = calculate_exponential_backoff(base_delay, 60_000, 2.0, 0, true);
            // Should be within ±25% of base delay
            assert!(delay >= 750 && delay <= 1250, "Delay {} outside jitter range", delay);
        }
    }

    #[test]
    fn test_retry_policy_next_retry_at_exponential() {
        let policy = RetryPolicy::exponential_with(1000, 60_000, 2.0, 3, false);
        let now = Utc::now();

        // Attempt 0 should retry after 1s
        let next = policy.next_retry_at(0, now).unwrap();
        let diff = (next - now).num_milliseconds();
        assert_eq!(diff, 1000);

        // Attempt 1 should retry after 2s
        let next = policy.next_retry_at(1, now).unwrap();
        let diff = (next - now).num_milliseconds();
        assert_eq!(diff, 2000);

        // Attempt 2 should retry after 4s
        let next = policy.next_retry_at(2, now).unwrap();
        let diff = (next - now).num_milliseconds();
        assert_eq!(diff, 4000);

        // Attempt 3 should not retry (max_attempts = 3)
        assert!(policy.next_retry_at(3, now).is_none());
    }

    #[test]
    fn test_retry_policy_next_retry_at_fixed() {
        let policy = RetryPolicy::fixed_with(5000, 2);
        let now = Utc::now();

        // All attempts should have same delay
        let next0 = policy.next_retry_at(0, now).unwrap();
        let next1 = policy.next_retry_at(1, now).unwrap();

        assert_eq!((next0 - now).num_milliseconds(), 5000);
        assert_eq!((next1 - now).num_milliseconds(), 5000);

        // Should not retry after max_attempts
        assert!(policy.next_retry_at(2, now).is_none());
    }

    #[test]
    fn test_retry_policy_next_retry_at_none() {
        let policy = RetryPolicy::none();
        let now = Utc::now();

        assert!(policy.next_retry_at(0, now).is_none());
        assert!(policy.next_retry_at(1, now).is_none());
    }

    #[test]
    fn test_retry_policy_allows_retry() {
        assert!(RetryPolicy::exponential().allows_retry());
        assert!(RetryPolicy::fixed().allows_retry());
        assert!(!RetryPolicy::none().allows_retry());
    }

    #[test]
    fn test_retry_policy_max_attempts() {
        assert_eq!(RetryPolicy::exponential().max_attempts(), 5);
        assert_eq!(RetryPolicy::fixed().max_attempts(), 3);
        assert_eq!(RetryPolicy::none().max_attempts(), 0);
    }

    #[test]
    fn test_retry_policy_serialization() {
        let policy = RetryPolicy::exponential_with(1000, 60_000, 2.0, 5, true);
        let json = serde_json::to_string(&policy).unwrap();
        let deserialized: RetryPolicy = serde_json::from_str(&json).unwrap();
        assert_eq!(policy, deserialized);

        let policy = RetryPolicy::fixed_with(5000, 3);
        let json = serde_json::to_string(&policy).unwrap();
        let deserialized: RetryPolicy = serde_json::from_str(&json).unwrap();
        assert_eq!(policy, deserialized);

        let policy = RetryPolicy::none();
        let json = serde_json::to_string(&policy).unwrap();
        let deserialized: RetryPolicy = serde_json::from_str(&json).unwrap();
        assert_eq!(policy, deserialized);
    }

    #[test]
    fn test_retry_predicate_on_retryable_error() {
        let predicate = RetryPredicate::OnRetryableError;

        let retryable_error = StepError::new(ErrorKind::NetworkError, "Connection failed");
        assert!(predicate.should_retry(&retryable_error));

        let permanent_error = StepError::new(ErrorKind::NotFound, "Resource not found");
        assert!(!predicate.should_retry(&permanent_error));
    }

    #[test]
    fn test_retry_predicate_on_any_error() {
        let predicate = RetryPredicate::OnAnyError;

        let error1 = StepError::new(ErrorKind::NetworkError, "Connection failed");
        let error2 = StepError::new(ErrorKind::NotFound, "Resource not found");

        assert!(predicate.should_retry(&error1));
        assert!(predicate.should_retry(&error2));
    }

    #[test]
    fn test_retry_predicate_never() {
        let predicate = RetryPredicate::Never;

        let error = StepError::new(ErrorKind::NetworkError, "Connection failed");
        assert!(!predicate.should_retry(&error));
    }

    #[test]
    fn test_retry_predicate_on_error_kind() {
        let predicate = RetryPredicate::OnErrorKind(vec![
            ErrorKind::NetworkError,
            ErrorKind::Timeout,
        ]);

        let network_error = StepError::new(ErrorKind::NetworkError, "Connection failed");
        let timeout_error = StepError::new(ErrorKind::Timeout, "Request timed out");
        let not_found_error = StepError::new(ErrorKind::NotFound, "Resource not found");

        assert!(predicate.should_retry(&network_error));
        assert!(predicate.should_retry(&timeout_error));
        assert!(!predicate.should_retry(&not_found_error));
    }

    #[test]
    fn test_retry_predicate_default() {
        let predicate = RetryPredicate::default();
        assert_eq!(predicate, RetryPredicate::OnRetryableError);
    }

    #[test]
    fn test_jitter_range() {
        // Test jitter multiple times to ensure it's within range
        for _ in 0..100 {
            let jittered = apply_jitter(1000);
            assert!(jittered >= 750 && jittered <= 1250);
        }
    }

    #[test]
    fn test_exponential_growth_sequence() {
        // Test a realistic exponential backoff sequence
        let delays: Vec<u64> = (0..5)
            .map(|attempt| calculate_exponential_backoff(1000, 300_000, 2.0, attempt, false))
            .collect();

        assert_eq!(delays, vec![
            1_000,    // 1s
            2_000,    // 2s
            4_000,    // 4s
            8_000,    // 8s
            16_000,   // 16s
        ]);
    }
}
