use alloy::primitives::Address;
use commit_boost::prelude::*;
use std::sync::Arc;

use crate::config::{XGAConfig, RetryConfig};
use crate::eigenlayer::EigenLayerConfig;

/// Create a test configuration for EigenLayer
pub fn test_eigenlayer_config() -> EigenLayerConfig {
    EigenLayerConfig {
        enabled: true,
        registry_address: "0x1234567890123456789012345678901234567890".to_string(),
        rpc_url: "https://eth-mainnet.example.com".to_string(),
        operator_address: Some("0xabcdefabcdefabcdefabcdefabcdefabcdefabcd".to_string()),
    }
}

/// Create a test CB config - simplified mock for testing
pub fn test_cb_config() -> Arc<StartCommitModuleConfig<XGAConfig>> {
    // For testing purposes, we create a mock config. In real usage, this would be loaded
    // via load_commit_module_config(). Since we can't easily construct all the internal
    // types, we'll use a different approach for unit tests.
    
    // Create a minimal XGAConfig for testing
    let _xga_config = XGAConfig {
        polling_interval_secs: 5,
        xga_relays: vec!["https://relay.example.com".to_string()],
        max_registration_age_secs: 60,
        probe_relay_capabilities: false,
        retry_config: RetryConfig::default(),
        eigenlayer: test_eigenlayer_config(),
    };
    
    // Note: In actual tests that need StartCommitModuleConfig, we should mock the
    // specific functionality needed rather than trying to construct the full object.
    // For now, we'll panic to indicate this needs proper mocking in tests.
    panic!("test_cb_config should not be used directly - mock specific functionality instead")
}

/// Test operator address
pub fn test_operator_address() -> Address {
    Address::from([0xab; 20])
}

/// Create a test XGA config
pub fn test_xga_config() -> XGAConfig {
    XGAConfig {
        polling_interval_secs: 5,
        xga_relays: vec!["https://relay.example.com".to_string()],
        max_registration_age_secs: 60,
        probe_relay_capabilities: false,
        retry_config: RetryConfig::default(),
        eigenlayer: test_eigenlayer_config(),
    }
}