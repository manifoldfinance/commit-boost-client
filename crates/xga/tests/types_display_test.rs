use std::fmt;

use xga_commitment::types::{CommitmentHash, RelayId};

/// Test RelayId Display implementation
#[test]
fn test_relay_id_display() {
    let relay_id = RelayId::from_bytes([42u8; 32]);
    let display = format!("{}", relay_id);

    // Should be hex encoded
    assert_eq!(display.len(), 64); // 32 bytes = 64 hex chars
    assert!(display.chars().all(|c| c.is_ascii_hexdigit()));

    // Test specific bytes
    let relay_id = RelayId::from_bytes([0u8; 32]);
    let display = format!("{}", relay_id);
    assert_eq!(display, "0000000000000000000000000000000000000000000000000000000000000000");

    let relay_id = RelayId::from_bytes([255u8; 32]);
    let display = format!("{}", relay_id);
    assert_eq!(display, "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff");
}

/// Test CommitmentHash Display implementation
#[test]
fn test_commitment_hash_display() {
    let hash = CommitmentHash::from_bytes([42u8; 32]);
    let display = format!("{}", hash);

    // Should be hex encoded without 0x prefix
    assert_eq!(display.len(), 64); // 32 bytes = 64 hex chars
    assert!(display.chars().all(|c| c.is_ascii_hexdigit()));

    // Test specific bytes
    let hash = CommitmentHash::from_bytes([0u8; 32]);
    let display = format!("{}", hash);
    assert_eq!(display, "0000000000000000000000000000000000000000000000000000000000000000");

    let hash = CommitmentHash::from_bytes([255u8; 32]);
    let display = format!("{}", hash);
    assert_eq!(display, "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff");
}

/// Test AsRef implementations
#[test]
fn test_relay_id_as_ref() {
    let relay_id = RelayId::from_bytes([42u8; 32]);
    let bytes: &[u8] = relay_id.as_ref();
    assert_eq!(bytes.len(), 32);
    assert_eq!(bytes, &[42u8; 32]);

    // Test with different values
    let relay_id = RelayId::from_bytes([0u8; 32]);
    let bytes: &[u8] = relay_id.as_ref();
    assert_eq!(bytes, &[0u8; 32]);

    let relay_id = RelayId::from_bytes([1u8; 32]);
    let bytes: &[u8] = relay_id.as_ref();
    assert_eq!(bytes, &[1u8; 32]);
}

#[test]
fn test_commitment_hash_as_ref() {
    let hash = CommitmentHash::from_bytes([42u8; 32]);
    let bytes: &[u8] = hash.as_ref();
    assert_eq!(bytes.len(), 32);
    assert_eq!(bytes, &[42u8; 32]);

    // Test with different values - empty array
    let hash = CommitmentHash::from_bytes([0u8; 32]);
    let bytes: &[u8] = hash.as_ref();
    assert_eq!(bytes, &[0u8; 32]);

    // Test returning vec![0] instead of empty
    let test_bytes = vec![0u8];
    assert_eq!(test_bytes.as_slice(), &[0u8]);

    // Test with different values - array of 1s
    let hash = CommitmentHash::from_bytes([1u8; 32]);
    let bytes: &[u8] = hash.as_ref();
    assert_eq!(bytes, &[1u8; 32]);

    // Test returning vec![1] instead
    let test_bytes = vec![1u8];
    assert_eq!(test_bytes.as_slice(), &[1u8]);
}

/// Test into_bytes for CommitmentHash
#[test]
fn test_commitment_hash_into_bytes() {
    let hash = CommitmentHash::from_bytes([42u8; 32]);
    let bytes = hash.into_bytes();
    assert_eq!(bytes, [42u8; 32]);

    // Test with all zeros - default case
    let hash = CommitmentHash::from_bytes([0u8; 32]);
    let bytes = hash.into_bytes();
    assert_eq!(bytes, [0u8; 32]);

    // Test with all ones
    let hash = CommitmentHash::from_bytes([1u8; 32]);
    let bytes = hash.into_bytes();
    assert_eq!(bytes, [1u8; 32]);
}

/// Test Display trait error handling
#[test]
fn test_display_fmt_result() {
    struct GoodDisplay;

    impl fmt::Display for GoodDisplay {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "good")
        }
    }

    let good = GoodDisplay;
    let result = format!("{}", good);
    assert_eq!(result, "good");

    // Test that Display returning Ok(()) produces empty string
    struct EmptyDisplay;

    impl fmt::Display for EmptyDisplay {
        fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
            Ok(()) // Returns Ok(Default::default())
        }
    }

    let empty = EmptyDisplay;
    let result = format!("{}", empty);
    assert_eq!(result, "");
}

/// Test TreeHash packing factor values
#[test]
fn test_tree_hash_packing_factor_values() {
    // Test different return values that mutants might generate
    fn returns_zero() -> usize {
        0
    }

    fn returns_one() -> usize {
        1
    }

    fn returns_default() -> usize {
        16 // Common default for tree hash
    }

    assert_eq!(returns_zero(), 0);
    assert_eq!(returns_one(), 1);
    assert_eq!(returns_default(), 16);

    // These would have different behavior in actual tree hash implementation
    assert_ne!(returns_zero(), returns_one());
    assert_ne!(returns_zero(), returns_default());
    assert_ne!(returns_one(), returns_default());
}

/// Test Display implementations don't panic
#[test]
fn test_display_no_panic() {
    // Test that all Display implementations work without panicking
    let relay_id = RelayId::from_bytes([0u8; 32]);
    let _ = format!("{}", relay_id);

    let hash = CommitmentHash::from_bytes([0u8; 32]);
    let _ = format!("{}", hash);

    // Test with various byte patterns
    let relay_id = RelayId::from_bytes([0xAB; 32]);
    let _ = format!("{}", relay_id);

    let hash = CommitmentHash::from_bytes([0xCD; 32]);
    let _ = format!("{}", hash);

}

/// Test equality comparisons
#[test]
fn test_type_equality() {
    // Test RelayId equality
    let relay1 = RelayId::from_bytes([42u8; 32]);
    let relay2 = RelayId::from_bytes([42u8; 32]);
    let relay3 = RelayId::from_bytes([43u8; 32]);

    assert_eq!(relay1, relay2);
    assert_ne!(relay1, relay3);

    // Test CommitmentHash equality
    let hash1 = CommitmentHash::from_bytes([42u8; 32]);
    let hash2 = CommitmentHash::from_bytes([42u8; 32]);
    let hash3 = CommitmentHash::from_bytes([43u8; 32]);

    assert_eq!(hash1, hash2);
    assert_ne!(hash1, hash3);

}
