use std::sync::Arc;
use std::time::{Duration, Instant};
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
