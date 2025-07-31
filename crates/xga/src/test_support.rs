use alloy::primitives::Address;
use commit_boost::prelude::*;

use crate::config::{XgaConfig, RetryConfig};
use crate::eigenlayer::EigenLayerConfig;

/// Mock implementation for StartCommitModuleConfig
/// This is used in tests where we need to simulate the Commit-Boost environment
pub struct MockModuleConfig {
    pub id: String,
    pub chain_id: u64,
    pub extra: XgaConfig,
}

impl MockModuleConfig {
    /// Create a new mock configuration for testing
    #[must_use]
    pub fn new(extra: XgaConfig) -> Self {
        Self {
            id: "test-xga-module".to_string(),
            chain_id: 1, // Mainnet
            extra,
        }
    }
    
    /// Get the module ID
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }
    
    /// Get the chain configuration
    #[must_use]
    pub fn chain(&self) -> Chain {
        Chain::Mainnet
    }
    
    /// Get the extra configuration
    #[must_use]
    pub fn extra(&self) -> &XgaConfig {
        &self.extra
    }
}

/// Create a test configuration for EigenLayer
#[must_use]
pub fn test_eigenlayer_config() -> EigenLayerConfig {
    EigenLayerConfig {
        enabled: false, // Disabled by default in tests
        registry_address: "0x1234567890123456789012345678901234567890".to_string(),
        rpc_url: "https://eth-mainnet.example.com".to_string(),
        operator_address: None,
    }
}

/// Create a minimal test XGA config
#[must_use]
pub fn test_xga_config() -> XgaConfig {
    XgaConfig {
        polling_interval_secs: 5,
        xga_relays: vec!["https://relay.example.com".to_string()],
        max_registration_age_secs: 60,
        probe_relay_capabilities: false,
        retry_config: RetryConfig::default(),
        eigenlayer: test_eigenlayer_config(),
        validator_rate_limit: Default::default(),
    }
}

/// Create a test configuration with custom relays
#[must_use]
pub fn test_xga_config_with_relays(relays: Vec<String>) -> XgaConfig {
    XgaConfig {
        polling_interval_secs: 5,
        xga_relays: relays,
        max_registration_age_secs: 60,
        probe_relay_capabilities: false,
        retry_config: RetryConfig::default(),
        eigenlayer: test_eigenlayer_config(),
        validator_rate_limit: Default::default(),
    }
}

/// Test operator address
#[must_use]
#[inline]
pub const fn test_operator_address() -> Address {
    Address::new([0xab; 20])
}

/// Builder for creating test configurations with specific properties
pub struct TestConfigBuilder {
    polling_interval_secs: u64,
    xga_relays: Vec<String>,
    max_registration_age_secs: u64,
    probe_relay_capabilities: bool,
    retry_config: RetryConfig,
    eigenlayer: EigenLayerConfig,
}

impl Default for TestConfigBuilder {
    fn default() -> Self {
        Self {
            polling_interval_secs: 5,
            xga_relays: vec!["https://relay.example.com".to_string()],
            max_registration_age_secs: 60,
            probe_relay_capabilities: false,
            retry_config: RetryConfig::default(),
            eigenlayer: test_eigenlayer_config(),
        }
    }
}

impl TestConfigBuilder {
    /// Create a new test config builder
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
    
    /// Set the polling interval
    #[must_use]
    pub fn with_polling_interval(mut self, secs: u64) -> Self {
        self.polling_interval_secs = secs;
        self
    }
    
    /// Set the relay URLs
    #[must_use]
    pub fn with_relays(mut self, relays: Vec<String>) -> Self {
        self.xga_relays = relays;
        self
    }
    
    /// Set the max registration age
    #[must_use]
    pub fn with_max_registration_age(mut self, secs: u64) -> Self {
        self.max_registration_age_secs = secs;
        self
    }
    
    /// Enable relay capability probing
    #[must_use]
    pub fn with_relay_probing(mut self, enabled: bool) -> Self {
        self.probe_relay_capabilities = enabled;
        self
    }
    
    /// Set retry configuration
    #[must_use]
    pub fn with_retry_config(mut self, config: RetryConfig) -> Self {
        self.retry_config = config;
        self
    }
    
    /// Enable EigenLayer with custom config
    #[must_use]
    pub fn with_eigenlayer(mut self, config: EigenLayerConfig) -> Self {
        self.eigenlayer = config;
        self
    }
    
    /// Build the configuration
    #[must_use]
    pub fn build(self) -> XgaConfig {
        XgaConfig {
            polling_interval_secs: self.polling_interval_secs,
            xga_relays: self.xga_relays,
            max_registration_age_secs: self.max_registration_age_secs,
            probe_relay_capabilities: self.probe_relay_capabilities,
            retry_config: self.retry_config,
            eigenlayer: self.eigenlayer,
            validator_rate_limit: Default::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_mock_module_config() {
        let xga_config = test_xga_config();
        let mock = MockModuleConfig::new(xga_config.clone());
        
        assert_eq!(mock.id(), "test-xga-module");
        assert_eq!(mock.chain_id, 1);
        assert_eq!(mock.extra().polling_interval_secs, xga_config.polling_interval_secs);
    }
    
    #[test]
    fn test_config_builder() {
        let config = TestConfigBuilder::new()
            .with_polling_interval(10)
            .with_relays(vec!["https://relay1.com".to_string(), "https://relay2.com".to_string()])
            .with_max_registration_age(120)
            .with_relay_probing(true)
            .build();
            
        assert_eq!(config.polling_interval_secs, 10);
        assert_eq!(config.xga_relays.len(), 2);
        assert_eq!(config.max_registration_age_secs, 120);
        assert!(config.probe_relay_capabilities);
    }
    
    #[test]
    fn test_eigenlayer_config_disabled_by_default() {
        let config = test_eigenlayer_config();
        assert!(!config.enabled);
        assert!(config.operator_address.is_none());
    }
}