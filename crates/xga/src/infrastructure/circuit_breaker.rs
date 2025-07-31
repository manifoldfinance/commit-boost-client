use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use tokio::sync::RwLock;
use tracing::warn;

/// Circuit breaker state for relay connections
#[derive(Debug, Clone, Default)]
struct CircuitBreakerState {
    failures: u32,
    last_failure: Option<Instant>,
    is_open: bool,
    half_open: bool,
}

/// Circuit breaker for managing relay connection failures.
///
/// This implementation provides fault tolerance by temporarily disabling
/// connections to relays that are experiencing failures. Once a relay
/// exceeds the failure threshold, the circuit "opens" and subsequent
/// requests are rejected until the reset timeout expires.
///
/// # Features
///
/// - Per-relay state tracking
/// - Configurable failure threshold
/// - Automatic reset after timeout
/// - Thread-safe operation
///
/// # Example
///
/// ```no_run
/// # use xga_commitment::infrastructure::CircuitBreaker;
/// # use std::time::Duration;
/// # async fn example() {
/// let circuit_breaker = CircuitBreaker::new(
///     3, // Open circuit after 3 failures
///     Duration::from_secs(60), // Reset after 60 seconds
/// );
///
/// // Check if relay is available
/// if !circuit_breaker.is_open("relay1").await {
///     // Try to use relay
///     match send_to_relay().await {
///         Ok(_) => circuit_breaker.record_success("relay1").await,
///         Err(_) => circuit_breaker.record_failure("relay1").await,
///     }
/// }
/// # async fn send_to_relay() -> Result<(), ()> { Ok(()) }
/// # }
/// ```
#[derive(Clone)]
pub struct CircuitBreaker {
    states: Arc<RwLock<HashMap<String, CircuitBreakerState>>>,
    failure_threshold: u32,
    reset_timeout: Duration,
}

impl CircuitBreaker {
    /// Creates a new circuit breaker with specified failure threshold and reset timeout.
    ///
    /// # Arguments
    ///
    /// * `failure_threshold` - Number of failures before opening the circuit
    /// * `reset_timeout` - Duration to wait before attempting to reset an open circuit
    ///
    /// # Example
    ///
    /// ```
    /// # use xga_commitment::infrastructure::CircuitBreaker;
    /// # use std::time::Duration;
    /// let cb = CircuitBreaker::new(3, Duration::from_secs(30));
    /// ```
    pub fn new(failure_threshold: u32, reset_timeout: Duration) -> Self {
        Self {
            states: Arc::new(RwLock::new(HashMap::with_capacity(10))),
            failure_threshold,
            reset_timeout,
        }
    }

    /// Checks if the circuit is open for a specific relay.
    ///
    /// This method also handles automatic circuit reset. If the circuit is open
    /// but the reset timeout has expired, it will transition to half-open state.
    /// In half-open state, one request is allowed through to test if the relay
    /// has recovered.
    ///
    /// # Arguments
    ///
    /// * `relay_id` - Identifier of the relay to check
    ///
    /// # Returns
    ///
    /// Returns `true` if the circuit is open (relay should not be used),
    /// `false` if the circuit is closed or half-open (relay can be used).
    pub async fn is_open(&self, relay_id: &str) -> bool {
        let mut states = self.states.write().await;
        let state = states.entry(relay_id.to_string()).or_default();

        // Check if we should transition from open to half-open
        if state.is_open && !state.half_open {
            if let Some(last_failure) = state.last_failure {
                if last_failure.elapsed() > self.reset_timeout {
                    state.half_open = true;
                    return false; // Allow one test request through
                }
            }
        }

        state.is_open && !state.half_open
    }

    /// Records a successful request to a relay.
    ///
    /// This method resets the failure count and closes the circuit if it was open,
    /// indicating that the relay is functioning properly again. If the circuit was
    /// in half-open state, this confirms the relay has recovered.
    ///
    /// # Arguments
    ///
    /// * `relay_id` - Identifier of the relay that succeeded
    pub async fn record_success(&self, relay_id: &str) {
        let mut states = self.states.write().await;
        if let Some(state) = states.get_mut(relay_id) {
            state.failures = 0;
            state.is_open = false;
            state.half_open = false;
            state.last_failure = None;
        }
    }

    /// Records a failed request to a relay.
    ///
    /// This method increments the failure count and opens the circuit if the
    /// failure threshold is reached. If the circuit was in half-open state,
    /// it immediately reopens the circuit without incrementing the failure count.
    ///
    /// # Arguments
    ///
    /// * `relay_id` - Identifier of the relay that failed
    pub async fn record_failure(&self, relay_id: &str) {
        let mut states = self.states.write().await;
        let state = states.entry(relay_id.to_string()).or_default();

        // If we're in half-open state, immediately reopen the circuit
        if state.half_open {
            state.is_open = true;
            state.half_open = false;
            state.last_failure = Some(Instant::now());
            warn!(
                relay_id = relay_id,
                "Circuit breaker reopened for relay after half-open test failed"
            );
            return;
        }

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
    async fn test_circuit_breaker_half_open_state() {
        let cb = CircuitBreaker::new(1, Duration::from_millis(50));
        let relay_id = "half-open-test-relay";

        // Open the circuit
        cb.record_failure(relay_id).await;
        assert!(cb.is_open(relay_id).await);

        // Wait for reset timeout
        tokio::time::sleep(Duration::from_millis(60)).await;

        // Circuit should transition to half-open (allowing one request)
        assert!(!cb.is_open(relay_id).await);
        
        // Successful request should close the circuit
        cb.record_success(relay_id).await;
        assert!(!cb.is_open(relay_id).await);
    }
    
    #[tokio::test]
    async fn test_circuit_breaker_half_open_failure() {
        let cb = CircuitBreaker::new(1, Duration::from_millis(50));
        let relay_id = "half-open-fail-relay";

        // Open the circuit
        cb.record_failure(relay_id).await;
        assert!(cb.is_open(relay_id).await);

        // Wait for reset timeout
        tokio::time::sleep(Duration::from_millis(60)).await;

        // Circuit should transition to half-open (allowing one request)
        assert!(!cb.is_open(relay_id).await);
        
        // Failed request should reopen the circuit
        cb.record_failure(relay_id).await;
        assert!(cb.is_open(relay_id).await);
    }
}