use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::warn;

/// Circuit breaker state for relay connections
#[derive(Debug, Clone, Default)]
struct CircuitBreakerState {
    failures: u32,
    last_failure: Option<Instant>,
    is_open: bool,
}

/// Circuit breaker for relay connections
pub struct CircuitBreaker {
    states: Arc<RwLock<HashMap<String, CircuitBreakerState>>>,
    failure_threshold: u32,
    reset_timeout: Duration,
}

impl CircuitBreaker {
    /// Create a new circuit breaker with specified failure threshold and reset timeout
    pub fn new(failure_threshold: u32, reset_timeout: Duration) -> Self {
        Self {
            states: Arc::new(RwLock::new(HashMap::with_capacity(10))), // Pre-allocate for typical relay count
            failure_threshold,
            reset_timeout,
        }
    }

    /// Check if circuit is open for a relay
    pub async fn is_open(&self, relay_id: &str) -> bool {
        let mut states = self.states.write().await;
        let state = states.entry(relay_id.to_string()).or_default();

        // Check if we should reset the circuit
        if state.is_open {
            if let Some(last_failure) = state.last_failure {
                if last_failure.elapsed() > self.reset_timeout {
                    state.is_open = false;
                    state.failures = 0;
                    state.last_failure = None;
                }
            }
        }

        state.is_open
    }

    /// Record a successful request
    pub async fn record_success(&self, relay_id: &str) {
        let mut states = self.states.write().await;
        if let Some(state) = states.get_mut(relay_id) {
            state.failures = 0;
            state.is_open = false;
            state.last_failure = None;
        }
    }

    /// Record a failed request
    pub async fn record_failure(&self, relay_id: &str) {
        let mut states = self.states.write().await;
        let state = states.entry(relay_id.to_string()).or_default();

        state.failures += 1;
        state.last_failure = Some(Instant::now());

        if state.failures >= self.failure_threshold {
            state.is_open = true;
            warn!(
                relay_id = relay_id,
                failures = state.failures,
                "Circuit breaker opened for relay"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_circuit_breaker() {
        let cb = CircuitBreaker::new(3, Duration::from_millis(100));
        let relay_id = "test-relay";

        // Initially closed
        assert!(!cb.is_open(relay_id).await);

        // Record failures
        cb.record_failure(relay_id).await;
        cb.record_failure(relay_id).await;
        assert!(!cb.is_open(relay_id).await);

        // Third failure opens the circuit
        cb.record_failure(relay_id).await;
        assert!(cb.is_open(relay_id).await);

        // Success resets the circuit
        cb.record_success(relay_id).await;
        assert!(!cb.is_open(relay_id).await);
    }

    #[tokio::test]
    async fn test_circuit_breaker_timeout_reset() {
        let cb = CircuitBreaker::new(1, Duration::from_millis(50));
        let relay_id = "timeout-test-relay";

        // Open the circuit
        cb.record_failure(relay_id).await;
        assert!(cb.is_open(relay_id).await);

        // Wait for reset timeout
        tokio::time::sleep(Duration::from_millis(60)).await;

        // Circuit should be closed after timeout
        assert!(!cb.is_open(relay_id).await);
    }
}
