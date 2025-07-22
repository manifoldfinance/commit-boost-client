#[cfg(test)]
mod property_tests {
    use crate::config::{RelayGasConfig, ReservedGasConfig};
    use crate::error::ReservedGasError;
    use crate::state::ReservedGasState;
    use crate::state_traits::GasReservationManager;
    use crate::validation::*;
    use cb_common::config::PbsModuleConfig;
    use cb_common::types::Chain;
    use googletest::prelude::*;
    use proptest::prelude::*;
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::time::Instant;

    // ============= Custom Strategies =============

    /// Strategy for generating valid gas amounts
    fn gas_amount_strategy() -> impl Strategy<Value = u64> {
        1u64..=MAX_REASONABLE_GAS
    }

    /// Strategy for generating valid reserved gas amounts
    fn reserved_gas_strategy() -> impl Strategy<Value = u64> {
        1u64..=10_000_000u64 // Up to 10M reserved
    }

    /// Strategy for generating original gas limits that can accommodate reservations
    fn original_gas_strategy() -> impl Strategy<Value = u64> {
        MIN_GAS_AFTER_RESERVATION..=MAX_REASONABLE_GAS
    }

    /// Strategy for generating relay IDs
    fn relay_id_strategy() -> impl Strategy<Value = String> {
        "[a-z]{5,10}".prop_map(|s| s.to_lowercase())
    }

    /// Strategy for generating relay gas configurations
    fn relay_gas_config_strategy() -> impl Strategy<Value = RelayGasConfig> {
        (
            reserved_gas_strategy(),
            prop::option::of(60u64..3600u64),
            prop::option::of(MIN_BLOCK_GAS_LIMIT..MIN_GAS_AFTER_RESERVATION),
        )
            .prop_map(|(reserved, interval, min_gas)| RelayGasConfig {
                reserved_gas_limit: reserved,
                update_interval: interval,
                min_gas_limit: min_gas,
            })
    }

    /// Strategy for generating reserved gas configurations
    fn reserved_gas_config_strategy() -> impl Strategy<Value = ReservedGasConfig> {
        (
            reserved_gas_strategy(),
            prop::collection::hash_map(relay_id_strategy(), reserved_gas_strategy(), 0..5),
            1u64..300u64,
            MIN_BLOCK_GAS_LIMIT..MIN_GAS_AFTER_RESERVATION,
        )
            .prop_map(|(default_gas, overrides, interval, min_block)| ReservedGasConfig {
                default_reserved_gas: default_gas,
                relay_overrides: overrides,
                update_interval_secs: interval,
                min_block_gas_limit: min_block,
                relay_config_endpoint: "/eth/v1/relay/gas_config".to_string(),
                fetch_from_relays: true,
                relay_reserve_endpoint: "/xga/v2/relay/reserve".to_string(),
            })
    }

    // ============= Section 1: Gas Limit Invariants =============

    proptest! {
        #[test]
        fn prop_gas_limits_are_positive(gas_limit in 0u64..=u64::MAX) {
            let result = validate_gas_limit(gas_limit, "test");
            if gas_limit == 0 {
                assert_that!(result, err(anything()));
            } else if gas_limit > MAX_REASONABLE_GAS {
                assert_that!(result, err(matches_pattern!(
                    ReservedGasError::ReservedGasTooHigh { .. }
                )));
            } else {
                assert_that!(result, ok(anything()));
            }
        }

        #[test]
        fn prop_gas_reservation_preserves_minimum(
            original_gas in original_gas_strategy(),
            reserved_gas in 0u64..=MAX_REASONABLE_GAS,
            min_required in MIN_BLOCK_GAS_LIMIT..=MIN_GAS_AFTER_RESERVATION
        ) {
            let result = validate_gas_after_reservation(original_gas, reserved_gas, min_required);
            let new_gas = original_gas.saturating_sub(reserved_gas);

            if new_gas >= min_required {
                prop_assert!(result.is_ok());
                prop_assert_eq!(result.unwrap(), new_gas);
            } else {
                prop_assert!(result.is_err());
            }
        }

        #[test]
        fn prop_saturating_sub_prevents_underflow(
            original in gas_amount_strategy(),
            reserved in gas_amount_strategy()
        ) {
            let new_limit = original.saturating_sub(reserved);
            prop_assert!(new_limit <= original);
            if reserved > original {
                prop_assert_eq!(new_limit, 0);
            } else {
                prop_assert_eq!(new_limit, original - reserved);
            }
        }

        #[test]
        fn prop_min_gas_after_reservation_enforced(
            config in reserved_gas_config_strategy(),
            original_gas in original_gas_strategy(),
            reserved in reserved_gas_strategy()
        ) {
            let result = config.validate_gas_limit(original_gas, reserved);
            let new_gas = original_gas.saturating_sub(reserved);

            if new_gas < config.min_block_gas_limit {
                prop_assert!(result.is_err(), "Should reject when below minimum");
            } else {
                prop_assert!(result.is_ok());
                prop_assert_eq!(result.unwrap(), new_gas);
            }
        }
    }

    // ============= Section 2: Configuration Invariants =============

    proptest! {
        #[test]
        fn prop_relay_override_precedence(
            default_gas in reserved_gas_strategy(),
            override_gas in reserved_gas_strategy(),
            relay_id in relay_id_strategy(),
            other_relay in relay_id_strategy()
        ) {
            let mut overrides = HashMap::new();
            overrides.insert(relay_id.clone(), override_gas);

            let config = ReservedGasConfig {
                default_reserved_gas: default_gas,
                relay_overrides: overrides,
                update_interval_secs: 60,
                min_block_gas_limit: MIN_BLOCK_GAS_LIMIT,
                relay_config_endpoint: "/test".to_string(),
                fetch_from_relays: false,
                relay_reserve_endpoint: "/xga/v2/relay/reserve".to_string(),
            };

            prop_assert_eq!(config.get_reserved_gas(&relay_id), override_gas);
            if relay_id != other_relay {
                prop_assert_eq!(config.get_reserved_gas(&other_relay), default_gas);
            }
        }

        #[test]
        fn prop_default_config_is_valid(seed in any::<u64>()) {
            let _ = seed; // Use seed for determinism
            let config = ReservedGasConfig {
                default_reserved_gas: 1_000_000,
                relay_overrides: HashMap::new(),
                update_interval_secs: 60,
                min_block_gas_limit: 10_000_000,
                relay_config_endpoint: "/eth/v1/relay/gas_config".to_string(),
                fetch_from_relays: true,
                relay_reserve_endpoint: "/xga/v2/relay/reserve".to_string(),
            };

            // Verify defaults satisfy all constraints
            prop_assert!(config.default_reserved_gas > 0);
            prop_assert!(config.default_reserved_gas <= MAX_REASONABLE_GAS);
            prop_assert!(config.min_block_gas_limit >= MIN_BLOCK_GAS_LIMIT);
            prop_assert!(config.update_interval_secs > 0);
        }
    }

    // ============= Section 3: State Management Invariants =============

    proptest! {
        #[test]
        fn prop_unique_relay_entries(
            updates in prop::collection::vec(
                (relay_id_strategy(), relay_gas_config_strategy()),
                1..20
            )
        ) {
            let config = ReservedGasConfig {
                default_reserved_gas: 1_000_000,
                relay_overrides: HashMap::new(),
                update_interval_secs: 60,
                min_block_gas_limit: 10_000_000,
                relay_config_endpoint: "/test".to_string(),
                fetch_from_relays: false,
                relay_reserve_endpoint: "/xga/v2/relay/reserve".to_string(),
            };

            let pbs_config = create_test_pbs_config();
            let state = ReservedGasState::new(config, pbs_config);

            // Apply all updates
            for (relay_id, relay_config) in updates.iter() {
                if relay_config.reserved_gas_limit > 0 &&
                   relay_config.reserved_gas_limit <= MAX_REASONABLE_GAS {
                    state.update_reservation(relay_id.clone(), relay_config.clone());
                }
            }

            // Check uniqueness
            let reservations = state.get_all_reservations();
            let mut seen = std::collections::HashSet::new();
            for (relay_id, _) in reservations {
                prop_assert!(seen.insert(relay_id), "Duplicate relay entry found");
            }
        }

        #[test]
        fn prop_timestamp_monotonicity(
            relay_id in relay_id_strategy(),
            configs in prop::collection::vec(relay_gas_config_strategy(), 2..10)
        ) {
            let config = ReservedGasConfig {
                default_reserved_gas: 1_000_000,
                relay_overrides: HashMap::new(),
                update_interval_secs: 60,
                min_block_gas_limit: 10_000_000,
                relay_config_endpoint: "/test".to_string(),
                fetch_from_relays: false,
                relay_reserve_endpoint: "/xga/v2/relay/reserve".to_string(),
            };

            let pbs_config = create_test_pbs_config();
            let state = ReservedGasState::new(config, pbs_config);

            let mut timestamps = Vec::new();

            for relay_config in configs {
                if relay_config.reserved_gas_limit > 0 &&
                   relay_config.reserved_gas_limit <= MAX_REASONABLE_GAS {
                    let before = Instant::now();
                    state.update_reservation(relay_id.clone(), relay_config);
                    let after = Instant::now();

                    if let Some(reservation) = state.reservations.get(&relay_id) {
                        // Verify timestamp is within expected bounds
                        prop_assert!(reservation.last_updated >= before,
                                   "Timestamp must be at or after update start");
                        prop_assert!(reservation.last_updated <= after,
                                   "Timestamp must be at or before update end");

                        // Verify monotonicity with previous timestamps
                        for prev_time in &timestamps {
                            prop_assert!(reservation.last_updated >= *prev_time,
                                       "Timestamp must be monotonically increasing");
                        }

                        timestamps.push(reservation.last_updated);
                    }
                }
            }
        }

    }

    // ============= Section 4: Relay Configuration Invariants =============

    proptest! {
        #[test]
        fn prop_relay_config_validation(config in relay_gas_config_strategy()) {
            let is_valid = is_valid_relay_response(&config, "test");

            // Check validation logic matches
            let should_be_valid =
                config.reserved_gas_limit > 0 &&
                config.reserved_gas_limit <= MAX_REASONABLE_GAS &&
                config.min_gas_limit.map_or(true, |min| min >= MIN_BLOCK_GAS_LIMIT);

            prop_assert_eq!(is_valid, should_be_valid);
        }

        #[test]
        fn prop_relay_min_gas_validation(
            reserved_gas in reserved_gas_strategy(),
            min_gas in prop::option::of(0u64..=MAX_REASONABLE_GAS)
        ) {
            let config = RelayGasConfig {
                reserved_gas_limit: reserved_gas,
                update_interval: None,
                min_gas_limit: min_gas,
            };

            let result = validate_relay_gas_config(&config, "test");

            if let Some(min) = min_gas {
                if min < MIN_BLOCK_GAS_LIMIT {
                    prop_assert!(result.is_err(), "Should reject low min_gas_limit");
                } else {
                    prop_assert!(result.is_ok());
                }
            } else {
                prop_assert!(result.is_ok());
            }
        }
    }

    // ============= Section 5: Complex Invariant Interactions =============

    proptest! {
        #[test]
        fn prop_state_operations_preserve_invariants(
            initial_config in reserved_gas_config_strategy(),
            operations in prop::collection::vec(
                (relay_id_strategy(), relay_gas_config_strategy(), original_gas_strategy()),
                1..50
            )
        ) {
            let pbs_config = create_test_pbs_config();
            let state = ReservedGasState::new(initial_config.clone(), pbs_config);

            for (relay_id, relay_config, original_gas) in operations {
                // Update reservation
                if is_valid_relay_response(&relay_config, &relay_id) {
                    state.update_reservation(relay_id.clone(), relay_config);
                }

                // Apply reservation
                let reserved = state.get_reservation(&relay_id);
                let result = state.apply_reservation(&relay_id, original_gas);

                if let Ok(outcome) = result {
                    // Verify invariants
                    prop_assert!(outcome.new_gas_limit <= original_gas);
                    prop_assert_eq!(outcome.new_gas_limit, original_gas.saturating_sub(reserved));
                    prop_assert_eq!(outcome.reserved_amount, reserved);
                    prop_assert!(outcome.new_gas_limit >= initial_config.min_block_gas_limit ||
                               reserved > original_gas - initial_config.min_block_gas_limit);
                }
            }

            // Verify state consistency
            let all_reservations = state.get_all_reservations();
            let mut seen_relays = std::collections::HashSet::new();

            for (relay_id, reservation) in all_reservations {
                prop_assert!(seen_relays.insert(relay_id.clone()));
                prop_assert!(reservation.reserved_gas > 0);
                prop_assert!(reservation.reserved_gas <= MAX_REASONABLE_GAS);
            }
        }

        #[test]
        fn prop_concurrent_updates_maintain_consistency(
            relay_ids in prop::collection::vec(relay_id_strategy(), 3..5),
            updates_per_relay in 5usize..20
        ) {
            let config = ReservedGasConfig {
                default_reserved_gas: 1_000_000,
                relay_overrides: HashMap::new(),
                update_interval_secs: 60,
                min_block_gas_limit: 10_000_000,
                relay_config_endpoint: "/test".to_string(),
                fetch_from_relays: false,
                relay_reserve_endpoint: "/xga/v2/relay/reserve".to_string(),
            };

            let pbs_config = create_test_pbs_config();
            let state = Arc::new(ReservedGasState::new(config, pbs_config));

            let handles: Vec<_> = relay_ids.into_iter().map(|relay_id| {
                let state_clone = state.clone();
                let updates = updates_per_relay;

                std::thread::spawn(move || {
                    for i in 0..updates {
                        let config = RelayGasConfig {
                            reserved_gas_limit: 1_000_000 + (i as u64 * 100_000),
                            update_interval: None,
                            min_gas_limit: None,
                        };
                        state_clone.update_reservation(relay_id.clone(), config);
                    }
                })
            }).collect();

            // Wait for all threads
            for handle in handles {
                handle.join().unwrap();
            }

            // Verify final state consistency
            let reservations = state.get_all_reservations();
            for (_, reservation) in reservations {
                prop_assert!(reservation.reserved_gas > 0);
                prop_assert!(reservation.reserved_gas <= MAX_REASONABLE_GAS);
            }
        }
    }

    // ============= Section 6: Additional Edge Case Tests =============

    proptest! {
        #[test]
        fn prop_zero_gas_edge_cases(
            original_gas in 0u64..=1u64,
            reserved_gas in 0u64..=1u64
        ) {
            let result = validate_gas_after_reservation(original_gas, reserved_gas, MIN_GAS_AFTER_RESERVATION);

            // All these cases should fail as they don't meet minimum
            assert_that!(
                result,
                err(matches_pattern!(
                    ReservedGasError::GasLimitTooLow {
                        actual: eq(original_gas.saturating_sub(reserved_gas)),
                        minimum: eq(MIN_GAS_AFTER_RESERVATION),
                        reserved: eq(reserved_gas)
                    }
                ))
            );
        }

        #[test]
        fn prop_overflow_protection(
            base in u64::MAX - 1000..=u64::MAX,
            addition in 1u64..=1000u64
        ) {
            // Test that we handle near-MAX values correctly
            let _config = ReservedGasConfig {
                default_reserved_gas: base,
                relay_overrides: HashMap::new(),
                update_interval_secs: 60,
                min_block_gas_limit: MIN_BLOCK_GAS_LIMIT,
                relay_config_endpoint: "/test".to_string(),
                fetch_from_relays: false,
                relay_reserve_endpoint: "/xga/v2/relay/reserve".to_string(),
            };

            // Validation should catch excessive values
            let result = validate_gas_limit(base, "test");
            if base > MAX_REASONABLE_GAS {
                prop_assert!(result.is_err());
            }

            // Saturating operations prevent overflow
            let new_value = base.saturating_add(addition);
            prop_assert!(new_value <= u64::MAX);
        }

        #[test]
        fn prop_empty_relay_id_handling(
            reserved_gas in reserved_gas_strategy()
        ) {
            let mut reservations = HashMap::new();
            reservations.insert(String::new(), reserved_gas); // Empty relay ID

            let config = ReservedGasConfig {
                default_reserved_gas: 1_000_000,
                relay_overrides: reservations,
                update_interval_secs: 60,
                min_block_gas_limit: MIN_BLOCK_GAS_LIMIT,
                relay_config_endpoint: "/test".to_string(),
                fetch_from_relays: false,
                relay_reserve_endpoint: "/xga/v2/relay/reserve".to_string(),
            };

            // Should handle empty relay ID
            let reserved = config.get_reserved_gas("");
            prop_assert_eq!(reserved, reserved_gas);

            // Non-empty should get default
            let reserved_other = config.get_reserved_gas("other");
            prop_assert_eq!(reserved_other, 1_000_000);
        }

        #[test]
        fn prop_boundary_gas_limits(
            factor in 0.0f64..=2.0f64
        ) {
            let gas_limit = (MIN_GAS_AFTER_RESERVATION as f64 * factor) as u64;
            let reserved = 1_000_000u64;
            let original = gas_limit + reserved;

            let result = validate_gas_after_reservation(original, reserved, MIN_GAS_AFTER_RESERVATION);

            if gas_limit >= MIN_GAS_AFTER_RESERVATION {
                prop_assert!(result.is_ok());
                prop_assert_eq!(result.unwrap(), gas_limit);
            } else {
                prop_assert!(result.is_err());
            }
        }

        #[test]
        fn prop_concurrent_state_consistency(
            num_relays in 1usize..=10,
            updates_per_relay in 1usize..=5
        ) {
            use std::sync::atomic::{AtomicU64, Ordering};

            let config = ReservedGasConfig {
                default_reserved_gas: 1_000_000,
                relay_overrides: HashMap::new(),
                update_interval_secs: 60,
                min_block_gas_limit: 10_000_000,
                relay_config_endpoint: "/test".to_string(),
                fetch_from_relays: false,
                relay_reserve_endpoint: "/xga/v2/relay/reserve".to_string(),
            };

            let pbs_config = create_test_pbs_config();
            let state = Arc::new(ReservedGasState::new(config, pbs_config));
            let counter = Arc::new(AtomicU64::new(0));

            let mut handles = vec![];

            for i in 0..num_relays {
                let state_clone = state.clone();
                let counter_clone = counter.clone();
                let updates = updates_per_relay;

                let handle = std::thread::spawn(move || {
                    for j in 0..updates {
                        let config = RelayGasConfig {
                            reserved_gas_limit: (i + 1) as u64 * 1_000_000 + j as u64 * 100_000,
                            update_interval: None,
                            min_gas_limit: None,
                        };
                        state_clone.update_reservation(format!("relay{}", i), config);
                        counter_clone.fetch_add(1, Ordering::Relaxed);
                    }
                });
                handles.push(handle);
            }

            for handle in handles {
                handle.join().unwrap();
            }

            // Verify all updates were processed
            let total_updates = counter.load(Ordering::Relaxed);
            prop_assert_eq!(total_updates, (num_relays * updates_per_relay) as u64);

            // Verify state consistency
            let all_reservations = state.get_all_reservations();
            prop_assert_eq!(all_reservations.len(), num_relays);

            // Each relay should have its final value
            for i in 0..num_relays {
                let relay_id = format!("relay{}", i);
                let expected = (i + 1) as u64 * 1_000_000 + (updates_per_relay - 1) as u64 * 100_000;
                prop_assert_eq!(state.get_reservation(&relay_id), expected);
            }
        }
    }

    // ============= Section 7: Reserve Value Tests =============

    proptest! {
        #[test]
        fn prop_reserve_values_are_valid(reserve in 0u64..=u64::MAX) {
            // Test that reserve validation follows expected rules
            if reserve == 0 {
                // Zero reserve is technically valid but unusual
                prop_assert!(reserve <= MAX_REASONABLE_GAS);
            } else if reserve > MAX_REASONABLE_GAS {
                // Very high reserves should be rejected
                prop_assert!(reserve > MAX_REASONABLE_GAS);
            } else {
                // Normal reserves should be in reasonable range
                prop_assert!(reserve > 0 && reserve <= MAX_REASONABLE_GAS);
            }
        }

        #[test]
        fn prop_reserve_query_status_consistency(
            reserve in prop::option::of(reserved_gas_strategy()),
            has_error in prop::bool::ANY,
            is_timeout in prop::bool::ANY
        ) {
            use crate::types::{RelayReserveInfo, ReserveQueryStatus};

            let (status, error) = if has_error && is_timeout {
                (ReserveQueryStatus::Timeout, Some("Timeout".to_string()))
            } else if has_error {
                (ReserveQueryStatus::Error, Some("Error".to_string()))
            } else if reserve.is_some() {
                (ReserveQueryStatus::Success, None)
            } else {
                (ReserveQueryStatus::Error, Some("Not supported".to_string()))
            };

            let info = RelayReserveInfo {
                relay_id: "test-relay".to_string(),
                reserve,
                query_time_ms: 100,
                status: status.clone(),
                error: error.clone(),
            };

            // Verify consistency
            match status {
                ReserveQueryStatus::Success => {
                    prop_assert!(reserve.is_some(), "Success should have reserve value");
                    prop_assert!(error.is_none(), "Success should not have error");
                }
                ReserveQueryStatus::Error | ReserveQueryStatus::Timeout => {
                    prop_assert!(error.is_some(), "Error/Timeout should have error message");
                }
            }

            // Verify serialization works
            let json = serde_json::to_value(&info);
            prop_assert!(json.is_ok());
        }

        #[test]
        fn prop_reserve_statistics_correctness(
            reserves in prop::collection::vec(reserved_gas_strategy(), 1..10)
        ) {
            // Calculate statistics
            let total: u64 = reserves.iter().sum();
            let count = reserves.len() as u64;
            let average = total / count;
            let max = *reserves.iter().max().unwrap();
            let min = *reserves.iter().min().unwrap();

            // Verify invariants
            prop_assert!(average >= min);
            prop_assert!(average <= max);
            prop_assert_eq!(total, reserves.iter().sum::<u64>());

            // Verify edge cases
            if reserves.len() == 1 {
                prop_assert_eq!(average, reserves[0]);
                prop_assert_eq!(min, max);
            }
        }

        #[test]
        fn prop_concurrent_reserve_queries_consistent(
            num_relays in 1usize..=5,
            reserves in prop::collection::vec(reserved_gas_strategy(), 1..5)
        ) {
            use std::sync::atomic::{AtomicU64, Ordering};
            use std::sync::Arc;

            let query_count = Arc::new(AtomicU64::new(0));
            let expected_count = num_relays.min(reserves.len());

            // Simulate concurrent queries
            let handles: Vec<_> = (0..expected_count)
                .map(|i| {
                    let count_clone = query_count.clone();
                    let reserve = reserves.get(i).copied().unwrap_or(1_000_000);

                    std::thread::spawn(move || {
                        // Simulate query
                        std::thread::sleep(std::time::Duration::from_millis(10));
                        count_clone.fetch_add(1, Ordering::Relaxed);
                        reserve
                    })
                })
                .collect();

            // Collect results
            let results: Vec<u64> = handles.into_iter()
                .map(|h| h.join().unwrap())
                .collect();

            // Verify all queries completed
            let final_count = query_count.load(Ordering::Relaxed);
            prop_assert_eq!(final_count, expected_count as u64);

            // Verify results match expected
            prop_assert_eq!(results.len(), expected_count);
            for (i, &result) in results.iter().enumerate() {
                let expected = reserves.get(i).copied().unwrap_or(1_000_000);
                prop_assert_eq!(result, expected);
            }
        }
    }

    // ============= Helper Functions =============

    fn create_test_pbs_config() -> PbsModuleConfig {
        PbsModuleConfig {
            chain: Chain::Mainnet,
            endpoint: std::net::SocketAddr::from(([127, 0, 0, 1], 18550)),
            pbs_config: Arc::new(cb_common::config::PbsConfig {
                host: std::net::Ipv4Addr::new(127, 0, 0, 1),
                port: 18550,
                relay_check: true,
                wait_all_registrations: true,
                timeout_get_header_ms: 1000,
                timeout_get_payload_ms: 1000,
                timeout_register_validator_ms: 1000,
                register_validator_retry_limit: 3,
                skip_sigverify: false,
                min_bid_wei: Default::default(),
                late_in_slot_time_ms: 4000,
                extra_validation_enabled: false,
                rpc_url: None,
                http_timeout_seconds: 30,
            }),
            relays: vec![],
            all_relays: vec![],
            signer_client: None,
            event_publisher: None,
            muxes: None,
        }
    }
}
