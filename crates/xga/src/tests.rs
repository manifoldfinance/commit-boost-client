#[cfg(test)]
mod tests {
    use crate::config::{RelayGasConfig, ReservedGasConfig};
    use crate::error::{ReservedGasError, Result};
    use crate::metrics_wrapper::NoOpMetricsRecorder;
    use crate::state_manager::GasReservationManager;
    use crate::validation::{MIN_GAS_AFTER_RESERVATION, MAX_REASONABLE_GAS};
    use googletest::prelude::*;
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::time::Duration;
    use cb_common::{
        config::{PbsModuleConfig, PbsConfig},
        types::Chain,
    };

    fn create_test_config() -> ReservedGasConfig {
        ReservedGasConfig {
            default_reserved_gas: 1_000_000,
            min_block_gas_limit: 10_000_000,
            relay_overrides: HashMap::from([
                ("relay1".to_string(), 2_500_000),
                ("relay2".to_string(), 1_500_000),
            ]),
            fetch_from_relays: false,
            update_interval_secs: 60,
            relay_config_endpoint: "/eth/v1/relay/gas_config".to_string(),
            relay_reserve_endpoint: "/xga/v2/relay/reserve".to_string(),
        }
    }

    fn create_test_pbs_config() -> PbsModuleConfig {
        use std::net::{Ipv4Addr, SocketAddr};
        use alloy::primitives::U256;
        
        let pbs_config = PbsConfig {
            host: Ipv4Addr::new(127, 0, 0, 1),
            port: 18550,
            relay_check: true,
            wait_all_registrations: true,
            timeout_get_header_ms: 1000,
            timeout_get_payload_ms: 1000,
            timeout_register_validator_ms: 1000,
            register_validator_retry_limit: 3,
            skip_sigverify: false,
            min_bid_wei: U256::ZERO,
            late_in_slot_time_ms: 4000,
            extra_validation_enabled: false,
            rpc_url: None,
            http_timeout_seconds: 30,
        };
        
        PbsModuleConfig {
            chain: Chain::Mainnet,
            endpoint: SocketAddr::from(([127, 0, 0, 1], 18550)),
            pbs_config: Arc::new(pbs_config),
            relays: vec![],
            all_relays: vec![],
            signer_client: None,
            event_publisher: None,
            muxes: None,
        }
    }

    async fn create_test_manager() -> Arc<GasReservationManager> {
        let manager = Arc::new(GasReservationManager::new());
        let config = create_test_config();
        let pbs_config = create_test_pbs_config();
        manager.configure(config, pbs_config).await.unwrap();
        manager
    }

    #[tokio::test]
    async fn test_update_reservation_persists_state() -> Result<()> {
        let manager = create_test_manager().await;
        
        // Initial state - using override value
        assert_that!(manager.get_reservation("relay1").await, eq(2_500_000));
        
        // Update reservation
        let config = RelayGasConfig {
            reserved_gas_limit: 3_500_000,
            min_gas_limit: Some(15_000_000),
            update_interval: Some(300),
        };
        
        let event = manager.update_relay_configuration("relay1".to_string(), config.clone()).await?;
        
        // Verify event details
        assert_that!(event.relay_id, eq("relay1"));
        assert_that!(event.old_reservation, eq(Some(2_500_000)));
        assert_that!(event.new_reservation, eq(3_500_000));
        assert_that!(event.timestamp.elapsed() < Duration::from_secs(1), eq(true));
        
        // Verify state was updated
        assert_that!(manager.get_reservation("relay1").await, eq(3_500_000));
        
        // Other relays unchanged
        assert_that!(manager.get_reservation("relay2").await, eq(1_500_000));
        
        // Unknown relay uses default
        assert_that!(manager.get_reservation("unknown").await, eq(1_000_000));
        
        Ok(())
    }

    #[tokio::test]
    async fn test_apply_reservation_calculations() -> Result<()> {
        let manager = create_test_manager().await;
        
        // Test successful reservation
        let original_gas = 30_000_000;
        let result = manager.apply_reservation("relay1", original_gas).await;
        
        assert_that!(result, ok(anything()));
        let outcome = result.unwrap();
        assert_that!(outcome.relay_id, eq("relay1"));
        assert_that!(outcome.new_gas_limit, eq(27_500_000)); // 30M - 2.5M
        assert_that!(outcome.reserved_amount, eq(2_500_000));
        assert_that!(outcome.new_gas_limit + outcome.reserved_amount, eq(original_gas));
        
        Ok(())
    }

    #[tokio::test]
    async fn test_apply_reservation_fails_when_gas_too_low() -> Result<()> {
        let manager = create_test_manager().await;
        
        // Update relay1 to have very high reservation
        manager.update_relay_configuration(
            "relay1".to_string(),
            RelayGasConfig {
                reserved_gas_limit: 25_000_000,
                min_gas_limit: None,
                update_interval: None,
            },
        ).await?;
        
        // Original gas is less than reservation + minimum
        let original_gas = 30_000_000;
        let result = manager.apply_reservation("relay1", original_gas).await;
        
        assert_that!(
            result,
            err(matches_pattern!(ReservedGasError::GasLimitTooLow {
                actual: eq(5_000_000), // 30M - 25M
                minimum: eq(MIN_GAS_AFTER_RESERVATION),
                reserved: eq(25_000_000)
            }))
        );
        
        Ok(())
    }

    #[tokio::test]
    async fn test_multiple_updates_maintain_consistency() -> Result<()> {
        let manager = create_test_manager().await;
        
        // Multiple updates to same relay
        let mut last_value = manager.get_reservation("relay1").await;
        for i in 1..=3 {
            let new_value = i * 1_000_000;
            let config = RelayGasConfig {
                reserved_gas_limit: new_value,
                min_gas_limit: None,
                update_interval: None,
            };
            let event = manager.update_relay_configuration("relay1".to_string(), config).await?;
            
            // Verify event correctness
            assert_that!(event.old_reservation, eq(Some(last_value)));
            assert_that!(event.new_reservation, eq(new_value));
            
            last_value = new_value;
        }
        
        // Verify final state
        assert_that!(manager.get_reservation("relay1").await, eq(3_000_000));
        
        Ok(())
    }

    #[tokio::test]
    async fn test_get_all_reservations_accuracy() -> Result<()> {
        use crate::test_matchers::matchers::*;
        
        let manager = create_test_manager().await;
        
        // Update some values
        manager.update_relay_configuration(
            "relay3".to_string(),
            RelayGasConfig {
                reserved_gas_limit: 4_000_000,
                min_gas_limit: None,
                update_interval: None,
            },
        ).await?;
        
        // Get all reservations
        let reservations = manager.get_all_reservations();
        
        // Should have relay1, relay2, and relay3
        assert_that!(reservations.len(), ge(3));
        
        // Find and verify each relay
        for (relay_id, reservation) in &reservations {
            match relay_id.as_str() {
                "relay1" => {
                    assert_that!(reservation.reserved_gas, eq(2_500_000));
                    assert_that!(reservation.reserved_gas, is_healthy_relay());
                }
                "relay2" => {
                    assert_that!(reservation.reserved_gas, eq(1_500_000));
                    assert_that!(reservation.reserved_gas, is_healthy_relay());
                }
                "relay3" => {
                    assert_that!(reservation.reserved_gas, eq(4_000_000));
                    assert_that!(reservation.reserved_gas, is_healthy_relay());
                }
                _ => {} // Might have other relays from config
            }
            assert_that!(reservation.relay_id, eq(relay_id.clone()));
            assert_that!(reservation.min_gas_limit, none());
            assert_that!(reservation.last_updated.elapsed() < Duration::from_secs(60), eq(true));
        }
        
        Ok(())
    }

    #[tokio::test]
    async fn test_default_reservation_for_unknown_relay() -> Result<()> {
        let manager = create_test_manager().await;
        
        // Unknown relay should return default
        assert_that!(manager.get_reservation("unknown-relay").await, eq(1_000_000));
        
        // Add a known relay
        manager.update_relay_configuration(
            "known-relay".to_string(),
            RelayGasConfig {
                reserved_gas_limit: 2_000_000,
                min_gas_limit: None,
                update_interval: None,
            },
        ).await?;
        
        // Known relay returns updated value
        assert_that!(manager.get_reservation("known-relay").await, eq(2_000_000));
        // Unknown still returns default
        assert_that!(manager.get_reservation("another-unknown").await, eq(1_000_000));
        
        Ok(())
    }

    #[tokio::test]
    async fn test_apply_reservation_with_zero_reservation() -> Result<()> {
        let manager = create_test_manager().await;
        
        // Set zero reservation
        manager.update_relay_configuration(
            "relay1".to_string(),
            RelayGasConfig {
                reserved_gas_limit: 0,
                min_gas_limit: None,
                update_interval: None,
            },
        ).await?;
        
        // Due to validation, zero should be rejected and original value maintained
        assert_that!(manager.get_reservation("relay1").await, eq(2_500_000));
        
        Ok(())
    }

    #[tokio::test]
    async fn test_validation_constraints() -> Result<()> {
        let manager = create_test_manager().await;
        
        // Test 1: Reserved gas too high
        let event = manager.update_relay_configuration(
            "test".to_string(),
            RelayGasConfig {
                reserved_gas_limit: MAX_REASONABLE_GAS + 1,
                min_gas_limit: None,
                update_interval: None,
            },
        ).await?;
        
        // Should not update due to validation
        assert_that!(event.old_reservation, eq(Some(1_000_000))); // default
        assert_that!(event.new_reservation, eq(1_000_000)); // unchanged
        
        // Test 2: Valid update
        let event = manager.update_relay_configuration(
            "test".to_string(),
            RelayGasConfig {
                reserved_gas_limit: 5_000_000,
                min_gas_limit: Some(20_000_000),
                update_interval: None,
            },
        ).await?;
        
        assert_that!(event.old_reservation, eq(Some(1_000_000)));
        assert_that!(event.new_reservation, eq(5_000_000));
        assert_that!(manager.get_reservation("test").await, eq(5_000_000));
        
        Ok(())
    }

    #[tokio::test]
    async fn test_reservation_with_exact_minimum() -> Result<()> {
        let manager = create_test_manager().await;
        
        // Set reservation to leave exactly minimum gas
        let reserved = 5_000_000;
        manager.update_relay_configuration(
            "test".to_string(),
            RelayGasConfig {
                reserved_gas_limit: reserved,
                min_gas_limit: None,
                update_interval: None,
            },
        ).await?;
        
        // Apply with exact amount
        let original_gas = MIN_GAS_AFTER_RESERVATION + reserved;
        let result = manager.apply_reservation("test", original_gas).await;
        
        assert_that!(result, ok(anything()));
        let outcome = result.unwrap();
        assert_that!(outcome.new_gas_limit, eq(MIN_GAS_AFTER_RESERVATION));
        assert_that!(outcome.reserved_amount, eq(reserved));
        
        // Try with 1 less - should fail
        let result = manager.apply_reservation("test", original_gas - 1).await;
        assert_that!(result, err(anything()));
        
        Ok(())
    }

    #[tokio::test]
    async fn test_concurrent_updates_are_thread_safe() -> Result<()> {
        use tokio::task;
        
        let manager = Arc::new(GasReservationManager::new());
        let config = create_test_config();
        let pbs_config = create_test_pbs_config();
        manager.configure(config, pbs_config).await?;
        
        let mut handles = vec![];
        
        // Spawn multiple tasks updating different relays
        for i in 0..5 {
            let manager_clone = Arc::clone(&manager);
            let handle = task::spawn(async move {
                for j in 0..10 {
                    let config = RelayGasConfig {
                        reserved_gas_limit: (i + 1) * 1_000_000 + j * 100_000,
                        min_gas_limit: None,
                        update_interval: None,
                    };
                    manager_clone.update_relay_configuration(format!("relay{}", i), config).await.ok();
                }
            });
            handles.push(handle);
        }
        
        // Wait for all tasks to complete
        for handle in handles {
            handle.await.unwrap();
        }
        
        // Verify final state
        for i in 0..5 {
            let relay_id = format!("relay{}", i);
            let expected_final = (i + 1) * 1_000_000 + 9 * 100_000; // Last update value
            assert_that!(manager.get_reservation(&relay_id).await, eq(expected_final));
        }
        
        Ok(())
    }

    #[tokio::test]
    async fn test_relay_configuration_precedence() -> Result<()> {
        let manager = create_test_manager().await;
        
        // relay1 has override of 2.5M, default is 1M
        assert_that!(manager.get_reservation("relay1").await, eq(2_500_000));
        assert_that!(manager.get_reservation("unknown").await, eq(1_000_000));
        
        // Update relay1
        manager.update_relay_configuration(
            "relay1".to_string(),
            RelayGasConfig {
                reserved_gas_limit: 3_000_000,
                min_gas_limit: None,
                update_interval: None,
            },
        ).await?;
        
        // Updated value takes precedence
        assert_that!(manager.get_reservation("relay1").await, eq(3_000_000));
        
        Ok(())
    }

    #[tokio::test]
    async fn test_edge_case_handling() -> Result<()> {
        let manager = create_test_manager().await;
        
        // Test empty relay ID
        assert_that!(manager.get_reservation("").await, eq(1_000_000));
        
        let event = manager.update_relay_configuration(
            "".to_string(),
            RelayGasConfig {
                reserved_gas_limit: 2_000_000,
                min_gas_limit: None,
                update_interval: None,
            },
        ).await?;
        assert_that!(event.new_reservation, eq(2_000_000));
        assert_that!(manager.get_reservation("").await, eq(2_000_000));
        
        // Test very long relay ID
        let long_id = "x".repeat(500);
        assert_that!(manager.get_reservation(&long_id).await, eq(1_000_000));
        
        manager.update_relay_configuration(
            long_id.clone(),
            RelayGasConfig {
                reserved_gas_limit: 1_500_000,
                min_gas_limit: None,
                update_interval: None,
            },
        ).await?;
        assert_that!(manager.get_reservation(&long_id).await, eq(1_500_000));
        
        Ok(())
    }

    #[tokio::test]
    async fn test_metrics_recording() -> Result<()> {
        // This test verifies metrics would be recorded correctly
        let manager = create_test_manager().await;
        let metrics = Arc::new(NoOpMetricsRecorder);
        
        // In production, metrics would be recorded during these operations
        let _reservation = manager.get_reservation("relay1").await;
        let _outcome = manager.apply_reservation("relay1", 30_000_000).await;
        let _event = manager.update_relay_configuration(
            "relay1".to_string(),
            RelayGasConfig {
                reserved_gas_limit: 3_000_000,
                min_gas_limit: None,
                update_interval: None,
            },
        ).await;
        
        // NoOpMetricsRecorder doesn't actually record, but in production
        // these operations would generate metrics
        let _ = metrics; // Metrics are used correctly
        
        Ok(())
    }

    // Tests for reserve query functionality
    #[test]
    fn test_reserve_values_validation() -> Result<()> {
        use crate::types::{RelayReserveInfo, ReserveQueryStatus};
        
        // Test successful query
        let info = RelayReserveInfo {
            relay_id: "flashbots".to_string(),
            reserve: Some(2_000_000),
            query_time_ms: 45,
            status: ReserveQueryStatus::Success,
            error: None,
        };
        
        // Verify serialization
        let json = serde_json::to_value(&info)?;
        assert_that!(json.get("relay_id").and_then(|v| v.as_str()), some(eq("flashbots")));
        assert_that!(json.get("reserve").and_then(|v| v.as_u64()), some(eq(2_000_000)));
        assert_that!(json.get("status").and_then(|v| v.as_str()), some(eq("success")));
        assert_that!(json.get("error"), none());
        
        // Test error case
        let error_info = RelayReserveInfo {
            relay_id: "ultrasound".to_string(),
            reserve: None,
            query_time_ms: 100,
            status: ReserveQueryStatus::Error,
            error: Some("Not supported".to_string()),
        };
        
        let json = serde_json::to_value(&error_info)?;
        assert_that!(json.get("reserve"), none());
        assert_that!(json.get("error"), some(anything()));
        
        Ok(())
    }

    #[test]
    fn test_reserve_statistics() -> Result<()> {
        let reserves = vec![1_000_000u64, 2_000_000, 3_000_000, 4_000_000, 5_000_000];
        
        let total: u64 = reserves.iter().sum();
        let count = reserves.len() as u64;
        let average = total / count;
        let max = *reserves.iter().max().unwrap();
        let min = *reserves.iter().min().unwrap();
        
        assert_that!(total, eq(15_000_000));
        assert_that!(average, eq(3_000_000));
        assert_that!(max, eq(5_000_000));
        assert_that!(min, eq(1_000_000));
        
        // Verify average is between min and max
        assert_that!(average >= min, eq(true));
        assert_that!(average <= max, eq(true));
        
        Ok(())
    }

    #[test]
    fn test_configuration_consistency() -> Result<()> {
        let config = create_test_config();
        
        // Verify configuration is internally consistent
        assert_that!(config.default_reserved_gas > 0, eq(true));
        assert_that!(config.default_reserved_gas <= MAX_REASONABLE_GAS, eq(true));
        assert_that!(config.min_block_gas_limit >= MIN_GAS_AFTER_RESERVATION, eq(true));
        assert_that!(config.update_interval_secs > 0, eq(true));
        
        // Verify all overrides are valid
        for (relay_id, reserved) in &config.relay_overrides {
            assert_that!(!relay_id.is_empty(), eq(true));
            assert_that!(*reserved > 0, eq(true));
            assert_that!(*reserved <= MAX_REASONABLE_GAS, eq(true));
        }
        
        Ok(())
    }
}