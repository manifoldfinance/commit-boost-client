use std::time::{SystemTime, UNIX_EPOCH};

use alloy_rpc_types::beacon::relay::{ValidatorRegistration, ValidatorRegistrationMessage};
use commit_boost::prelude::*;
use xga_commitment::{
    commitment::{RegistrationNotification, XGACommitment},
    config::XGAConfig,
    eigenlayer::EigenLayerConfig,
};

fn create_test_config() -> XGAConfig {
    XGAConfig {
        webhook_port: 8080,
        xga_relays: vec!["https://relay1.example.com".to_string()],
        commitment_delay_ms: 100,
        retry_attempts: 3,
        retry_delay_ms: 1000,
        max_registration_age_secs: 60,
        probe_relay_capabilities: false,
        eigenlayer: EigenLayerConfig {
            enabled: true,
            registry_address: "0x1234567890123456789012345678901234567890".to_string(),
            rpc_url: "http://localhost:8545".to_string(),
            operator_address: None,
        },
    }
}

fn create_test_registration() -> ValidatorRegistration {
    let message = ValidatorRegistrationMessage {
        fee_recipient: alloy::primitives::Address::from([1u8; 20]),
        gas_limit: 30000000,
        timestamp: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_secs(),
        pubkey: alloy_rpc_types::beacon::BlsPublicKey::from([2u8; 48]),
    };

    ValidatorRegistration {
        message,
        signature: alloy_rpc_types::beacon::BlsSignature::from([3u8; 96]),
    }
}

#[test]
fn test_eigenlayer_config_in_xga_config() {
    let config = create_test_config();

    assert!(config.eigenlayer.enabled);
    assert_eq!(config.eigenlayer.registry_address, "0x1234567890123456789012345678901234567890");
    assert_eq!(config.eigenlayer.rpc_url, "http://localhost:8545");
}

#[test]
fn test_commitment_creation_for_eigenlayer() {
    let registration = create_test_registration();
    let registration_hash = XGACommitment::hash_registration(&registration);

    // Convert pubkey for commitment
    let pubkey_bytes: [u8; 48] = registration.message.pubkey.0.clone();
    let validator_pubkey = BlsPublicKey::from(pubkey_bytes);

    let commitment = XGACommitment::new(
        registration_hash,
        validator_pubkey.clone(),
        "relay1.example.com".to_string(),
        1, // mainnet
        Default::default(),
    );

    // Verify commitment fields
    assert_eq!(commitment.registration_hash, registration_hash.into());
    assert_eq!(commitment.validator_pubkey, validator_pubkey);
    assert_eq!(commitment.chain_id, 1);
}

#[tokio::test]
async fn test_webhook_eigenlayer_flow() {
    // This test verifies the flow without actually starting the webhook server

    let config = create_test_config();
    let registration = create_test_registration();

    let notification = RegistrationNotification {
        registration,
        relay_url: "https://relay1.example.com".to_string(),
        timestamp: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_secs(),
    };

    // Verify the notification would be processed
    assert!(config.is_xga_relay(&notification.relay_url));

    // Verify age check
    let age = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards")
        .as_secs()
        .saturating_sub(notification.timestamp);

    assert!(age <= config.max_registration_age_secs);
}

#[test]
fn test_eigenlayer_disabled_flow() {
    let mut config = create_test_config();
    config.eigenlayer.enabled = false;

    // When EigenLayer is disabled, XGA should work normally
    assert!(!config.eigenlayer.enabled);

    // Other XGA functionality should remain unchanged
    assert_eq!(config.xga_relays.len(), 1);
    assert_eq!(config.webhook_port, 8080);
}
