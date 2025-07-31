use std::future::Future;
use std::time::Duration;

use tokio_retry2::{Retry, RetryError};
use tokio_retry2::strategy::{ExponentialBackoff, jitter, FixedInterval};

use crate::config::RetryConfig;

/// Create a retry strategy for polling operations
pub fn polling_retry_strategy(config: &RetryConfig) -> impl Iterator<Item = Duration> {
    ExponentialBackoff::from_millis(config.initial_backoff_ms)
        .factor(2)
        .max_delay_millis(config.max_backoff_secs * 1000)
        .map(jitter)
        .take(config.max_retries as usize)
}

/// Create a retry strategy for signing operations (fewer retries, fixed delay)
pub fn signing_retry_strategy() -> impl Iterator<Item = Duration> {
    FixedInterval::from_millis(500)
        .take(2)
}

/// Create a retry strategy for relay send operations
pub fn relay_send_retry_strategy(config: &RetryConfig) -> impl Iterator<Item = Duration> {
    ExponentialBackoff::from_millis(config.initial_backoff_ms)
        .factor(2)
        .max_delay_millis(config.max_backoff_secs * 1000)
        .map(jitter)
        .take(config.max_retries as usize)
}

/// Execute an operation with retry using the provided strategy
pub async fn execute_with_retry<F, Fut, T, E, I>(
    strategy: I,
    operation: F,
) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, RetryError<E>>>,
    I: IntoIterator<Item = Duration>,
{
    Retry::spawn(strategy, operation).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;
    use std::time::Instant;

    #[tokio::test]
    async fn test_retry_strategy_retries_on_failure() {
        let attempt_count = Arc::new(AtomicU32::new(0));
        let count_clone = attempt_count.clone();

        let strategy = FixedInterval::from_millis(10).take(3);

        let action = || {
            let count_clone = count_clone.clone();
            async move {
                let count = count_clone.fetch_add(1, Ordering::SeqCst);
                if count < 2 {
                    Err(RetryError::transient("simulated failure"))
                } else {
                    Ok("success")
                }
            }
        };

        let result = Retry::spawn(strategy, action).await;
        assert_eq!(result.unwrap(), "success");
        assert_eq!(attempt_count.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_exponential_backoff_timing() {
        let start = Instant::now();
        let config = RetryConfig {
            max_retries: 2,
            initial_backoff_ms: 100,
            max_backoff_secs: 1,
        };
        let strategy = polling_retry_strategy(&config);

        let action = || async {
            Err::<(), RetryError<&str>>(RetryError::transient("always fail"))
        };

        let _ = Retry::spawn(strategy, action).await;

        let elapsed = start.elapsed();
        // Should have delays of ~100ms and ~200ms (with some jitter)
        // Jitter can add up to 100% of the delay, and there are potential system delays
        assert!(elapsed >= Duration::from_millis(100), "Expected at least 100ms, got {:?}", elapsed);
        assert!(elapsed < Duration::from_secs(2), "Expected less than 2s, got {:?}", elapsed);
    }

    #[tokio::test]
    async fn test_max_retries_respected() {
        let attempt_count = Arc::new(AtomicU32::new(0));
        let count_clone = attempt_count.clone();

        let strategy = FixedInterval::from_millis(1).take(5);

        let action = || {
            let count_clone = count_clone.clone();
            async move {
                count_clone.fetch_add(1, Ordering::SeqCst);
                Err::<(), RetryError<&str>>(RetryError::transient("always fail"))
            }
        };

        let _ = Retry::spawn(strategy, action).await;

        // Initial attempt + 5 retries = 6 total attempts
        assert_eq!(attempt_count.load(Ordering::SeqCst), 6);
    }

    #[tokio::test]
    async fn test_signing_retry_strategy() {
        let strategy = signing_retry_strategy();
        let start = Instant::now();

        let action = || async {
            Err::<(), RetryError<&str>>(RetryError::transient("fail twice"))
        };

        let _ = Retry::spawn(strategy, action).await;

        let elapsed = start.elapsed();
        // Should have 2 retries with 500ms delay each
        assert!(elapsed >= Duration::from_millis(1000));
        assert!(elapsed < Duration::from_millis(1200));
    }

    #[tokio::test]
    async fn test_permanent_error_stops_retry() {
        let attempt_count = Arc::new(AtomicU32::new(0));
        let count_clone = attempt_count.clone();

        let strategy = FixedInterval::from_millis(10).take(5);

        let action = || {
            let count_clone = count_clone.clone();
            async move {
                let count = count_clone.fetch_add(1, Ordering::SeqCst);
                if count == 0 {
                    // First attempt fails with transient error
                    Err::<&str, RetryError<&str>>(RetryError::transient("transient error"))
                } else {
                    // Second attempt fails with permanent error
                    Err(RetryError::permanent("permanent error"))
                }
            }
        };

        let result = Retry::spawn(strategy, action).await;
        // The result should be an error (the permanent error message)
        assert!(result.is_err());
        assert_eq!(result.err().unwrap(), "permanent error");
        // Should stop after permanent error (2 attempts total)
        assert_eq!(attempt_count.load(Ordering::SeqCst), 2);
    }
}