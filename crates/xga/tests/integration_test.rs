use commit_boost::prelude::*;
use xga_commitment::commitment::{XgaCommitment, XgaParameters};

#[test]
fn test_commitment_hash_consistency() {
    // Create two identical commitments
    let registration_hash = [1u8; 32];
    let validator_pubkey = BlsPublicKey::from([2u8; 48]);
    let relay_id = "test-relay";
    let chain_id = 1;
    let params = XgaParameters::default();

    let commitment1 = XgaCommitment::new(
        registration_hash,
        validator_pubkey,
        relay_id,
        chain_id,
        params.clone(),
    );

    // Create a second commitment with same data but wait to ensure different
    // timestamp
    std::thread::sleep(std::time::Duration::from_secs(1));

    let commitment2 =
        XgaCommitment::new(registration_hash, validator_pubkey, relay_id, chain_id, params);

    // Timestamps should be different (in seconds)
    assert!(commitment2.timestamp >= commitment1.timestamp + 1);

    // The commitments should be different due to different timestamps
    // We can verify this by checking that their serialized forms are different
    let json1 = serde_json::to_string(&commitment1).expect("Failed to serialize commitment1");
    let json2 = serde_json::to_string(&commitment2).expect("Failed to serialize commitment2");

    assert_ne!(json1, json2); // Different due to timestamp
    assert_eq!(commitment1.chain_id, commitment2.chain_id);
    assert_eq!(commitment1.signing_domain, commitment2.signing_domain);
}

#[test]
fn test_xga_parameters_default() {
    let params = XgaParameters::default();
    assert_eq!(params.version, 1);
    assert_eq!(params.min_inclusion_slot, 0);
    assert_eq!(params.max_inclusion_slot, 0);
    assert_eq!(params.flags, 0);
}

#[test]
fn test_commitment_edge_cases() {
    // Test with empty relay ID
    let commitment = XgaCommitment::new(
        [0u8; 32],                     // zero registration hash
        BlsPublicKey::from([0u8; 48]), // zero pubkey
        "",                            // empty relay ID
        0,                             // chain ID 0
        XgaParameters::default(),
    );

    // Should still be valid, even with edge case values
    assert_eq!(commitment.chain_id, 0);
    // relay_id is hashed to [u8; 32], so just verify it exists
    assert_eq!(commitment.relay_id.as_bytes().len(), 32);

    // Test with very long relay ID
    let long_relay_id = "a".repeat(1000);
    let commitment_long = XgaCommitment::new(
        [1u8; 32],
        BlsPublicKey::from([2u8; 48]),
        &long_relay_id,
        1,
        XgaParameters::default(),
    );
    // relay_id is hashed to [u8; 32], so verify length not content
    assert_eq!(commitment_long.relay_id.as_bytes().len(), 32);
}

#[test]
fn test_xga_parameters_boundary_values() {
    // Test with maximum values
    let max_params = XgaParameters {
        version: u64::MAX,
        min_inclusion_slot: u64::MAX,
        max_inclusion_slot: u64::MAX,
        flags: u64::MAX,
    };

    let commitment = XgaCommitment::new(
        [1u8; 32],
        BlsPublicKey::from([2u8; 48]),
        "test-relay",
        1,
        max_params.clone(),
    );

    assert_eq!(commitment.parameters.version, u64::MAX);
    assert_eq!(commitment.parameters.min_inclusion_slot, u64::MAX);
    assert_eq!(commitment.parameters.max_inclusion_slot, u64::MAX);
    assert_eq!(commitment.parameters.flags, u64::MAX);

    // Test with min > max slots (invalid but should handle gracefully)
    let invalid_params = XgaParameters {
        version: 1,
        min_inclusion_slot: 100,
        max_inclusion_slot: 50, // less than min
        flags: 0,
    };

    let commitment_invalid = XgaCommitment::new(
        [1u8; 32],
        BlsPublicKey::from([2u8; 48]),
        "test-relay",
        1,
        invalid_params,
    );

    // Should accept the values as-is, validation happens elsewhere
    assert_eq!(commitment_invalid.parameters.min_inclusion_slot, 100);
    assert_eq!(commitment_invalid.parameters.max_inclusion_slot, 50);
}
