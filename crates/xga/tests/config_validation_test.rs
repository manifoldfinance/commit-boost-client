use xga_commitment::{config::{XgaConfig, RetryConfig}, eigenlayer::EigenLayerConfig};

#[test]
fn test_polling_interval_minimum_boundary() {
    // Test polling_interval < 1 (should fail)
    let config = XgaConfig {
        polling_interval_secs: 0,
        xga_relays: vec!["https://relay.example.com".to_string()],
        max_registration_age_secs: 60,
        probe_relay_capabilities: false,
        retry_config: RetryConfig::default(),
        eigenlayer: EigenLayerConfig::default(),
        validator_rate_limit: Default::default(),
    };

    let result = config.validate();
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Polling interval must be between 1 second and 1 hour"));
}

#[test]
fn test_polling_interval_maximum_boundary() {
    // Test polling_interval > 3600 (should fail)
    let config = XgaConfig {
        polling_interval_secs: 3601,
        xga_relays: vec!["https://relay.example.com".to_string()],
        max_registration_age_secs: 60,
        probe_relay_capabilities: false,
        retry_config: RetryConfig::default(),
        eigenlayer: EigenLayerConfig::default(),
        validator_rate_limit: Default::default(),
    };

    let result = config.validate();
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Polling interval must be between 1 second and 1 hour"));
}

#[test]
fn test_polling_interval_valid() {
    // Test valid polling interval
    let config = XgaConfig {
        polling_interval_secs: 5,
        xga_relays: vec!["https://relay.example.com".to_string()],
        max_registration_age_secs: 60,
        probe_relay_capabilities: false,
        retry_config: RetryConfig::default(),
        eigenlayer: EigenLayerConfig::default(),
        validator_rate_limit: Default::default(),
    };

    let result = config.validate();
    assert!(result.is_ok());
}

#[test]
fn test_max_registration_age_zero() {
    // Test max_registration_age_secs < 1 (should fail)
    let config = XgaConfig {
        polling_interval_secs: 5,
        xga_relays: vec!["https://relay.example.com".to_string()],
        max_registration_age_secs: 0,
        probe_relay_capabilities: false,
        retry_config: RetryConfig::default(),
        eigenlayer: EigenLayerConfig::default(),
        validator_rate_limit: Default::default(),
    };

    let result = config.validate();
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Max registration age must be between 1 second and 10 minutes"));
}

#[test]
fn test_max_registration_age_too_high() {
    // Test max_registration_age_secs > 600 (should fail)
    let config = XgaConfig {
        polling_interval_secs: 5,
        xga_relays: vec!["https://relay.example.com".to_string()],
        max_registration_age_secs: 601,
        probe_relay_capabilities: false,
        retry_config: RetryConfig::default(),
        eigenlayer: EigenLayerConfig::default(),
        validator_rate_limit: Default::default(),
    };

    let result = config.validate();
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Max registration age must be between 1 second and 10 minutes"));
}

#[test]
fn test_retry_config_max_retries_reasonable() {
    // Test reasonable retry config
    let config = XgaConfig {
        polling_interval_secs: 5,
        xga_relays: vec!["https://relay.example.com".to_string()],
        max_registration_age_secs: 60,
        probe_relay_capabilities: false,
        retry_config: RetryConfig {
            max_retries: 10,
            initial_backoff_ms: 100,
            max_backoff_secs: 60,
        },
        eigenlayer: EigenLayerConfig::default(),
        validator_rate_limit: Default::default(),
    };

    let result = config.validate();
    assert!(result.is_ok());
}

#[test]
fn test_relay_url_validation() {
    // Test invalid relay URL (HTTP instead of HTTPS)
    let config = XgaConfig {
        polling_interval_secs: 5,
        xga_relays: vec!["http://relay.example.com".to_string()],
        max_registration_age_secs: 60,
        probe_relay_capabilities: false,
        retry_config: RetryConfig::default(),
        eigenlayer: EigenLayerConfig::default(),
        validator_rate_limit: Default::default(),
    };

    let result = config.validate();
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Invalid XGA relay URL"));
}

#[test]
fn test_empty_relays_invalid() {
    // Empty relay list should be invalid
    let config = XgaConfig {
        polling_interval_secs: 5,
        xga_relays: vec![],
        max_registration_age_secs: 60,
        probe_relay_capabilities: false,
        retry_config: RetryConfig::default(),
        eigenlayer: EigenLayerConfig::default(),
        validator_rate_limit: Default::default(),
    };

    let result = config.validate();
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("At least one XGA relay must be configured"));
}

// Removed test_eigenlayer_config_validation as EigenLayer config
// validation happens when creating the integration, not in config validation

#[test]
fn test_multiple_valid_relays() {
    // Test multiple valid relay URLs
    let config = XgaConfig {
        polling_interval_secs: 5,
        xga_relays: vec![
            "https://relay1.example.com".to_string(),
            "https://relay2.example.com:8080".to_string(),
            "https://relay3.example.com/api/v1".to_string(),
        ],
        max_registration_age_secs: 60,
        probe_relay_capabilities: false,
        retry_config: RetryConfig::default(),
        eigenlayer: EigenLayerConfig::default(),
        validator_rate_limit: Default::default(),
    };

    let result = config.validate();
    assert!(result.is_ok());
}