use alloy::{primitives::Address, rpc::types::beacon::BlsPublicKey};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::debug;

const REGISTRATION_TTL: Duration = Duration::from_secs(27 * 60 * 60); // 27 hours

/// Cache to track recently registered validators
pub struct RegistrationCache {
    /// Map of validator pubkey to registration data and timestamp
    registrations: HashMap<BlsPublicKey, RegistrationEntry>,
    /// Cache metrics
    hits: u64,
    misses: u64,
}

#[derive(Clone)]
struct RegistrationEntry {
    fee_recipient: Address,
    gas_limit: u64,
    registered_at: Instant,
}

impl RegistrationCache {
    pub fn new() -> Self {
        Self { registrations: HashMap::new(), hits: 0, misses: 0 }
    }

    /// Check if validator needs re-registration
    pub fn needs_registration(
        &mut self,
        pubkey: &BlsPublicKey,
        fee_recipient: &Address,
        gas_limit: u64,
    ) -> bool {
        match self.registrations.get(pubkey) {
            Some(entry) => {
                // Check if registration is still valid
                let expired = entry.registered_at.elapsed() > REGISTRATION_TTL;
                let params_changed =
                    entry.fee_recipient != *fee_recipient || entry.gas_limit != gas_limit;

                if expired || params_changed {
                    self.misses += 1;
                    if expired {
                        debug!(pubkey = %pubkey, "Registration expired, needs re-registration");
                    } else {
                        debug!(
                            pubkey = %pubkey,
                            old_fee_recipient = %entry.fee_recipient,
                            new_fee_recipient = %fee_recipient,
                            old_gas_limit = entry.gas_limit,
                            new_gas_limit = gas_limit,
                            "Registration params changed, needs re-registration"
                        );
                    }
                    true
                } else {
                    self.hits += 1;
                    false
                }
            }
            None => {
                self.misses += 1;
                debug!(pubkey = %pubkey, "First time registration");
                true
            }
        }
    }

    /// Mark validator as registered
    pub fn mark_registered(
        &mut self,
        pubkey: BlsPublicKey,
        fee_recipient: Address,
        gas_limit: u64,
    ) {
        self.registrations.insert(
            pubkey,
            RegistrationEntry { fee_recipient, gas_limit, registered_at: Instant::now() },
        );

        // Periodically clean expired entries to prevent unbounded growth
        if self.registrations.len() % 1000 == 0 {
            self.cleanup_expired();
        }
    }

    /// Remove expired entries
    fn cleanup_expired(&mut self) {
        let initial_size = self.registrations.len();
        self.registrations.retain(|_, entry| entry.registered_at.elapsed() <= REGISTRATION_TTL);
        let removed = initial_size - self.registrations.len();
        if removed > 0 {
            debug!(
                removed_entries = removed,
                remaining_entries = self.registrations.len(),
                "Cleaned up expired registration cache entries"
            );
        }
    }

    /// Get cache statistics
    pub fn stats(&self) -> (u64, u64, usize) {
        (self.hits, self.misses, self.registrations.len())
    }

    /// Force cleanup for testing
    #[cfg(test)]
    pub fn force_cleanup(&mut self) {
        self.cleanup_expired();
    }

    /// Get hit rate percentage
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            (self.hits as f64 / total as f64) * 100.0
        }
    }
}

// Global cache instance
lazy_static::lazy_static! {
    pub static ref REGISTRATION_CACHE: Arc<RwLock<RegistrationCache>> =
        Arc::new(RwLock::new(RegistrationCache::new()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::Address;
    use std::thread;

    #[test]
    fn test_registration_cache_ttl() {
        let mut cache = RegistrationCache::new();
        let pubkey = BlsPublicKey::from([1u8; 48]);
        let fee_recipient = Address::from([2u8; 20]);

        // First registration should need registration
        assert!(cache.needs_registration(&pubkey, &fee_recipient, 30_000_000));

        // Mark as registered
        cache.mark_registered(pubkey, fee_recipient, 30_000_000);

        // Should not need registration immediately
        assert!(!cache.needs_registration(&pubkey, &fee_recipient, 30_000_000));

        // Verify cache hit was recorded
        assert_eq!(cache.hits, 1);
        assert_eq!(cache.misses, 1);
        assert_eq!(cache.hit_rate(), 50.0);
    }

    #[test]
    fn test_params_change_triggers_reregistration() {
        let mut cache = RegistrationCache::new();
        let pubkey = BlsPublicKey::from([1u8; 48]);
        let fee_recipient1 = Address::from([2u8; 20]);
        let fee_recipient2 = Address::from([3u8; 20]);

        cache.mark_registered(pubkey, fee_recipient1, 30_000_000);

        // Different fee recipient should trigger re-registration
        assert!(cache.needs_registration(&pubkey, &fee_recipient2, 30_000_000));

        // Different gas limit should trigger re-registration
        assert!(cache.needs_registration(&pubkey, &fee_recipient1, 25_000_000));

        // Cache should have 2 misses (initial + param change)
        assert_eq!(cache.misses, 2);
    }

    #[test]
    fn test_cleanup_expired() {
        let mut cache = RegistrationCache::new();

        // Add entries
        for i in 0..10 {
            let pubkey = BlsPublicKey::from([i; 48]);
            cache.mark_registered(pubkey, Address::default(), 30_000_000);
        }

        assert_eq!(cache.registrations.len(), 10);

        // Cleanup should not remove fresh entries
        cache.force_cleanup();
        assert_eq!(cache.registrations.len(), 10);
    }

    #[test]
    fn test_concurrent_access() {
        use std::sync::Arc;

        let cache = Arc::new(RwLock::new(RegistrationCache::new()));
        let mut handles = vec![];

        // Spawn multiple threads accessing cache
        for i in 0..10 {
            let cache_clone = cache.clone();
            let handle = thread::spawn(move || {
                let pubkey = BlsPublicKey::from([i; 48]);
                let fee_recipient = Address::from([i; 20]);

                let mut cache = cache_clone.write();
                cache.needs_registration(&pubkey, &fee_recipient, 30_000_000);
                cache.mark_registered(pubkey, fee_recipient, 30_000_000);
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.join().unwrap();
        }

        let cache = cache.read();
        assert_eq!(cache.registrations.len(), 10);
    }

    #[test]
    fn test_hit_rate_calculation() {
        let mut cache = RegistrationCache::new();

        // No requests yet
        assert_eq!(cache.hit_rate(), 0.0);

        let pubkey = BlsPublicKey::from([1u8; 48]);
        let fee_recipient = Address::from([2u8; 20]);

        // First call is a miss
        cache.needs_registration(&pubkey, &fee_recipient, 30_000_000);
        cache.mark_registered(pubkey, fee_recipient, 30_000_000);
        assert_eq!(cache.hit_rate(), 0.0); // 0 hits, 1 miss

        // Second call is a hit
        cache.needs_registration(&pubkey, &fee_recipient, 30_000_000);
        assert_eq!(cache.hit_rate(), 50.0); // 1 hit, 1 miss

        // Third call is another hit
        cache.needs_registration(&pubkey, &fee_recipient, 30_000_000);
        assert!((cache.hit_rate() - 66.67).abs() < 0.1); // 2 hits, 1 miss ≈ 66.67%
    }

    #[test]
    fn test_stats() {
        let mut cache = RegistrationCache::new();
        let pubkey = BlsPublicKey::from([1u8; 48]);
        let fee_recipient = Address::from([2u8; 20]);

        // Initial stats
        let (hits, misses, size) = cache.stats();
        assert_eq!((hits, misses, size), (0, 0, 0));

        // After registration
        cache.needs_registration(&pubkey, &fee_recipient, 30_000_000);
        cache.mark_registered(pubkey, fee_recipient, 30_000_000);

        let (hits, misses, size) = cache.stats();
        assert_eq!((hits, misses, size), (0, 1, 1));

        // After cache hit
        cache.needs_registration(&pubkey, &fee_recipient, 30_000_000);

        let (hits, misses, size) = cache.stats();
        assert_eq!((hits, misses, size), (1, 1, 1));
    }
}
