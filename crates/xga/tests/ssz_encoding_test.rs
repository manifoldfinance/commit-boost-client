//! Comprehensive SSZ encoding/decoding tests for XGA types

use ssz::{Decode, Encode};
use xga_commitment::{
    commitment::{SignedXgaCommitment, XgaCommitment, XgaParameters},
    types::{CommitmentHash, RelayId},
};

/// Test round-trip encoding/decoding for XgaParameters
#[test]
fn test_xga_parameters_ssz_round_trip() {
    let params = XgaParameters {
        version: 1,
        min_inclusion_slot: 100,
        max_inclusion_slot: 200,
        flags: 0x123456,
    };

    // Encode to SSZ
    let encoded = params.as_ssz_bytes();

    // Decode from SSZ
    let decoded = XgaParameters::from_ssz_bytes(&encoded).expect("Should decode successfully");

    // Verify equality
    assert_eq!(params.version, decoded.version);
    assert_eq!(params.min_inclusion_slot, decoded.min_inclusion_slot);
    assert_eq!(params.max_inclusion_slot, decoded.max_inclusion_slot);
    assert_eq!(params.flags, decoded.flags);
}

/// Test SSZ encoding produces consistent results
#[test]
fn test_xga_parameters_ssz_deterministic() {
    let params = XgaParameters {
        version: 2,
        min_inclusion_slot: 50,
        max_inclusion_slot: 150,
        flags: 0xABCDEF,
    };

    let encoded1 = params.as_ssz_bytes();
    let encoded2 = params.as_ssz_bytes();

    assert_eq!(encoded1, encoded2, "SSZ encoding should be deterministic");
}

/// Test RelayId SSZ encoding (transparent)
#[test]
fn test_relay_id_ssz_transparent() {
    let relay_id = RelayId::from_bytes([0x42; 32]);

    // Encode
    let encoded = relay_id.as_ssz_bytes();

    // Should be exactly 32 bytes (transparent encoding)
    assert_eq!(encoded.len(), 32);
    assert_eq!(&encoded[..], &[0x42; 32]);

    // Decode
    let decoded = RelayId::from_ssz_bytes(&encoded).expect("Should decode");
    assert_eq!(relay_id, decoded);
}

/// Test CommitmentHash SSZ encoding (transparent)
#[test]
fn test_commitment_hash_ssz_transparent() {
    let hash = CommitmentHash::from_bytes([0x99; 32]);

    // Encode
    let encoded = hash.as_ssz_bytes();

    // Should be exactly 32 bytes (transparent encoding)
    assert_eq!(encoded.len(), 32);
    assert_eq!(&encoded[..], &[0x99; 32]);

    // Decode
    let decoded = CommitmentHash::from_ssz_bytes(&encoded).expect("Should decode");
    assert_eq!(hash, decoded);
}

/// Test XgaCommitment SSZ round-trip
#[test]
fn test_xga_commitment_ssz_round_trip() {
    use commit_boost::prelude::BlsPublicKey;

    let commitment = XgaCommitment::new(
        [0x11; 32],
        BlsPublicKey::from([0x22; 48]),
        "https://relay.example.com",
        1,
        XgaParameters {
            version: 3,
            min_inclusion_slot: 1000,
            max_inclusion_slot: 2000,
            flags: 0xFF00FF,
        },
    );

    // Encode
    let encoded = commitment.as_ssz_bytes();

    // Decode
    let decoded = XgaCommitment::from_ssz_bytes(&encoded).expect("Should decode");

    // Verify all fields
    assert_eq!(commitment.registration_hash, decoded.registration_hash);
    assert_eq!(commitment.validator_pubkey, decoded.validator_pubkey);
    assert_eq!(commitment.relay_id, decoded.relay_id);
    assert_eq!(commitment.xga_version, decoded.xga_version);
    assert_eq!(commitment.parameters.version, decoded.parameters.version);
    assert_eq!(commitment.parameters.min_inclusion_slot, decoded.parameters.min_inclusion_slot);
    assert_eq!(commitment.parameters.max_inclusion_slot, decoded.parameters.max_inclusion_slot);
    assert_eq!(commitment.parameters.flags, decoded.parameters.flags);
    assert_eq!(commitment.chain_id, decoded.chain_id);
    assert_eq!(commitment.signing_domain, decoded.signing_domain);
}

/// Test SignedXgaCommitment SSZ round-trip
#[test]
fn test_signed_xga_commitment_ssz_round_trip() {
    use commit_boost::prelude::{BlsPublicKey, BlsSignature};

    let commitment = XgaCommitment::new(
        [0xAA; 32],
        BlsPublicKey::from([0xBB; 48]),
        "https://test-relay.example.com",
        5,
        XgaParameters::default(),
    );

    let signed = SignedXgaCommitment {
        message: commitment,
        signature: BlsSignature::from([0xCC; 96]),
    };

    // Encode
    let encoded = signed.as_ssz_bytes();

    // Decode
    let decoded = SignedXgaCommitment::from_ssz_bytes(&encoded).expect("Should decode");

    // Verify
    assert_eq!(signed.message.registration_hash, decoded.message.registration_hash);
    assert_eq!(signed.message.validator_pubkey, decoded.message.validator_pubkey);
    assert_eq!(signed.signature, decoded.signature);
}

/// Test SSZ encoding size calculations
#[test]
fn test_ssz_encoding_sizes() {
    use commit_boost::prelude::{BlsPublicKey, BlsSignature};

    // Test XgaParameters size
    let params = XgaParameters::default();
    let params_encoded = params.as_ssz_bytes();
    assert_eq!(params_encoded.len(), 4 * 8); // 4 u64 fields

    // Test XgaCommitment size
    let commitment = XgaCommitment::new(
        [0; 32],
        BlsPublicKey::default(),
        "test",
        1,
        XgaParameters::default(),
    );
    let commitment_encoded = commitment.as_ssz_bytes();
    // 32 (hash) + 48 (pubkey) + 32 (relay_id) + 8 (version) + 32 (params) + 8 (timestamp) + 8 (chain_id) + 32 (domain)
    assert_eq!(commitment_encoded.len(), 32 + 48 + 32 + 8 + 32 + 8 + 8 + 32);

    // Test SignedXgaCommitment size
    let signed = SignedXgaCommitment {
        message: commitment,
        signature: BlsSignature::default(),
    };
    let signed_encoded = signed.as_ssz_bytes();
    assert_eq!(signed_encoded.len(), 200 + 96); // commitment + signature
}

/// Test decoding from invalid data
#[test]
fn test_ssz_decode_errors() {
    // Empty data
    assert!(XgaParameters::from_ssz_bytes(&[]).is_err());
    assert!(RelayId::from_ssz_bytes(&[]).is_err());
    assert!(CommitmentHash::from_ssz_bytes(&[]).is_err());

    // Wrong size for RelayId
    assert!(RelayId::from_ssz_bytes(&[0; 31]).is_err());
    assert!(RelayId::from_ssz_bytes(&[0; 33]).is_err());

    // Wrong size for CommitmentHash
    assert!(CommitmentHash::from_ssz_bytes(&[0; 31]).is_err());
    assert!(CommitmentHash::from_ssz_bytes(&[0; 33]).is_err());

    // Truncated XgaParameters
    assert!(XgaParameters::from_ssz_bytes(&[0; 31]).is_err());
}

/// Test that SSZ encoding is compatible with tree hash
#[test]
fn test_ssz_tree_hash_compatibility() {
    use tree_hash::TreeHash;

    let params = XgaParameters {
        version: 10,
        min_inclusion_slot: 500,
        max_inclusion_slot: 1500,
        flags: 0x123,
    };

    // Get tree hash root
    let tree_root = params.tree_hash_root();

    // Encode to SSZ and back
    let encoded = params.as_ssz_bytes();
    let decoded = XgaParameters::from_ssz_bytes(&encoded).unwrap();

    // Tree hash should be the same
    assert_eq!(tree_root, decoded.tree_hash_root());
}

/// Benchmark-style test for SSZ encoding performance
#[test]
fn test_ssz_encoding_performance() {
    use commit_boost::prelude::BlsPublicKey;
    use std::time::Instant;

    let commitment = XgaCommitment::new(
        [0xFF; 32],
        BlsPublicKey::from([0xEE; 48]),
        "https://perf-test-relay.example.com",
        1,
        XgaParameters {
            version: 100,
            min_inclusion_slot: 10000,
            max_inclusion_slot: 20000,
            flags: 0xFFFFFF,
        },
    );

    const ITERATIONS: usize = 10000;

    // Encoding performance
    let start = Instant::now();
    for _ in 0..ITERATIONS {
        let _ = commitment.as_ssz_bytes();
    }
    let encode_duration = start.elapsed();

    // Pre-encode for decoding test
    let encoded = commitment.as_ssz_bytes();

    // Decoding performance
    let start = Instant::now();
    for _ in 0..ITERATIONS {
        let _ = XgaCommitment::from_ssz_bytes(&encoded).unwrap();
    }
    let decode_duration = start.elapsed();

    println!("SSZ Encoding {} iterations: {:?}", ITERATIONS, encode_duration);
    println!("SSZ Decoding {} iterations: {:?}", ITERATIONS, decode_duration);

    // Ensure it's reasonably fast (less than 1ms per op on average)
    assert!(encode_duration.as_millis() < ITERATIONS as u128);
    assert!(decode_duration.as_millis() < ITERATIONS as u128);
}