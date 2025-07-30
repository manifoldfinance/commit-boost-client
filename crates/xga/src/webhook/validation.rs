use eyre::Result;

use crate::{
    commitment::RegistrationNotification,
    config::XGAConfig,
    infrastructure::{get_current_timestamp, parse_and_validate_url},
};

/// Minimum gas limit for validator registration
const MIN_GAS_LIMIT: u64 = 1_000_000;

/// Maximum gas limit for validator registration
const MAX_GAS_LIMIT: u64 = 50_000_000;

/// Maximum allowed clock skew in seconds
const MAX_CLOCK_SKEW_SECONDS: u64 = 5;

/// Validate incoming registration notification
pub fn validate_notification(
    notification: &RegistrationNotification,
    config: &XGAConfig,
) -> Result<()> {
    // Validate relay URL
    validate_relay_url(&notification.relay_url)?;

    // Check registration age
    let current_time = get_current_timestamp()?;

    let age = current_time.saturating_sub(notification.timestamp);
    if age > config.max_registration_age_secs {
        return Err(eyre::eyre!("Registration too old: {} seconds", age));
    }

    // Check for future timestamp (with tolerance for clock skew)
    if notification.timestamp > current_time + MAX_CLOCK_SKEW_SECONDS {
        return Err(eyre::eyre!("Registration timestamp is in the future"));
    }

    // Validate pubkey is not all zeros
    if notification.registration.message.pubkey.0 == [0u8; 48] {
        return Err(eyre::eyre!("Invalid validator public key: all zeros"));
    }

    // Validate fee recipient - check if it's the zero address
    if notification.registration.message.fee_recipient.0 == [0u8; 20] {
        return Err(eyre::eyre!("Invalid fee recipient: zero address"));
    }

    // Validate gas limit is within reasonable bounds
    let gas_limit = notification.registration.message.gas_limit;
    if gas_limit < MIN_GAS_LIMIT {
        return Err(eyre::eyre!("Gas limit too low: {} < {}", gas_limit, MIN_GAS_LIMIT));
    }
    if gas_limit > MAX_GAS_LIMIT {
        return Err(eyre::eyre!("Gas limit too high: {} > {}", gas_limit, MAX_GAS_LIMIT));
    }

    // Validate signature is not all zeros
    if notification.registration.signature.0 == [0u8; 96] {
        return Err(eyre::eyre!("Invalid signature: all zeros"));
    }

    Ok(())
}

/// Validate that a relay URL is properly formatted and uses HTTPS
pub fn validate_relay_url(relay_url: &str) -> Result<()> {
    // Use safe URL parsing from infrastructure module
    parse_and_validate_url(relay_url)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use alloy_rpc_types::beacon::relay::{ValidatorRegistration, ValidatorRegistrationMessage};

    use super::*;

    fn create_valid_registration() -> ValidatorRegistration {
        ValidatorRegistration {
            message: ValidatorRegistrationMessage {
                pubkey: alloy_rpc_types::beacon::BlsPublicKey::from([0x01u8; 48]),
                fee_recipient: alloy::primitives::Address::from([0x02u8; 20]),
                gas_limit: 30_000_000,
                timestamp: get_current_timestamp()
                    .expect("Failed to get timestamp for test")
                    .saturating_sub(10), // 10 seconds ago
            },
            signature: alloy_rpc_types::beacon::BlsSignature::from([0x03u8; 96]),
        }
    }

    fn create_test_config() -> XGAConfig {
        XGAConfig {
            max_registration_age_secs: 300,
            webhook_port: 8080,
            commitment_delay_ms: 0,
            xga_relays: vec![],
            retry_attempts: 3,
            retry_delay_ms: 1000,
            probe_relay_capabilities: false,
            eigenlayer: crate::eigenlayer::EigenLayerConfig::default(),
        }
    }

    #[test]
    fn test_validate_relay_url() {
        // Valid HTTPS URLs
        assert!(validate_relay_url("https://relay.example.com").is_ok());
        assert!(validate_relay_url("https://relay.example.com:8080").is_ok());
        assert!(validate_relay_url("https://relay.example.com/api/v1").is_ok());

        // Invalid: HTTP instead of HTTPS
        assert!(validate_relay_url("http://relay.example.com").is_err());
        let err =
            validate_relay_url("http://relay.example.com").expect_err("Should fail for HTTP URL");
        assert!(err.to_string().contains("HTTPS"));

        // Invalid: localhost and local IPs
        assert!(validate_relay_url("https://localhost").is_err());
        assert!(validate_relay_url("https://127.0.0.1").is_err());
        assert!(validate_relay_url("https://0.0.0.0").is_err());
        assert!(validate_relay_url("https://192.168.1.1").is_err());
        assert!(validate_relay_url("https://10.0.0.1").is_err());

        // Invalid: malformed URLs
        assert!(validate_relay_url("not-a-url").is_err());
        assert!(validate_relay_url("").is_err());
        assert!(validate_relay_url("https://").is_err());
    }

    #[test]
    fn test_validate_notification() {
        let valid_registration = create_valid_registration();
        let valid_notification = RegistrationNotification {
            registration: valid_registration.clone(),
            relay_url: "https://relay.example.com".to_string(),
            timestamp: valid_registration.message.timestamp,
        };

        let config = create_test_config();

        // Valid notification should pass
        assert!(validate_notification(&valid_notification, &config).is_ok());

        // Test invalid relay URL
        let mut invalid_url = valid_notification.clone();
        invalid_url.relay_url = "http://relay.example.com".to_string();
        assert!(validate_notification(&invalid_url, &config).is_err());

        // Test too old registration
        let mut old_notification = valid_notification.clone();
        old_notification.timestamp =
            get_current_timestamp().expect("Failed to get timestamp for test").saturating_sub(400); // 400 seconds ago
        assert!(validate_notification(&old_notification, &config).is_err());

        // Test future timestamp
        let mut future_notification = valid_notification.clone();
        future_notification.timestamp =
            get_current_timestamp().expect("Failed to get timestamp for test").saturating_add(10); // 10 seconds in future
        assert!(validate_notification(&future_notification, &config).is_err());

        // Test zero pubkey
        let mut zero_pubkey = valid_notification.clone();
        zero_pubkey.registration.message.pubkey =
            alloy_rpc_types::beacon::BlsPublicKey::from([0u8; 48]);
        assert!(validate_notification(&zero_pubkey, &config).is_err());

        // Test zero fee recipient
        let mut zero_fee = valid_notification.clone();
        zero_fee.registration.message.fee_recipient = alloy::primitives::Address::from([0u8; 20]);
        assert!(validate_notification(&zero_fee, &config).is_err());

        // Test gas limit too low
        let mut low_gas = valid_notification.clone();
        low_gas.registration.message.gas_limit = 500_000;
        assert!(validate_notification(&low_gas, &config).is_err());

        // Test gas limit too high
        let mut high_gas = valid_notification.clone();
        high_gas.registration.message.gas_limit = 60_000_000;
        assert!(validate_notification(&high_gas, &config).is_err());

        // Test zero signature
        let mut zero_sig = valid_notification.clone();
        zero_sig.registration.signature = alloy_rpc_types::beacon::BlsSignature::from([0u8; 96]);
        assert!(validate_notification(&zero_sig, &config).is_err());
    }
}
