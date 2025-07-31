use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use commit_boost::prelude::BlsPublicKey;
use tokio::sync::RwLock;
use tracing::debug;

/// Rate limiter state
#[derive(Debug)]
struct RateLimiterState {
    requests: Vec<Instant>,
    max_requests: usize,
    window: Duration,
}

/// Simple sliding window rate limiter.
///
/// This rate limiter uses a sliding window approach to track requests
/// within a time window. Old requests are automatically cleaned up
/// when checking the rate limit.
///
/// # Example
///
/// ```
/// # use xga_commitment::infrastructure::RateLimiter;
/// # use std::time::Duration;
/// # async fn example() {
/// let limiter = RateLimiter::new(10, Duration::from_secs(60));
/// 
/// if limiter.check_rate_limit().await {
///     // Request allowed
/// } else {
///     // Rate limit exceeded
/// }
/// # }
/// ```
pub struct RateLimiter {
    state: Arc<RwLock<RateLimiterState>>,
}

/// Per-validator rate limiter for XGA commitments.
///
/// This rate limiter maintains separate rate limits for each validator,
/// allowing fair access to the system regardless of overall load.
/// It automatically cleans up old limiters that haven't been used recently.
///
/// # Example
///
/// ```
/// # use xga_commitment::infrastructure::ValidatorRateLimiter;
/// # use std::time::Duration;
/// # use commit_boost::prelude::BlsPublicKey;
/// # async fn example() {
/// let limiter = ValidatorRateLimiter::new(5, Duration::from_secs(60));
/// let validator_key = BlsPublicKey::from([1u8; 48]);
/// 
/// if limiter.check_rate_limit(&validator_key).await {
///     // Request allowed for this validator
/// } else {
///     // Rate limit exceeded for this validator
/// }
/// # }
/// ```
pub struct ValidatorRateLimiter {
    limiters: Arc<RwLock<HashMap<BlsPublicKey, (RateLimiter, Instant)>>>,
    max_requests: usize,
    window: Duration,
}

impl RateLimiter {
    /// Creates a new rate limiter with specified max requests per window.
    ///
    /// # Arguments
    ///
    /// * `max_requests` - Maximum number of requests allowed in the window
    /// * `window` - Time window for rate limiting
    pub fn new(max_requests: usize, window: Duration) -> Self {
        Self {
            state: Arc::new(RwLock::new(RateLimiterState {
                requests: Vec::with_capacity(max_requests),
                max_requests,
                window,
            })),
        }
    }

    /// Checks if a request is allowed under the rate limit.
    ///
    /// This method automatically cleans up old requests outside the window
    /// before checking the limit.
    ///
    /// # Returns
    ///
    /// Returns `true` if the request is allowed, `false` if rate limit exceeded.
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

    /// Gets the current request count within the window.
    ///
    /// This method cleans up old requests before returning the count.
    ///
    /// # Returns
    ///
    /// The number of requests within the current time window.
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
    /// Creates a new per-validator rate limiter.
    ///
    /// # Arguments
    ///
    /// * `max_requests` - Maximum requests per validator per window
    /// * `window` - Time window for rate limiting
    pub fn new(max_requests: usize, window: Duration) -> Self {
        Self {
            limiters: Arc::new(RwLock::new(HashMap::new())),
            max_requests,
            window,
        }
    }

    /// Checks if a request from a specific validator is allowed.
    ///
    /// This method creates a new rate limiter for the validator if one
    /// doesn't exist, and updates the last access time.
    ///
    /// # Arguments
    ///
    /// * `validator` - The validator's public key
    ///
    /// # Returns
    ///
    /// Returns `true` if the request is allowed, `false` if rate limit exceeded.
    pub async fn check_rate_limit(&self, validator: &BlsPublicKey) -> bool {
        let mut limiters = self.limiters.write().await;
        let now = Instant::now();
        
        // Get or create rate limiter for this validator
        let (limiter, last_access) = limiters
            .entry(*validator)
            .or_insert_with(|| (RateLimiter::new(self.max_requests, self.window), now));
        
        // Update last access time
        *last_access = now;
        
        limiter.check_rate_limit().await
    }

    /// Gets the current request count for a specific validator.
    ///
    /// # Arguments
    ///
    /// * `validator` - The validator's public key
    ///
    /// # Returns
    ///
    /// The number of requests from this validator within the current window,
    /// or 0 if the validator has no recorded requests.
    pub async fn current_count(&self, validator: &BlsPublicKey) -> usize {
        let limiters = self.limiters.read().await;
        
        if let Some((limiter, _)) = limiters.get(validator) {
            limiter.current_count().await
        } else {
            0
        }
    }

    /// Cleans up old rate limiters that haven't been used recently.
    ///
    /// This method removes rate limiters that haven't been accessed in
    /// the last hour, helping to prevent memory leaks from validators
    /// that are no longer active.
    ///
    /// # Returns
    ///
    /// The number of rate limiters that were removed.
    ///
    /// # Example
    ///
    /// ```
    /// # use xga_commitment::infrastructure::ValidatorRateLimiter;
    /// # use std::time::Duration;
    /// # async fn example() {
    /// let limiter = ValidatorRateLimiter::new(5, Duration::from_secs(60));
    /// 
    /// // Periodically clean up old limiters
    /// let removed = limiter.cleanup_old_limiters().await;
    /// println!("Removed {} old limiters", removed);
    /// # }
    /// ```
    pub async fn cleanup_old_limiters(&self) -> usize {
        let mut limiters = self.limiters.write().await;
        let now = Instant::now();
        let initial_count = limiters.len();
        
        // Remove limiters that haven't been used in the last hour
        limiters.retain(|_, (_, last_access)| {
            now.duration_since(*last_access) < Duration::from_secs(3600)
        });
        
        let removed_count = initial_count - limiters.len();
        if removed_count > 0 {
            debug!("Cleaned up {} old rate limiters", removed_count);
        }
        
        removed_count
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

    #[tokio::test]
    async fn test_cleanup_old_limiters() {
        // Create limiter with short window for testing
        let vrl = ValidatorRateLimiter::new(2, Duration::from_millis(100));
        let validator1 = BlsPublicKey::from([1u8; 48]);
        let validator2 = BlsPublicKey::from([2u8; 48]);
        let validator3 = BlsPublicKey::from([3u8; 48]);

        // Create some rate limiters by making requests
        assert!(vrl.check_rate_limit(&validator1).await);
        assert!(vrl.check_rate_limit(&validator2).await);
        assert!(vrl.check_rate_limit(&validator3).await);

        // Initial cleanup should remove nothing
        assert_eq!(vrl.cleanup_old_limiters().await, 0);

        // Access validator1 and validator2 to update their last access time
        assert!(vrl.check_rate_limit(&validator1).await);
        assert!(vrl.check_rate_limit(&validator2).await); // Second request for validator2
        assert!(!vrl.check_rate_limit(&validator2).await); // This updates last access even if limit exceeded

        // Manually create an old limiter that should be cleaned up
        {
            let mut limiters = vrl.limiters.write().await;
            let old_time = Instant::now() - Duration::from_secs(7200); // 2 hours ago
            limiters.insert(
                BlsPublicKey::from([4u8; 48]),
                (RateLimiter::new(2, Duration::from_millis(100)), old_time)
            );
        }

        // Cleanup should remove the old limiter
        assert_eq!(vrl.cleanup_old_limiters().await, 1);

        // Verify the old limiter was removed
        assert_eq!(vrl.current_count(&BlsPublicKey::from([4u8; 48])).await, 0);
        
        // Verify other limiters still exist
        assert!(vrl.current_count(&validator1).await > 0);
        assert!(vrl.current_count(&validator2).await > 0);
        assert!(vrl.current_count(&validator3).await > 0);
    }

    #[tokio::test]
    async fn test_cleanup_old_limiters_retains_recent() {
        let vrl = ValidatorRateLimiter::new(5, Duration::from_secs(1));
        let validator = BlsPublicKey::from([1u8; 48]);

        // Make a request
        assert!(vrl.check_rate_limit(&validator).await);

        // Cleanup immediately - should not remove anything
        assert_eq!(vrl.cleanup_old_limiters().await, 0);

        // Verify limiter still exists
        assert!(vrl.current_count(&validator).await > 0);
    }

    #[tokio::test]
    async fn test_cleanup_returns_correct_count() {
        let vrl = ValidatorRateLimiter::new(1, Duration::from_millis(50));

        // Create multiple old limiters
        {
            let mut limiters = vrl.limiters.write().await;
            let old_time = Instant::now() - Duration::from_secs(7200); // 2 hours ago
            
            for i in 0..5 {
                limiters.insert(
                    BlsPublicKey::from([i; 48]),
                    (RateLimiter::new(1, Duration::from_millis(50)), old_time)
                );
            }
        }

        // Create some recent limiters
        for i in 10..13 {
            let validator = BlsPublicKey::from([i; 48]);
            assert!(vrl.check_rate_limit(&validator).await);
        }

        // Cleanup should remove exactly 5 old limiters
        assert_eq!(vrl.cleanup_old_limiters().await, 5);

        // Verify correct limiters remain
        for i in 10..13 {
            assert!(vrl.current_count(&BlsPublicKey::from([i; 48])).await > 0);
        }
    }
}