#![allow(dead_code)]

use dashmap::DashMap;
use parking_lot::RwLock;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, warn};

/// Cache entry with value, ETag, and expiration time
#[derive(Clone, Debug)]
pub struct CacheEntry<T> {
    pub value: T,
    pub etag: Option<String>,
    pub expires_at: Instant,
    pub last_accessed: Instant,
}

impl<T> CacheEntry<T> {
    fn new(value: T, etag: Option<String>, ttl: Duration) -> Self {
        let now = Instant::now();
        Self {
            value,
            etag,
            expires_at: now + ttl,
            last_accessed: now,
        }
    }

    fn is_expired(&self) -> bool {
        Instant::now() > self.expires_at
    }

    fn touch(&mut self) {
        self.last_accessed = Instant::now();
    }
}

/// LRU Cache with TTL and ETag support
pub struct LruCache<T: Clone> {
    cache: Arc<DashMap<String, CacheEntry<T>>>,
    lru_order: Arc<RwLock<VecDeque<String>>>,
    max_size: usize,
    default_ttl: Duration,
    hits: AtomicU64,
    misses: AtomicU64,
}

impl<T: Clone> LruCache<T> {
    pub fn new(max_size: usize, default_ttl: Duration) -> Self {
        Self {
            cache: Arc::new(DashMap::new()),
            lru_order: Arc::new(RwLock::new(VecDeque::with_capacity(max_size))),
            max_size,
            default_ttl,
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
        }
    }

    /// Get a value from the cache
    pub fn get(&self, key: &str) -> Option<(T, Option<String>)> {
        let mut entry = self.cache.get_mut(key)?;
        
        if entry.is_expired() {
            drop(entry);
            self.remove(key);
            self.misses.fetch_add(1, Ordering::Relaxed);
            return None;
        }

        entry.touch();
        let value = entry.value.clone();
        let etag = entry.etag.clone();
        drop(entry);

        // Update LRU order
        self.update_lru_order(key);
        self.hits.fetch_add(1, Ordering::Relaxed);
        
        Some((value, etag))
    }

    /// Get only the ETag for a key
    pub fn get_etag(&self, key: &str) -> Option<String> {
        let entry = self.cache.get(key)?;
        
        if entry.is_expired() {
            drop(entry);
            self.remove(key);
            return None;
        }

        entry.etag.clone()
    }

    /// Insert a value into the cache
    pub fn insert(&self, key: String, value: T, etag: Option<String>) {
        self.insert_with_ttl(key, value, etag, self.default_ttl);
    }

    /// Insert a value with custom TTL
    pub fn insert_with_ttl(&self, key: String, value: T, etag: Option<String>, ttl: Duration) {
        let entry = CacheEntry::new(value, etag, ttl);
        
        // If we're at capacity, evict the least recently used item
        if self.cache.len() >= self.max_size {
            self.evict_lru();
        }

        self.cache.insert(key.clone(), entry);
        self.update_lru_order(&key);
        
        debug!(key = %key, ttl_secs = ttl.as_secs(), "Cached item");
    }

    /// Remove a specific key from the cache
    pub fn remove(&self, key: &str) -> Option<T> {
        let entry = self.cache.remove(key)?;
        
        // Remove from LRU order
        let mut order = self.lru_order.write();
        order.retain(|k| k != key);
        
        Some(entry.1.value)
    }

    /// Clear all expired entries
    pub fn clear_expired(&self) {
        let keys_to_remove: Vec<String> = self
            .cache
            .iter()
            .filter(|entry| entry.value().is_expired())
            .map(|entry| entry.key().clone())
            .collect();

        for key in keys_to_remove {
            self.remove(&key);
        }
    }

    /// Get cache statistics
    pub fn stats(&self) -> CacheStats {
        CacheStats {
            size: self.cache.len(),
            capacity: self.max_size,
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
        }
    }

    fn update_lru_order(&self, key: &str) {
        let mut order = self.lru_order.write();
        order.retain(|k| k != key);
        order.push_back(key.to_string());
    }

    fn evict_lru(&self) {
        let mut order = self.lru_order.write();
        if let Some(oldest_key) = order.pop_front() {
            self.cache.remove(&oldest_key);
            debug!(key = %oldest_key, "Evicted LRU item");
        }
    }
}

#[derive(Debug, Clone)]
pub struct CacheStats {
    pub size: usize,
    pub capacity: usize,
    pub hits: u64,
    pub misses: u64,
}

impl CacheStats {
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            self.hits as f64 / total as f64
        }
    }
}

/// Circuit breaker states
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CircuitState {
    Closed,
    Open,
    HalfOpen,
}

/// Circuit breaker for handling relay failures
pub struct CircuitBreaker {
    state: Arc<RwLock<CircuitState>>,
    failure_count: AtomicUsize,
    last_failure_time: Arc<RwLock<Option<Instant>>>,
    failure_threshold: usize,
    recovery_timeout: Duration,
    half_open_success_threshold: usize,
    half_open_attempts: AtomicUsize,
}

impl CircuitBreaker {
    pub fn new(
        failure_threshold: usize,
        recovery_timeout: Duration,
        half_open_success_threshold: usize,
    ) -> Self {
        Self {
            state: Arc::new(RwLock::new(CircuitState::Closed)),
            failure_count: AtomicUsize::new(0),
            last_failure_time: Arc::new(RwLock::new(None)),
            failure_threshold,
            recovery_timeout,
            half_open_success_threshold,
            half_open_attempts: AtomicUsize::new(0),
        }
    }

    /// Check if the circuit breaker allows a request
    pub fn can_proceed(&self) -> bool {
        let current_state = *self.state.read();
        
        match current_state {
            CircuitState::Closed => true,
            CircuitState::Open => {
                // Check if we should transition to half-open
                let last_failure = *self.last_failure_time.read();
                if let Some(failure_time) = last_failure {
                    if Instant::now() > failure_time + self.recovery_timeout {
                        let _ = last_failure;
                        *self.state.write() = CircuitState::HalfOpen;
                        self.half_open_attempts.store(0, Ordering::Relaxed);
                        debug!("Circuit breaker transitioning to half-open");
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            CircuitState::HalfOpen => {
                // Allow limited requests in half-open state
                self.half_open_attempts.fetch_add(1, Ordering::Relaxed) < self.half_open_success_threshold
            }
        }
    }

    /// Record a successful request
    pub fn record_success(&self) {
        let current_state = *self.state.read();
        
        match current_state {
            CircuitState::HalfOpen => {
                let attempts = self.half_open_attempts.load(Ordering::Relaxed);
                if attempts >= self.half_open_success_threshold {
                    let _ = current_state;
                    *self.state.write() = CircuitState::Closed;
                    self.failure_count.store(0, Ordering::Relaxed);
                    self.half_open_attempts.store(0, Ordering::Relaxed);
                    debug!("Circuit breaker closed after successful recovery");
                }
            }
            CircuitState::Closed => {
                // Reset failure count on success
                self.failure_count.store(0, Ordering::Relaxed);
            }
            _ => {}
        }
    }

    /// Record a failed request
    pub fn record_failure(&self) {
        let failures = self.failure_count.fetch_add(1, Ordering::Relaxed) + 1;
        *self.last_failure_time.write() = Some(Instant::now());
        
        let current_state = *self.state.read();
        
        match current_state {
            CircuitState::Closed => {
                if failures >= self.failure_threshold {
                    let _ = current_state;
                    *self.state.write() = CircuitState::Open;
                    warn!(
                        failures = failures,
                        threshold = self.failure_threshold,
                        "Circuit breaker opened due to excessive failures"
                    );
                }
            }
            CircuitState::HalfOpen => {
                let _ = current_state;
                *self.state.write() = CircuitState::Open;
                self.half_open_attempts.store(0, Ordering::Relaxed);
                warn!("Circuit breaker reopened due to failure in half-open state");
            }
            _ => {}
        }
    }

    /// Get the current state of the circuit breaker
    pub fn state(&self) -> CircuitState {
        *self.state.read()
    }

    /// Reset the circuit breaker
    pub fn reset(&self) {
        *self.state.write() = CircuitState::Closed;
        self.failure_count.store(0, Ordering::Relaxed);
        *self.last_failure_time.write() = None;
        self.half_open_attempts.store(0, Ordering::Relaxed);
    }
}

/// Exponential backoff calculator
pub struct ExponentialBackoff {
    base_delay: Duration,
    max_delay: Duration,
    multiplier: f64,
}

impl ExponentialBackoff {
    pub fn new(base_delay: Duration, max_delay: Duration, multiplier: f64) -> Self {
        Self {
            base_delay,
            max_delay,
            multiplier,
        }
    }

    /// Calculate delay for the given retry attempt (0-indexed)
    pub fn calculate_delay(&self, attempt: u32) -> Duration {
        let delay_ms = self.base_delay.as_millis() as f64 * self.multiplier.powi(attempt as i32);
        let delay_ms = delay_ms.min(self.max_delay.as_millis() as f64) as u64;
        Duration::from_millis(delay_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn test_lru_cache_basic() {
        let cache: LruCache<String> = LruCache::new(2, Duration::from_secs(60));
        
        cache.insert("key1".to_string(), "value1".to_string(), None);
        cache.insert("key2".to_string(), "value2".to_string(), Some("etag1".to_string()));
        
        let (value, etag) = cache.get("key1").unwrap();
        assert_eq!(value, "value1");
        assert!(etag.is_none());
        
        let (value, etag) = cache.get("key2").unwrap();
        assert_eq!(value, "value2");
        assert_eq!(etag, Some("etag1".to_string()));
    }

    #[test]
    fn test_lru_eviction() {
        let cache: LruCache<String> = LruCache::new(2, Duration::from_secs(60));
        
        cache.insert("key1".to_string(), "value1".to_string(), None);
        cache.insert("key2".to_string(), "value2".to_string(), None);
        cache.insert("key3".to_string(), "value3".to_string(), None);
        
        // key1 should be evicted
        assert!(cache.get("key1").is_none());
        assert!(cache.get("key2").is_some());
        assert!(cache.get("key3").is_some());
    }

    #[test]
    fn test_cache_expiration() {
        let cache: LruCache<String> = LruCache::new(10, Duration::from_millis(100));
        
        cache.insert("key1".to_string(), "value1".to_string(), None);
        thread::sleep(Duration::from_millis(150));
        
        assert!(cache.get("key1").is_none());
    }

    #[test]
    fn test_circuit_breaker() {
        let breaker = CircuitBreaker::new(3, Duration::from_millis(100), 2);
        
        assert_eq!(breaker.state(), CircuitState::Closed);
        assert!(breaker.can_proceed());
        
        // Record failures
        for _ in 0..3 {
            breaker.record_failure();
        }
        
        assert_eq!(breaker.state(), CircuitState::Open);
        assert!(!breaker.can_proceed());
        
        // Wait for recovery timeout
        thread::sleep(Duration::from_millis(150));
        assert!(breaker.can_proceed());
        assert_eq!(breaker.state(), CircuitState::HalfOpen);
        
        // Need to make successful attempts in half-open state
        // Each can_proceed() call in half-open state counts as an attempt
        assert!(breaker.can_proceed()); // First attempt
        breaker.record_success();
        
        assert!(breaker.can_proceed()); // Second attempt
        breaker.record_success();
        
        // After 2 successful attempts, circuit should be closed
        assert_eq!(breaker.state(), CircuitState::Closed);
    }

    #[test]
    fn test_exponential_backoff() {
        let backoff = ExponentialBackoff::new(
            Duration::from_millis(100),
            Duration::from_secs(10),
            2.0,
        );
        
        assert_eq!(backoff.calculate_delay(0), Duration::from_millis(100));
        assert_eq!(backoff.calculate_delay(1), Duration::from_millis(200));
        assert_eq!(backoff.calculate_delay(2), Duration::from_millis(400));
        
        // Should be capped at max_delay
        assert_eq!(backoff.calculate_delay(10), Duration::from_secs(10));
    }
}