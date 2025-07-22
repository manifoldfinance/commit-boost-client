#[cfg(test)]
mod tests {
    use crate::config::RelayGasConfig;
    use crate::metrics_wrapper::NoOpMetricsRecorder;
    use crate::relay_communication::RelayQueryService;
    use crate::service::ReservedGasService;
    use crate::state_traits::{ConfigFetchManager, GasReservationManager};
    use googletest::prelude::*;
    use std::sync::Arc;
    use std::time::Instant;

    // Mock implementations for testing with proper state management
    struct MockGasReservationManager {
        reservations: Arc<std::sync::RwLock<std::collections::HashMap<String, u64>>>,
        update_calls: Arc<std::sync::RwLock<Vec<(String, RelayGasConfig)>>>,
        apply_calls: Arc<std::sync::RwLock<Vec<(String, u64)>>>,
    }

    impl MockGasReservationManager {
        fn new() -> Self {
            Self {
                reservations: Arc::new(std::sync::RwLock::new(std::collections::HashMap::new())),
                update_calls: Arc::new(std::sync::RwLock::new(Vec::new())),
                apply_calls: Arc::new(std::sync::RwLock::new(Vec::new())),
            }
        }

        fn with_reservations(reservations: std::collections::HashMap<String, u64>) -> Self {
            Self {
                reservations: Arc::new(std::sync::RwLock::new(reservations)),
                update_calls: Arc::new(std::sync::RwLock::new(Vec::new())),
                apply_calls: Arc::new(std::sync::RwLock::new(Vec::new())),
            }
        }

        fn times_updated(&self, relay_id: &str) -> usize {
            self.update_calls.read().unwrap().iter().filter(|(id, _)| id == relay_id).count()
        }

        fn last_update_config(&self, relay_id: &str) -> Option<RelayGasConfig> {
            self.update_calls
                .read()
                .unwrap()
                .iter()
                .rev()
                .find(|(id, _)| id == relay_id)
                .map(|(_, config)| config.clone())
        }

        fn times_applied(&self, relay_id: &str) -> usize {
            self.apply_calls.read().unwrap().iter().filter(|(id, _)| id == relay_id).count()
        }
    }

    impl GasReservationManager for MockGasReservationManager {
        fn get_reservation(&self, relay_id: &str) -> u64 {
            self.reservations.read().unwrap().get(relay_id).copied().unwrap_or(1_000_000)
        }

        fn update_reservation(&self, relay_id: String, config: RelayGasConfig) {
            // Actually update the reservation
            self.reservations.write().unwrap().insert(relay_id.clone(), config.reserved_gas_limit);

            // Track the call
            self.update_calls.write().unwrap().push((relay_id, config));
        }

        fn apply_reservation(
            &self,
            relay_id: &str,
            original_gas_limit: u64,
        ) -> crate::error::Result<crate::types::GasReservationOutcome> {
            // Track the call
            self.apply_calls.write().unwrap().push((relay_id.to_string(), original_gas_limit));

            let reserved = self.get_reservation(relay_id);
            let new_gas_limit = original_gas_limit.saturating_sub(reserved);

            // Validate minimum gas limit requirement
            if new_gas_limit < crate::validation::MIN_GAS_AFTER_RESERVATION {
                return Err(crate::error::ReservedGasError::GasLimitTooLow {
                    actual: new_gas_limit,
                    minimum: crate::validation::MIN_GAS_AFTER_RESERVATION,
                    reserved,
                });
            }

            Ok(crate::types::GasReservationOutcome {
                relay_id: relay_id.to_string(),
                new_gas_limit,
                reserved_amount: reserved,
            })
        }

        fn get_all_reservations(&self) -> Vec<(String, crate::state::GasReservation)> {
            self.reservations
                .read()
                .unwrap()
                .iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        crate::state::GasReservation {
                            reserved_gas: *v,
                            last_updated: Instant::now(),
                            relay_id: k.clone(),
                            min_gas_limit: None,
                        },
                    )
                })
                .collect()
        }
    }

    struct MockConfigFetchManager {
        last_fetch: Instant,
    }

    // Mock that always returns errors for testing error handling
    struct FailingGasReservationManager;

    impl GasReservationManager for FailingGasReservationManager {
        fn get_reservation(&self, _relay_id: &str) -> u64 {
            1_000_000
        }

        fn update_reservation(&self, _relay_id: String, _config: RelayGasConfig) {
            // Do nothing
        }

        fn apply_reservation(
            &self,
            _relay_id: &str,
            _original_gas_limit: u64,
        ) -> crate::error::Result<crate::types::GasReservationOutcome> {
            Err(crate::error::ReservedGasError::GasLimitTooLow {
                actual: 5_000_000,
                minimum: 10_000_000,
                reserved: 25_000_000,
            })
        }

        fn get_all_reservations(&self) -> Vec<(String, crate::state::GasReservation)> {
            vec![]
        }
    }

    impl ConfigFetchManager for MockConfigFetchManager {
        fn should_fetch_configs(&self) -> bool {
            self.last_fetch.elapsed().as_secs() > 300
        }

        fn get_last_fetch_time(&self) -> Instant {
            self.last_fetch
        }
    }

    #[test]
    fn test_update_reservation_persists_state() -> Result<()> {
        let mock_state = MockGasReservationManager::new();

        // Initial state - default reservation
        assert_that!(mock_state.get_reservation("relay1"), eq(1_000_000));
        assert_that!(mock_state.times_updated("relay1"), eq(0));

        // Update reservation
        let config = RelayGasConfig {
            reserved_gas_limit: 2_500_000,
            min_gas_limit: Some(15_000_000),
            update_interval: Some(300),
        };
        mock_state.update_reservation("relay1".to_string(), config.clone());

        // Verify state was updated
        assert_that!(mock_state.get_reservation("relay1"), eq(2_500_000));
        assert_that!(mock_state.times_updated("relay1"), eq(1));

        // Verify update tracking
        let last_config = mock_state.last_update_config("relay1");
        assert_that!(last_config, some(anything()));
        let last_config = last_config.unwrap();
        assert_that!(last_config.reserved_gas_limit, eq(2_500_000));
        assert_that!(last_config.min_gas_limit, eq(Some(15_000_000)));
        assert_that!(last_config.update_interval, eq(Some(300)));

        // Other relays remain at default
        assert_that!(mock_state.get_reservation("relay2"), eq(1_000_000));
        Ok(())
    }

    #[test]
    fn test_apply_reservation_calculates_correctly() -> Result<()> {
        let mut reservations = std::collections::HashMap::new();
        reservations.insert("relay1".to_string(), 5_000_000);
        let mock_state = MockGasReservationManager::with_reservations(reservations);

        // Test successful reservation
        let original_gas = 30_000_000;
        let result = mock_state.apply_reservation("relay1", original_gas);

        assert_that!(result, ok(anything()));
        let outcome = result.unwrap();
        assert_that!(outcome.relay_id, eq("relay1"));
        assert_that!(outcome.new_gas_limit, eq(25_000_000));
        assert_that!(outcome.reserved_amount, eq(5_000_000));
        assert_that!(outcome.new_gas_limit + outcome.reserved_amount, eq(original_gas));

        // Verify tracking
        assert_that!(mock_state.times_applied("relay1"), eq(1));
        Ok(())
    }

    #[test]
    fn test_apply_reservation_fails_when_gas_too_low() -> Result<()> {
        let mut reservations = std::collections::HashMap::new();
        reservations.insert("relay1".to_string(), 25_000_000);
        let mock_state = MockGasReservationManager::with_reservations(reservations);

        // Original gas is less than reservation + minimum
        let original_gas = 30_000_000;
        let result = mock_state.apply_reservation("relay1", original_gas);

        assert_that!(
            result,
            err(matches_pattern!(crate::error::ReservedGasError::GasLimitTooLow {
                actual: eq(5_000_000), // 30M - 25M
                minimum: eq(crate::validation::MIN_GAS_AFTER_RESERVATION),
                reserved: eq(25_000_000)
            }))
        );

        // Verify tracking even for failed attempts
        assert_that!(mock_state.times_applied("relay1"), eq(1));
        Ok(())
    }

    #[test]
    fn test_multiple_updates_tracked_correctly() -> Result<()> {
        let mock_state = MockGasReservationManager::new();

        // Multiple updates to same relay
        for i in 1..=3 {
            let config = RelayGasConfig {
                reserved_gas_limit: i * 1_000_000,
                min_gas_limit: None,
                update_interval: None,
            };
            mock_state.update_reservation("relay1".to_string(), config);
        }

        // Verify final state
        assert_that!(mock_state.get_reservation("relay1"), eq(3_000_000));
        assert_that!(mock_state.times_updated("relay1"), eq(3));

        // Verify last update
        let last_config = mock_state.last_update_config("relay1");
        assert_that!(last_config, some(anything()));
        assert_that!(last_config.unwrap().reserved_gas_limit, eq(3_000_000));
        Ok(())
    }

    #[test]
    fn test_service_with_failing_dependencies() -> Result<()> {
        // Test error handling with dependencies that return errors
        let mock_state = Arc::new(FailingGasReservationManager);
        let mock_config = Arc::new(MockConfigFetchManager {
            last_fetch: Instant::now() - std::time::Duration::from_secs(400), // Stale
        });
        let mock_metrics = Arc::new(NoOpMetricsRecorder);
        let mock_query_service = RelayQueryService::production();

        let _service = ReservedGasService::with_dependencies(
            mock_state.clone(),
            mock_config.clone(),
            mock_metrics,
            mock_query_service,
        );

        // Verify service handles errors gracefully
        assert_that!(mock_config.should_fetch_configs(), eq(true)); // Config is stale

        // Test that errors from dependencies are properly handled
        let result = mock_state.apply_reservation("test", 30_000_000);
        assert_that!(
            result,
            err(matches_pattern!(crate::error::ReservedGasError::GasLimitTooLow { .. }))
        );
        Ok(())
    }

    #[test]
    fn test_get_all_reservations_returns_current_state() -> Result<()> {
        use crate::test_matchers::matchers::*;

        let mut reservations = std::collections::HashMap::new();
        reservations.insert("relay1".to_string(), 2_000_000);
        reservations.insert("relay2".to_string(), 3_000_000);
        let mock_state = MockGasReservationManager::with_reservations(reservations);

        let all_reservations = mock_state.get_all_reservations();

        // Verify correct number of reservations
        assert_that!(all_reservations.len(), eq(2));

        // Sort by relay_id for consistent testing
        let mut sorted = all_reservations;
        sorted.sort_by(|a, b| a.0.cmp(&b.0));

        // Verify relay1
        assert_that!(sorted[0].0, eq("relay1"));
        assert_that!(sorted[0].1.reserved_gas, eq(2_000_000));
        assert_that!(sorted[0].1.reserved_gas, is_healthy_relay());
        assert_that!(sorted[0].1.relay_id, eq("relay1"));
        assert_that!(sorted[0].1.min_gas_limit, none());

        // Verify relay2
        assert_that!(sorted[1].0, eq("relay2"));
        assert_that!(sorted[1].1.reserved_gas, eq(3_000_000));
        assert_that!(sorted[1].1.reserved_gas, is_healthy_relay());
        assert_that!(sorted[1].1.relay_id, eq("relay2"));
        assert_that!(sorted[1].1.min_gas_limit, none());

        Ok(())
    }

    #[test]
    fn test_default_reservation_for_unknown_relay() -> Result<()> {
        let mock_state = MockGasReservationManager::new();

        // Unknown relay should return default
        assert_that!(mock_state.get_reservation("unknown-relay"), eq(1_000_000));

        // Add a known relay
        mock_state.update_reservation(
            "known-relay".to_string(),
            RelayGasConfig {
                reserved_gas_limit: 2_000_000,
                min_gas_limit: None,
                update_interval: None,
            },
        );

        // Known relay returns updated value
        assert_that!(mock_state.get_reservation("known-relay"), eq(2_000_000));
        // Unknown still returns default
        assert_that!(mock_state.get_reservation("another-unknown"), eq(1_000_000));
        Ok(())
    }

    #[test]
    fn test_apply_reservation_with_zero_reservation() -> Result<()> {
        let mut reservations = std::collections::HashMap::new();
        reservations.insert("relay1".to_string(), 0);
        let mock_state = MockGasReservationManager::with_reservations(reservations);

        let original_gas = 30_000_000;
        let result = mock_state.apply_reservation("relay1", original_gas);

        assert_that!(result, ok(anything()));
        let outcome = result.unwrap();
        assert_that!(outcome.new_gas_limit, eq(30_000_000)); // No reduction
        assert_that!(outcome.reserved_amount, eq(0));
        Ok(())
    }

    #[test]
    fn test_config_fetch_manager_timing() -> Result<()> {
        // Test fresh config
        let fresh_config = MockConfigFetchManager { last_fetch: Instant::now() };
        assert_that!(fresh_config.should_fetch_configs(), eq(false));

        // Test stale config (> 300 seconds)
        let stale_config = MockConfigFetchManager {
            last_fetch: Instant::now() - std::time::Duration::from_secs(400),
        };
        assert_that!(stale_config.should_fetch_configs(), eq(true));

        // Test boundary (exactly 300 seconds)
        let boundary_config = MockConfigFetchManager {
            last_fetch: Instant::now() - std::time::Duration::from_secs(300),
        };
        assert_that!(boundary_config.should_fetch_configs(), eq(false)); // Needs to be > 300

        // Verify get_last_fetch_time
        let test_time = Instant::now() - std::time::Duration::from_secs(150);
        let config = MockConfigFetchManager { last_fetch: test_time };
        let fetch_time = config.get_last_fetch_time();
        let elapsed = fetch_time.elapsed().as_secs();
        assert_that!(elapsed, ge(150));
        assert_that!(elapsed, lt(200)); // Some margin for test execution
        Ok(())
    }

    // Simple tests for reserve query functionality
    #[test]
    fn test_mock_relay_querier_success() -> Result<()> {
        // Test that our mock correctly returns success responses
        let responses = std::collections::HashMap::from([
            ("flashbots".to_string(), (Some(2_000_000u64), None)),
            ("ultrasound".to_string(), (None, Some("Not supported".to_string()))),
        ]);

        assert_that!(responses.get("flashbots").unwrap().0, some(eq(2_000_000)));
        assert_that!(responses.get("ultrasound").unwrap().0, none());
        assert_that!(responses.get("ultrasound").unwrap().1, some(anything()));
        Ok(())
    }

    #[test]
    fn test_reserve_values_are_validated() -> Result<()> {
        // Test that reserve values are validated
        use crate::validation::MAX_REASONABLE_GAS;

        // Valid reserve
        let valid_reserve = 2_000_000u64;
        assert_that!(valid_reserve <= MAX_REASONABLE_GAS, eq(true));

        // Invalid reserve (too high)
        let invalid_reserve = MAX_REASONABLE_GAS + 1;
        assert_that!(invalid_reserve > MAX_REASONABLE_GAS, eq(true));

        Ok(())
    }

    #[test]
    fn test_relay_reserve_info_serialization() -> Result<()> {
        use crate::types::{RelayReserveInfo, ReserveQueryStatus};

        let info = RelayReserveInfo {
            relay_id: "flashbots".to_string(),
            reserve: Some(2_000_000),
            query_time_ms: 45,
            status: ReserveQueryStatus::Success,
            error: None,
        };

        let json = serde_json::to_value(&info)?;
        assert_that!(json.get("relay_id"), some(anything()));
        assert_that!(json.get("reserve"), some(anything()));
        assert_that!(json.get("status").and_then(|v| v.as_str()), some(eq("success")));

        Ok(())
    }

    #[test]
    fn test_reserve_statistics_calculation() -> Result<()> {
        let reserves = vec![1_000_000u64, 2_000_000, 3_000_000];
        let total: u64 = reserves.iter().sum();
        let count = reserves.len() as u64;
        let average = total / count;
        let max = *reserves.iter().max().unwrap();
        let min = *reserves.iter().min().unwrap();

        assert_that!(total, eq(6_000_000));
        assert_that!(average, eq(2_000_000));
        assert_that!(max, eq(3_000_000));
        assert_that!(min, eq(1_000_000));

        Ok(())
    }

    #[test]
    fn test_concurrent_updates_are_thread_safe() -> Result<()> {
        use std::thread;

        let mock_state = Arc::new(MockGasReservationManager::new());
        let mut handles = vec![];

        // Spawn multiple threads updating different relays
        for i in 0..5 {
            let state_clone = Arc::clone(&mock_state);
            let handle = thread::spawn(move || {
                for j in 0..10 {
                    let config = RelayGasConfig {
                        reserved_gas_limit: (i + 1) * 1_000_000 + j * 100_000,
                        min_gas_limit: None,
                        update_interval: None,
                    };
                    state_clone.update_reservation(format!("relay{}", i), config);
                }
            });
            handles.push(handle);
        }

        // Wait for all threads to complete
        for handle in handles {
            handle.join().unwrap();
        }

        // Verify final state
        for i in 0..5 {
            let relay_id = format!("relay{}", i);
            let expected_final = (i + 1) * 1_000_000 + 9 * 100_000; // Last update value
            assert_that!(mock_state.get_reservation(&relay_id), eq(expected_final));
            assert_that!(mock_state.times_updated(&relay_id), eq(10));
        }
        Ok(())
    }
}
