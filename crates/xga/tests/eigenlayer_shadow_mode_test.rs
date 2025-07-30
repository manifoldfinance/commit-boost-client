// Simple tests for shadow mode functionality
use commit_boost::prelude::*;
use xga_commitment::{
    commitment::{XGACommitment, XGAParameters},
    eigenlayer::EigenLayerConfig,
};

#[test]
fn test_eigenlayer_config_shadow_mode_only() {
    // Verify configuration no longer has shadow_mode or private_key_path
    let config = EigenLayerConfig {
        enabled: true,
        registry_address: "0x1234567890123456789012345678901234567890".to_string(),
        rpc_url: "http://localhost:8545".to_string(),
        operator_address: None,
    };

    assert!(config.enabled);
    assert_eq!(config.registry_address, "0x1234567890123456789012345678901234567890");
    assert_eq!(config.rpc_url, "http://localhost:8545");
}

#[test]
fn test_commitment_for_shadow_mode() {
    // Test that commitments work for shadow mode tracking
    let validator_pubkey = BlsPublicKey::from([1u8; 48]);
    let commitment = XGACommitment::new(
        [42u8; 32],
        validator_pubkey.clone(),
        "test-relay".to_string(),
        1,
        XGAParameters::default(),
    );

    // Verify commitment can be created and hashed
    let hash = commitment.get_tree_hash_root();
    assert_eq!(commitment.validator_pubkey, validator_pubkey);
    assert_eq!(commitment.chain_id, 1);

    // Hash should be deterministic
    let hash2 = commitment.get_tree_hash_root();
    assert_eq!(hash, hash2);
}

#[test]
fn test_config_defaults() {
    let config = EigenLayerConfig::default();
    assert!(!config.enabled);
    assert_eq!(config.registry_address, "");
    assert_eq!(config.rpc_url, "");
}
