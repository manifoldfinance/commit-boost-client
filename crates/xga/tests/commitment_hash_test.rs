use alloy::rpc::types::beacon::relay::{ValidatorRegistration, ValidatorRegistrationMessage};
use commit_boost::prelude::*;
use xga_commitment::commitment::{XgaCommitment, XgaParameters, XGA_SIGNING_DOMAIN};

#[test]
fn test_registration_hash_consistency() {
    let message = ValidatorRegistrationMessage {
        fee_recipient: alloy::primitives::Address::from([1u8; 20]),
        gas_limit: 30000000,
        timestamp: 1234567890,
        pubkey: alloy::rpc::types::beacon::BlsPublicKey::from([2u8; 48]),
    };

    let registration = ValidatorRegistration {
        message,
        signature: alloy::rpc::types::beacon::BlsSignature::from([3u8; 96]),
    };

    // Hash the same registration twice
    let hash1 = XgaCommitment::hash_registration(&registration);
    let hash2 = XgaCommitment::hash_registration(&registration);

    // Hashes should be identical
    assert_eq!(hash1, hash2);

    // Hash should not be all zeros
    assert_ne!(hash1, [0u8; 32]);
}

#[test]
fn test_registration_hash_includes_signature() {
    let message = ValidatorRegistrationMessage {
        fee_recipient: alloy::primitives::Address::from([1u8; 20]),
        gas_limit: 30000000,
        timestamp: 1234567890,
        pubkey: alloy::rpc::types::beacon::BlsPublicKey::from([2u8; 48]),
    };

    let registration1 = ValidatorRegistration {
        message: message.clone(),
        signature: alloy::rpc::types::beacon::BlsSignature::from([3u8; 96]),
    };

    let registration2 = ValidatorRegistration {
        message,
        signature: alloy::rpc::types::beacon::BlsSignature::from([4u8; 96]), // Different signature
    };

    let hash1 = XgaCommitment::hash_registration(&registration1);
    let hash2 = XgaCommitment::hash_registration(&registration2);

    // Hashes should be different due to different signatures
    assert_ne!(hash1, hash2);
}

#[test]
fn test_commitment_tree_hash() {
    let commitment = XgaCommitment::new(
        [1u8; 32],
        BlsPublicKey::from([2u8; 48]),
        "test-relay",
        1,
        XgaParameters::default(),
    );

    // TreeHash should produce consistent results
    let root1 = commitment.get_tree_hash_root();
    let root2 = commitment.get_tree_hash_root();

    assert_eq!(root1, root2);
    assert_ne!(root1.0, [0u8; 32]);
}

#[test]
fn test_commitment_includes_signing_domain() {
    let commitment = XgaCommitment::new(
        [1u8; 32],
        BlsPublicKey::from([2u8; 48]),
        "test-relay",
        1,
        XgaParameters::default(),
    );

    // Verify signing domain is set correctly
    assert_eq!(commitment.signing_domain, XGA_SIGNING_DOMAIN);

    // Create another commitment with same data
    let commitment2 = XgaCommitment::new(
        [1u8; 32],
        BlsPublicKey::from([2u8; 48]),
        "test-relay",
        1,
        XgaParameters::default(),
    );

    // Both should have same signing domain
    assert_eq!(commitment.signing_domain, commitment2.signing_domain);
}

#[test]
fn test_relay_id_deterministic() {
    // Same relay string should produce same relay_id
    let commitment1 = XgaCommitment::new(
        [1u8; 32],
        BlsPublicKey::from([2u8; 48]),
        "relay.example.com",
        1,
        XgaParameters::default(),
    );

    let commitment2 = XgaCommitment::new(
        [1u8; 32],
        BlsPublicKey::from([2u8; 48]),
        "relay.example.com",
        1,
        XgaParameters::default(),
    );

    assert_eq!(commitment1.relay_id, commitment2.relay_id);

    // Different relay string should produce different relay_id
    let commitment3 = XgaCommitment::new(
        [1u8; 32],
        BlsPublicKey::from([2u8; 48]),
        "other-relay.example.com",
        1,
        XgaParameters::default(),
    );

    assert_ne!(commitment1.relay_id, commitment3.relay_id);
}

#[test]
fn test_xga_parameters_affect_tree_hash() {
    let params1 =
        XgaParameters { version: 1, min_inclusion_slot: 100, max_inclusion_slot: 200, flags: 0 };

    let params2 = XgaParameters {
        version: 1,
        min_inclusion_slot: 100,
        max_inclusion_slot: 300, // Different
        flags: 0,
    };

    let commitment1 = XgaCommitment::new(
        [1u8; 32],
        BlsPublicKey::from([2u8; 48]),
        "test-relay",
        1,
        params1,
    );

    let commitment2 = XgaCommitment::new(
        [1u8; 32],
        BlsPublicKey::from([2u8; 48]),
        "test-relay",
        1,
        params2,
    );

    // Different parameters should produce different tree hash
    assert_ne!(commitment1.get_tree_hash_root(), commitment2.get_tree_hash_root());
}
