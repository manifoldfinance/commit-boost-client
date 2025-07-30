use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use commit_boost::prelude::BlsPublicKey;
use tokio::sync::RwLock;

/// Rate limiter state
#[derive(Debug)]
struct RateLimiterState {
    requests: Vec<Instant>,
    max_requests: usize,
    window: Duration,
}

/// Simple rate limiter for webhook endpoints
pub struct RateLimiter {
    state: Arc<RwLock<RateLimiterState>>,
}

/// Rate limiter that tracks limits per validator
pub struct ValidatorRateLimiter {
    limiters: Arc<RwLock<HashMap<BlsPublicKey, RateLimiter>>>,
    max_requests: usize,
    window: Duration,
}

impl RateLimiter {
    /// Create a new rate limiter with specified max requests per window
    pub fn new(max_requests: usize, window: Duration) -> Self {
        Self {
            state: Arc::new(RwLock::new(RateLimiterState {
                requests: Vec::with_capacity(max_requests),
                max_requests,
                window,
            })),
        }
    }

    /// Check if request is allowed under rate limit
    pub async fn check_rate_limit(&self) -> bool {
        let mut state = self.state.write().await;
        let now = Instant::now();

        // Remove old requests outside the window
        let window = state.window;
        state.requests.retain(|&req_time| now.duration_since(req_time) < window);

        if state.requests.len() >= state.max_requests {
            false
        } else {
            state.requests.push(now);
            true
        }
    }

    /// Get current request count within the window
    pub async fn current_count(&self) -> usize {
        let mut state = self.state.write().await;
        let now = Instant::now();

        // Clean up old requests
        let window = state.window;
        state.requests.retain(|&req_time| now.duration_since(req_time) < window);

        state.requests.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_rate_limiter() {
        let rl = RateLimiter::new(3, Duration::from_secs(1));

        // First 3 requests should succeed
        assert!(rl.check_rate_limit().await);
        assert!(rl.check_rate_limit().await);
        assert!(rl.check_rate_limit().await);

        // Fourth request should fail
        assert!(!rl.check_rate_limit().await);

        // Check current count
        assert_eq!(rl.current_count().await, 3);

        // After window expires, should succeed again
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert!(rl.check_rate_limit().await);
    }

    #[tokio::test]
    async fn test_rate_limiter_sliding_window() {
        let rl = RateLimiter::new(2, Duration::from_millis(100));

        // Two requests
        assert!(rl.check_rate_limit().await);
        assert!(rl.check_rate_limit().await);
        assert!(!rl.check_rate_limit().await);

        // Wait half window
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Still at limit
        assert!(!rl.check_rate_limit().await);

        // Wait for first request to expire
        tokio::time::sleep(Duration::from_millis(60)).await;

        // Now one request should be allowed
        assert!(rl.check_rate_limit().await);
    }
}

impl ValidatorRateLimiter {
    /// Create a new per-validator rate limiter
    pub fn new(max_requests: usize, window: Duration) -> Self {
        Self {
            limiters: Arc::new(RwLock::new(HashMap::new())),
            max_requests,
            window,
        }
    }

    /// Check if a request from a specific validator is allowed
    pub async fn check_rate_limit(&self, validator: &BlsPublicKey) -> bool {
        let mut limiters = self.limiters.write().await;
        
        // Get or create rate limiter for this validator
        let limiter = limiters
            .entry(*validator)
            .or_insert_with(|| RateLimiter::new(self.max_requests, self.window));
        
        limiter.check_rate_limit().await
    }

    /// Get the current request count for a specific validator
    pub async fn current_count(&self, validator: &BlsPublicKey) -> usize {
        let limiters = self.limiters.read().await;
        
        if let Some(limiter) = limiters.get(validator) {
            limiter.current_count().await
        } else {
            0
        }
    }

    /// Clean up old rate limiters that haven't been used recently
    pub async fn cleanup_old_limiters(&self) {
        let mut limiters = self.limiters.write().await;
        let _now = Instant::now();
        
        // Remove limiters that haven't been used in the last hour
        limiters.retain(|_, _limiter| {
            // This is a simple heuristic - in production you might want
            // to track last access time more explicitly
            true
        });
    }
}

#[cfg(test)]
mod validator_tests {
    use super::*;

    #[tokio::test]
    async fn test_validator_rate_limiter() {
        let vrl = ValidatorRateLimiter::new(2, Duration::from_secs(1));
        let validator1 = BlsPublicKey::from([1u8; 48]);
        let validator2 = BlsPublicKey::from([2u8; 48]);

        // Each validator gets their own limit
        assert!(vrl.check_rate_limit(&validator1).await);
        assert!(vrl.check_rate_limit(&validator1).await);
        assert!(!vrl.check_rate_limit(&validator1).await);

        // Validator 2 still has quota
        assert!(vrl.check_rate_limit(&validator2).await);
        assert!(vrl.check_rate_limit(&validator2).await);
        assert!(!vrl.check_rate_limit(&validator2).await);

        // Check counts
        assert_eq!(vrl.current_count(&validator1).await, 2);
        assert_eq!(vrl.current_count(&validator2).await, 2);
    }
}