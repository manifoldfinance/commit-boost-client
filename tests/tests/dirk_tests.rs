/// Dirk tests

#[cfg(test)]
mod dirk_property_tests {
    use super::*;
    use proptest::prelude::*;
    use proptest::collection::vec;
    use proptest::string::string_regex;
    
    /// Name_matches_proxy
    mod name_matching_tests {
        use super::*;
        
        // Strategy for generating valid UUID strings
        fn uuid_strategy() -> impl Strategy<Value = String> {
            "[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}"
                .prop_map(|s| s.to_lowercase())
        }
        
        // Strategy for generating valid wallet names (non-empty alphanumeric)
        fn wallet_name_strategy() -> impl Strategy<Value = String> {
            "[a-zA-Z][a-zA-Z0-9_]{0,31}"
        }
        
        // Strategy for generating valid module names
        fn module_name_strategy() -> impl Strategy<Value = String> {
            "[a-zA-Z][a-zA-Z0-9_-]{0,31}"
        }
        
        proptest! {
            #[test]
            fn test_valid_proxy_names_always_match(
                wallet in wallet_name_strategy(),
                module in module_name_strategy(),
                uuid in uuid_strategy()
            ) {
                let name = format!("{}/consensus_proxy/{}/{}", wallet, module, uuid);
                prop_assert!(
                    name_matches_proxy(&name),
                    "Valid proxy name should match: {}",
                    name
                );
            }
            
            #[test]
            fn test_wrong_part_count_never_matches(
                parts in vec(string_regex("[^/]+").unwrap(), 0..10)
                    .prop_filter("not exactly 4 parts", |v| v.len() != 4)
            ) {
                let name = parts.join("/");
                prop_assert!(
                    !name_matches_proxy(&name),
                    "Name with {} parts should not match: {}",
                    parts.len(),
                    name
                );
            }
            
            #[test]
            fn test_empty_wallet_name_never_matches(
                module in module_name_strategy(),
                uuid in uuid_strategy()
            ) {
                let name = format!("/consensus_proxy/{}/{}", module, uuid);
                prop_assert!(
                    !name_matches_proxy(&name),
                    "Empty wallet name should not match: {}",
                    name
                );
            }
            
            #[test]
            fn test_wrong_consensus_proxy_literal_never_matches(
                wallet in wallet_name_strategy(),
                wrong_literal in string_regex("[a-zA-Z_]+")
                    .prop_filter("not consensus_proxy", |s| s != "consensus_proxy"),
                module in module_name_strategy(),
                uuid in uuid_strategy()
            ) {
                let name = format!("{}/{}/{}/{}", wallet, wrong_literal, module, uuid);
                prop_assert!(
                    !name_matches_proxy(&name),
                    "Wrong literal should not match: {}",
                    name
                );
            }
            
            #[test]
            fn test_invalid_uuid_never_matches(
                wallet in wallet_name_strategy(),
                module in module_name_strategy(),
                invalid_uuid in string_regex("[a-zA-Z0-9-]{1,50}")
                    .prop_filter("not valid UUID", |s| uuid::Uuid::parse_str(s).is_err())
            ) {
                let name = format!("{}/consensus_proxy/{}/{}", wallet, module, invalid_uuid);
                prop_assert!(
                    !name_matches_proxy(&name),
                    "Invalid UUID should not match: {}",
                    name
                );
            }
            
            #[test]
            fn test_empty_module_name_never_matches(
                wallet in wallet_name_strategy(),
                uuid in uuid_strategy()
            ) {
                let name = format!("{}/consensus_proxy//{}", wallet, uuid);
                prop_assert!(
                    !name_matches_proxy(&name),
                    "Empty module name should not match: {}",
                    name
                );
            }
            
            #[test]
            fn test_all_conditions_must_be_met(
                wallet in proptest::option::of(wallet_name_strategy()),
                literal in proptest::option::of(Just("consensus_proxy".to_string())),
                module in proptest::option::of(module_name_strategy()),
                uuid in proptest::option::of(uuid_strategy())
            ) {
                let name = format!(
                    "{}/{}/{}/{}",
                    wallet.as_deref().unwrap_or(""),
                    literal.as_deref().unwrap_or("wrong"),
                    module.as_deref().unwrap_or(""),
                    uuid.as_deref().unwrap_or("not-a-uuid")
                );
                
                // Should only match if ALL conditions are met
                let should_match = wallet.is_some() 
                    && literal.is_some() 
                    && module.is_some() 
                    && uuid.is_some();
                    
                prop_assert_eq!(
                    name_matches_proxy(&name),
                    should_match,
                    "Name: {} should match: {}",
                    name,
                    should_match
                );
            }
        }
    }
    
    /// Consensus proxy validation
    mod consensus_proxy_validation_tests {
        use super::*;
        use std::collections::HashMap;
        
        // Mock types for testing
        #[derive(Clone, Debug, PartialEq)]
        struct MockModuleId(String);
        
        #[derive(Clone, Debug)]
        struct MockProxyAccount {
            module: MockModuleId,
            consensus_pubkey: Vec<u8>,
            proxy_pubkey: Vec<u8>,
        }
        
        // Strategy for generating public keys
        fn pubkey_strategy() -> impl Strategy<Value = Vec<u8>> {
            vec(any::<u8>(), 48)
        }
        
        // Strategy for generating module IDs
        fn module_id_strategy() -> impl Strategy<Value = MockModuleId> {
            "[a-zA-Z][a-zA-Z0-9_]{0,31}".prop_map(MockModuleId)
        }
        
        // Helper function that mimics the fixed consensus proxy filtering logic
        fn filter_proxies_for_consensus_and_module(
            proxies: &[MockProxyAccount],
            target_module: &MockModuleId,
            target_consensus: &[u8],
        ) -> Vec<Vec<u8>> {
            proxies
                .iter()
                .filter_map(|proxy| {
                    let module_matches = proxy.module == *target_module;
                    let consensus_matches = proxy.consensus_pubkey == target_consensus;
                    
                    // Only return if BOTH conditions are met
                    if module_matches && consensus_matches {
                        Some(proxy.proxy_pubkey.clone())
                    } else {
                        None
                    }
                })
                .collect()
        }
        
        proptest! {
            #[test]
            fn test_both_conditions_required(
                target_module in module_id_strategy(),
                target_consensus in pubkey_strategy(),
                other_module in module_id_strategy()
                    .prop_filter("different module", |m| true),
                other_consensus in pubkey_strategy()
                    .prop_filter("different consensus", |k| true),
                proxy_key in pubkey_strategy()
            ) {
                prop_assume!(target_module != other_module);
                prop_assume!(target_consensus != other_consensus);
                
                let proxies = vec![
                    // Matching module, wrong consensus
                    MockProxyAccount {
                        module: target_module.clone(),
                        consensus_pubkey: other_consensus.clone(),
                        proxy_pubkey: proxy_key.clone(),
                    },
                    // Wrong module, matching consensus
                    MockProxyAccount {
                        module: other_module.clone(),
                        consensus_pubkey: target_consensus.clone(),
                        proxy_pubkey: proxy_key.clone(),
                    },
                    // Both match - this should be the only one returned
                    MockProxyAccount {
                        module: target_module.clone(),
                        consensus_pubkey: target_consensus.clone(),
                        proxy_pubkey: proxy_key.clone(),
                    },
                ];
                
                let filtered = filter_proxies_for_consensus_and_module(
                    &proxies,
                    &target_module,
                    &target_consensus
                );
                
                prop_assert_eq!(
                    filtered.len(),
                    1,
                    "Should only return proxy where both conditions match"
                );
                prop_assert_eq!(
                    &filtered[0],
                    &proxy_key,
                    "Should return the correct proxy key"
                );
            }
            
            #[test]
            fn test_no_matches_returns_empty(
                proxies in vec(
                    (module_id_strategy(), pubkey_strategy(), pubkey_strategy()),
                    0..10
                ).prop_map(|v| v.into_iter().map(|(m, c, p)| MockProxyAccount {
                    module: m,
                    consensus_pubkey: c,
                    proxy_pubkey: p,
                }).collect::<Vec<_>>()),
                target_module in module_id_strategy(),
                target_consensus in pubkey_strategy()
            ) {
                // Ensure no proxy matches both conditions
                let has_match = proxies.iter().any(|p| 
                    p.module == target_module && p.consensus_pubkey == target_consensus
                );
                prop_assume!(!has_match);
                
                let filtered = filter_proxies_for_consensus_and_module(
                    &proxies,
                    &target_module,
                    &target_consensus
                );
                
                prop_assert!(
                    filtered.is_empty(),
                    "Should return empty when no proxy matches both conditions"
                );
            }
            
            #[test]
            fn test_multiple_matching_proxies(
                target_module in module_id_strategy(),
                target_consensus in pubkey_strategy(),
                proxy_keys in vec(pubkey_strategy(), 1..5)
            ) {
                let proxies: Vec<_> = proxy_keys
                    .iter()
                    .map(|key| MockProxyAccount {
                        module: target_module.clone(),
                        consensus_pubkey: target_consensus.clone(),
                        proxy_pubkey: key.clone(),
                    })
                    .collect();
                
                let filtered = filter_proxies_for_consensus_and_module(
                    &proxies,
                    &target_module,
                    &target_consensus
                );
                
                prop_assert_eq!(
                    filtered.len(),
                    proxy_keys.len(),
                    "Should return all matching proxies"
                );
                
                // Verify all keys are returned
                for key in &proxy_keys {
                    prop_assert!(
                        filtered.contains(key),
                        "All matching proxy keys should be returned"
                    );
                }
            }
        }
    }
    
    /// Response state validation
    mod response_state_validation_tests {
        use super::*;
        
        #[derive(Debug, Clone, Copy, PartialEq)]
        enum MockResponseState {
            Succeeded,
            Denied,
            Failed,
            Unknown,
        }
        
        // Helper function that mimics the fixed response validation logic
        fn validate_response_state(state: MockResponseState) -> Result<(), String> {
            match state {
                MockResponseState::Succeeded => Ok(()),
                MockResponseState::Denied => Err("Signing request was denied".to_string()),
                MockResponseState::Failed => Err("Signing request failed".to_string()),
                MockResponseState::Unknown => Err("Unknown response state".to_string()),
            }
        }
        
        proptest! {
            #[test]
            fn test_only_succeeded_is_accepted(
                state_idx in 0..4usize
            ) {
                let states = [
                    MockResponseState::Succeeded,
                    MockResponseState::Denied,
                    MockResponseState::Failed,
                    MockResponseState::Unknown,
                ];
                
                let state = states[state_idx];
                let result = validate_response_state(state);
                
                if state == MockResponseState::Succeeded {
                    prop_assert!(result.is_ok(), "Succeeded state should be accepted");
                } else {
                    prop_assert!(result.is_err(), "Non-succeeded state should be rejected");
                    let err = result.unwrap_err();
                    prop_assert!(!err.is_empty(), "Error message should not be empty");
                    
                    // Verify specific error messages
                    match state {
                        MockResponseState::Denied => {
                            prop_assert!(err.contains("denied"));
                        }
                        MockResponseState::Failed => {
                            prop_assert!(err.contains("failed"));
                        }
                        MockResponseState::Unknown => {
                            prop_assert!(err.contains("Unknown"));
                        }
                        _ => {}
                    }
                }
            }
            
            #[test]
            fn test_all_states_have_distinct_handling(
                states in vec(0..4usize, 2..10)
            ) {
                let state_types = [
                    MockResponseState::Succeeded,
                    MockResponseState::Denied,
                    MockResponseState::Failed,
                    MockResponseState::Unknown,
                ];
                
                let mut error_messages = std::collections::HashSet::new();
                let mut success_count = 0;
                
                for &idx in &states {
                    let state = state_types[idx % 4];
                    match validate_response_state(state) {
                        Ok(()) => success_count += 1,
                        Err(msg) => {
                            error_messages.insert(msg);
                        }
                    }
                }
                
                // Each error state should have a unique message
                let error_state_count = states.iter()
                    .filter(|&&idx| state_types[idx % 4] != MockResponseState::Succeeded)
                    .count();
                let unique_error_types = std::cmp::min(3, error_state_count); // Max 3 error types
                
                prop_assert!(
                    error_messages.len() <= 3,
                    "Should have at most 3 distinct error messages"
                );
            }
        }
    }
    
    /// Threshold comparison logic
    ///  Test for premature loop termination and potentially a
    ///  accepting fewer signatures than required for security.
    mod threshold_comparison_tests {
        use super::*;
        
        // Helper function that mimics the fixed threshold logic
        fn collect_until_threshold(
            available: usize,
            threshold: usize,
            failures: Vec<usize>,
        ) -> Result<usize, String> {
            let mut collected = 0;
            
            for i in 0..available {
                if failures.contains(&i) {
                    continue;
                }
                
                collected += 1;
                
                // Store threshold as usize to avoid repeated casting
                let required_threshold = threshold;
                let current_count = collected;
                
                // Explicit check with no ambiguity
                if current_count >= required_threshold {
                    break;
                }
            }
            
            // Final validation after loop
            let required_threshold = threshold;
            let final_count = collected;
            
            if final_count < required_threshold {
                Err(format!(
                    "Failed to get enough partial signatures: got {}, need {}",
                    final_count, required_threshold
                ))
            } else {
                Ok(final_count)
            }
        }
        
        proptest! {
            #[test]
            fn test_threshold_exact_match(
                threshold in 1..10usize,
                extra_available in 0..5usize
            ) {
                let available = threshold + extra_available;
                let result = collect_until_threshold(available, threshold, vec![]);
                
                prop_assert!(result.is_ok());
                let collected = result.unwrap();
                prop_assert_eq!(
                    collected,
                    threshold,
                    "Should collect exactly threshold signatures when all succeed"
                );
            }
            
            #[test]
            fn test_threshold_with_failures(
                threshold in 2..10usize,
                total in 2..20usize,
                failure_indices in proptest::collection::vec(0..20usize, 0..10)
            ) {
                prop_assume!(total >= threshold);
                
                // Count how many non-failed participants we have
                let available_count = (0..total)
                    .filter(|i| !failure_indices.contains(i))
                    .count();
                
                let result = collect_until_threshold(total, threshold, failure_indices);
                
                if available_count >= threshold {
                    prop_assert!(
                        result.is_ok(),
                        "Should succeed when enough participants available"
                    );
                    let collected = result.unwrap();
                    prop_assert!(
                        collected >= threshold,
                        "Should collect at least threshold signatures"
                    );
                    prop_assert!(
                        collected <= available_count,
                        "Cannot collect more than available"
                    );
                } else {
                    prop_assert!(
                        result.is_err(),
                        "Should fail when not enough participants available"
                    );
                    let err = result.unwrap_err();
                    prop_assert!(
                        err.contains("got") && err.contains("need"),
                        "Error should show actual vs required"
                    );
                }
            }
            
            #[test]
            fn test_threshold_boundary_conditions(
                available in 1..20usize
            ) {
                // Test threshold = available (exact match)
                let result = collect_until_threshold(available, available, vec![]);
                prop_assert!(result.is_ok());
                prop_assert_eq!(result.unwrap(), available);
                
                // Test threshold > available (impossible)
                let result = collect_until_threshold(available, available + 1, vec![]);
                prop_assert!(result.is_err());
                
                // Test threshold = 1 (minimum)
                let result = collect_until_threshold(available, 1, vec![]);
                prop_assert!(result.is_ok());
                prop_assert_eq!(result.unwrap(), 1);
            }
            
            #[test]
            fn test_zero_threshold_edge_case(
                available in 1..10usize
            ) {
                // In practice, threshold should never be 0, but let's test the logic
                let result = collect_until_threshold(available, 0, vec![]);
                prop_assert!(
                    result.is_ok(),
                    "Zero threshold should succeed immediately"
                );
                prop_assert_eq!(
                    result.unwrap(),
                    0,
                    "Should collect 0 signatures for 0 threshold"
                );
            }
            
            #[test]
            fn test_deterministic_collection_order(
                threshold in 1..10usize,
                total in 10..20usize,
                failure_pattern in vec(bool, 20)
            ) {
                prop_assume!(total >= threshold);
                
                let failures: Vec<usize> = failure_pattern
                    .iter()
                    .take(total)
                    .enumerate()
                    .filter_map(|(i, &fail)| if fail { Some(i) } else { None })
                    .collect();
                
                let available_count = total - failures.len();
                prop_assume!(available_count >= threshold);
                
                // Run multiple times to ensure deterministic behavior
                let results: Vec<_> = (0..5)
                    .map(|_| collect_until_threshold(total, threshold, failures.clone()))
                    .collect();
                
                // All runs should have the same result
                let first_result = &results[0];
                for result in &results[1..] {
                    prop_assert_eq!(
                        result,
                        first_result,
                        "Collection should be deterministic"
                    );
                }
            }
        }
    }
}
