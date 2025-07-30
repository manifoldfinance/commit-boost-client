use xga_commitment::config::XGAConfig;
use xga_commitment::eigenlayer::EigenLayerConfig;

#[test]
fn test_webhook_port_minimum_boundary() {
    // Test port < 1024 (should fail)
    let config = XGAConfig {
        webhook_port: 1023,
        xga_relays: vec!["https://relay.example.com".to_string()],
        commitment_delay_ms: 100,
        retry_attempts: 3,
        retry_delay_ms: 1000,
        max_registration_age_secs: 60,
        probe_relay_capabilities: false,
        eigenlayer: EigenLayerConfig::default(),
    };

    let result = config.validate();
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Webhook port must be >= 1024"));
}

#[test]
fn test_webhook_port_exact_minimum() {
    // Test port == 1024 (should pass)
    let config = XGAConfig {
        webhook_port: 1024,
        xga_relays: vec!["https://relay.example.com".to_string()],
        commitment_delay_ms: 100,
        retry_attempts: 3,
        retry_delay_ms: 1000,
        max_registration_age_secs: 60,
        probe_relay_capabilities: false,
        eigenlayer: EigenLayerConfig::default(),
    };

    let result = config.validate();
    if let Err(e) = &result {
        println!("Validation error: {}", e);
    }
    assert!(result.is_ok());
}

#[test]
fn test_commitment_delay_zero() {
    // Test commitment_delay_ms < 1 (should fail)
    let config = XGAConfig {
        webhook_port: 8080,
        xga_relays: vec!["https://relay.example.com".to_string()],
        commitment_delay_ms: 0,
        retry_attempts: 3,
        retry_delay_ms: 1000,
        max_registration_age_secs: 60,
        probe_relay_capabilities: false,
        eigenlayer: EigenLayerConfig::default(),
    };

    let result = config.validate();
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Commitment delay must be between 1ms and 60 seconds"));
}

#[test]
fn test_commitment_delay_minimum() {
    // Test commitment_delay_ms == 1 (should pass)
    let config = XGAConfig {
        webhook_port: 8080,
        xga_relays: vec!["https://relay.example.com".to_string()],
        commitment_delay_ms: 1,
        retry_attempts: 3,
        retry_delay_ms: 1000,
        max_registration_age_secs: 60,
        probe_relay_capabilities: false,
        eigenlayer: EigenLayerConfig::default(),
    };

    assert!(config.validate().is_ok());
}

#[test]
fn test_commitment_delay_maximum() {
    // Test commitment_delay_ms == 60000 (should pass)
    let config = XGAConfig {
        webhook_port: 8080,
        xga_relays: vec!["https://relay.example.com".to_string()],
        commitment_delay_ms: 60000,
        retry_attempts: 3,
        retry_delay_ms: 1000,
        max_registration_age_secs: 60,
        probe_relay_capabilities: false,
        eigenlayer: EigenLayerConfig::default(),
    };

    assert!(config.validate().is_ok());
}

#[test]
fn test_commitment_delay_over_maximum() {
    // Test commitment_delay_ms > 60000 (should fail)
    let config = XGAConfig {
        webhook_port: 8080,
        xga_relays: vec!["https://relay.example.com".to_string()],
        commitment_delay_ms: 60001,
        retry_attempts: 3,
        retry_delay_ms: 1000,
        max_registration_age_secs: 60,
        probe_relay_capabilities: false,
        eigenlayer: EigenLayerConfig::default(),
    };

    let result = config.validate();
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Commitment delay must be between 1ms and 60 seconds"));
}

#[test]
fn test_retry_delay_below_minimum() {
    // Test retry_delay_ms < 100 (should fail)
    let config = XGAConfig {
        webhook_port: 8080,
        xga_relays: vec!["https://relay.example.com".to_string()],
        commitment_delay_ms: 100,
        retry_attempts: 3,
        retry_delay_ms: 99,
        max_registration_age_secs: 60,
        probe_relay_capabilities: false,
        eigenlayer: EigenLayerConfig::default(),
    };

    let result = config.validate();
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Retry delay must be between 100ms and 60 seconds"));
}

#[test]
fn test_retry_delay_minimum() {
    // Test retry_delay_ms == 100 (should pass)
    let config = XGAConfig {
        webhook_port: 8080,
        xga_relays: vec!["https://relay.example.com".to_string()],
        commitment_delay_ms: 100,
        retry_attempts: 3,
        retry_delay_ms: 100,
        max_registration_age_secs: 60,
        probe_relay_capabilities: false,
        eigenlayer: EigenLayerConfig::default(),
    };

    assert!(config.validate().is_ok());
}

#[test]
fn test_retry_delay_maximum() {
    // Test retry_delay_ms == 60000 (should pass)
    let config = XGAConfig {
        webhook_port: 8080,
        xga_relays: vec!["https://relay.example.com".to_string()],
        commitment_delay_ms: 100,
        retry_attempts: 3,
        retry_delay_ms: 60000,
        max_registration_age_secs: 60,
        probe_relay_capabilities: false,
        eigenlayer: EigenLayerConfig::default(),
    };

    assert!(config.validate().is_ok());
}

#[test]
fn test_retry_delay_over_maximum() {
    // Test retry_delay_ms > 60000 (should fail)
    let config = XGAConfig {
        webhook_port: 8080,
        xga_relays: vec!["https://relay.example.com".to_string()],
        commitment_delay_ms: 100,
        retry_attempts: 3,
        retry_delay_ms: 60001,
        max_registration_age_secs: 60,
        probe_relay_capabilities: false,
        eigenlayer: EigenLayerConfig::default(),
    };

    let result = config.validate();
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Retry delay must be between 100ms and 60 seconds"));
}

#[test]
fn test_retry_attempts_zero() {
    // Test retry_attempts < 1 (should fail)
    let config = XGAConfig {
        webhook_port: 8080,
        xga_relays: vec!["https://relay.example.com".to_string()],
        commitment_delay_ms: 100,
        retry_attempts: 0,
        retry_delay_ms: 1000,
        max_registration_age_secs: 60,
        probe_relay_capabilities: false,
        eigenlayer: EigenLayerConfig::default(),
    };

    let result = config.validate();
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Retry attempts must be between 1 and 10"));
}

#[test]
fn test_retry_attempts_minimum() {
    // Test retry_attempts == 1 (should pass)
    let config = XGAConfig {
        webhook_port: 8080,
        xga_relays: vec!["https://relay.example.com".to_string()],
        commitment_delay_ms: 100,
        retry_attempts: 1,
        retry_delay_ms: 1000,
        max_registration_age_secs: 60,
        probe_relay_capabilities: false,
        eigenlayer: EigenLayerConfig::default(),
    };

    assert!(config.validate().is_ok());
}

#[test]
fn test_retry_attempts_maximum() {
    // Test retry_attempts == 10 (should pass)
    let config = XGAConfig {
        webhook_port: 8080,
        xga_relays: vec!["https://relay.example.com".to_string()],
        commitment_delay_ms: 100,
        retry_attempts: 10,
        retry_delay_ms: 1000,
        max_registration_age_secs: 60,
        probe_relay_capabilities: false,
        eigenlayer: EigenLayerConfig::default(),
    };

    assert!(config.validate().is_ok());
}

#[test]
fn test_retry_attempts_over_maximum() {
    // Test retry_attempts > 10 (should fail)
    let config = XGAConfig {
        webhook_port: 8080,
        xga_relays: vec!["https://relay.example.com".to_string()],
        commitment_delay_ms: 100,
        retry_attempts: 11,
        retry_delay_ms: 1000,
        max_registration_age_secs: 60,
        probe_relay_capabilities: false,
        eigenlayer: EigenLayerConfig::default(),
    };

    let result = config.validate();
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Retry attempts must be between 1 and 10"));
}

#[test]
fn test_max_registration_age_zero() {
    // Test max_registration_age_secs < 1 (should fail)
    let config = XGAConfig {
        webhook_port: 8080,
        xga_relays: vec!["https://relay.example.com".to_string()],
        commitment_delay_ms: 100,
        retry_attempts: 3,
        retry_delay_ms: 1000,
        max_registration_age_secs: 0,
        probe_relay_capabilities: false,
        eigenlayer: EigenLayerConfig::default(),
    };

    let result = config.validate();
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Max registration age must be between 1 second and 10 minutes"));
}

#[test]
fn test_max_registration_age_minimum() {
    // Test max_registration_age_secs == 1 (should pass)
    let config = XGAConfig {
        webhook_port: 8080,
        xga_relays: vec!["https://relay.example.com".to_string()],
        commitment_delay_ms: 100,
        retry_attempts: 3,
        retry_delay_ms: 1000,
        max_registration_age_secs: 1,
        probe_relay_capabilities: false,
        eigenlayer: EigenLayerConfig::default(),
    };

    assert!(config.validate().is_ok());
}

#[test]
fn test_max_registration_age_maximum() {
    // Test max_registration_age_secs == 600 (should pass)
    let config = XGAConfig {
        webhook_port: 8080,
        xga_relays: vec!["https://relay.example.com".to_string()],
        commitment_delay_ms: 100,
        retry_attempts: 3,
        retry_delay_ms: 1000,
        max_registration_age_secs: 600,
        probe_relay_capabilities: false,
        eigenlayer: EigenLayerConfig::default(),
    };

    assert!(config.validate().is_ok());
}

#[test]
fn test_max_registration_age_over_maximum() {
    // Test max_registration_age_secs > 600 (should fail)
    let config = XGAConfig {
        webhook_port: 8080,
        xga_relays: vec!["https://relay.example.com".to_string()],
        commitment_delay_ms: 100,
        retry_attempts: 3,
        retry_delay_ms: 1000,
        max_registration_age_secs: 601,
        probe_relay_capabilities: false,
        eigenlayer: EigenLayerConfig::default(),
    };

    let result = config.validate();
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Max registration age must be between 1 second and 10 minutes"));
}

#[test]
fn test_no_xga_relays() {
    // Test empty xga_relays (should fail)
    let config = XGAConfig {
        webhook_port: 8080,
        xga_relays: vec![],
        commitment_delay_ms: 100,
        retry_attempts: 3,
        retry_delay_ms: 1000,
        max_registration_age_secs: 60,
        probe_relay_capabilities: false,
        eigenlayer: EigenLayerConfig::default(),
    };

    let result = config.validate();
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("At least one XGA relay must be configured"));
}

#[test]
fn test_invalid_relay_url() {
    // Test invalid URL format
    let config = XGAConfig {
        webhook_port: 8080,
        xga_relays: vec!["not-a-valid-url".to_string()],
        commitment_delay_ms: 100,
        retry_attempts: 3,
        retry_delay_ms: 1000,
        max_registration_age_secs: 60,
        probe_relay_capabilities: false,
        eigenlayer: EigenLayerConfig::default(),
    };

    let result = config.validate();
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Invalid XGA relay URL"));
}

#[test]
fn test_is_xga_relay_found() {
    let config = XGAConfig {
        webhook_port: 8080,
        xga_relays: vec![
            "https://relay1.example.com".to_string(),
            "https://relay2.example.com".to_string(),
        ],
        commitment_delay_ms: 100,
        retry_attempts: 3,
        retry_delay_ms: 1000,
        max_registration_age_secs: 60,
        probe_relay_capabilities: false,
        eigenlayer: EigenLayerConfig::default(),
    };

    assert!(config.is_xga_relay("https://relay1.example.com"));
    assert!(config.is_xga_relay("https://relay2.example.com"));
}

#[test]
fn test_is_xga_relay_not_found() {
    let config = XGAConfig {
        webhook_port: 8080,
        xga_relays: vec![
            "https://relay1.example.com".to_string(),
            "https://relay2.example.com".to_string(),
        ],
        commitment_delay_ms: 100,
        retry_attempts: 3,
        retry_delay_ms: 1000,
        max_registration_age_secs: 60,
        probe_relay_capabilities: false,
        eigenlayer: EigenLayerConfig::default(),
    };

    assert!(!config.is_xga_relay("https://unknown.example.com"));
    assert!(!config.is_xga_relay("https://relay1.example.com:8080")); // Different port
}

// Test default values
#[test]
fn test_default_commitment_delay() {
    use serde_json::json;

    let config_json = json!({
        "webhook_port": 8080,
        "xga_relays": ["https://relay.example.com"]
    });

    let config: XGAConfig = serde_json::from_value(config_json).unwrap();
    assert_eq!(config.commitment_delay_ms, 100);
}

#[test]
fn test_default_retry_attempts() {
    use serde_json::json;

    let config_json = json!({
        "webhook_port": 8080,
        "xga_relays": ["https://relay.example.com"]
    });

    let config: XGAConfig = serde_json::from_value(config_json).unwrap();
    assert_eq!(config.retry_attempts, 3);
}

#[test]
fn test_default_retry_delay() {
    use serde_json::json;

    let config_json = json!({
        "webhook_port": 8080,
        "xga_relays": ["https://relay.example.com"]
    });

    let config: XGAConfig = serde_json::from_value(config_json).unwrap();
    assert_eq!(config.retry_delay_ms, 1000);
}

#[test]
fn test_default_max_registration_age() {
    use serde_json::json;

    let config_json = json!({
        "webhook_port": 8080,
        "xga_relays": ["https://relay.example.com"]
    });

    let config: XGAConfig = serde_json::from_value(config_json).unwrap();
    assert_eq!(config.max_registration_age_secs, 60);
}

#[test]
fn test_default_probe_relay_capabilities() {
    use serde_json::json;

    let config_json = json!({
        "webhook_port": 8080,
        "xga_relays": ["https://relay.example.com"]
    });

    let config: XGAConfig = serde_json::from_value(config_json).unwrap();
    assert_eq!(config.probe_relay_capabilities, false);
}

// Test compound conditions
#[test]
fn test_validate_all_fields_at_minimum() {
    let config = XGAConfig {
        webhook_port: 1024,
        xga_relays: vec!["https://relay.example.com".to_string()],
        commitment_delay_ms: 1,
        retry_attempts: 1,
        retry_delay_ms: 100,
        max_registration_age_secs: 1,
        probe_relay_capabilities: false,
        eigenlayer: EigenLayerConfig::default(),
    };

    assert!(config.validate().is_ok());
}

#[test]
fn test_validate_all_fields_at_maximum() {
    let config = XGAConfig {
        webhook_port: 65535,
        xga_relays: vec!["https://relay.example.com".to_string()],
        commitment_delay_ms: 60000,
        retry_attempts: 10,
        retry_delay_ms: 60000,
        max_registration_age_secs: 600,
        probe_relay_capabilities: true,
        eigenlayer: EigenLayerConfig::default(),
    };

    assert!(config.validate().is_ok());
}
