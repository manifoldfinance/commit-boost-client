use blst::min_pk::SecretKey;
use commit_boost::prelude::*;
use xga_commitment::commitment::{XGACommitment, XGAParameters};
use xga_commitment::signer::verify_signature;

#[test]
fn test_signature_verification_flow() {
    // Generate a test keypair
    let ikm = [42u8; 32]; // Test entropy
    let sk = SecretKey::key_gen(&ikm, &[]).expect("Failed to generate secret key for testing");
    let pk = sk.sk_to_pk();

    // Convert to our types
    let pk_bytes = pk.compress();
    let validator_pubkey = BlsPublicKey::from(pk_bytes);

    // Create a test commitment
    let commitment = XGACommitment::new(
        [1u8; 32], // registration hash
        validator_pubkey.clone(),
        "test-relay".to_string(),
        1, // mainnet
        XGAParameters::default(),
    );

    // Get the message to sign (TreeHash root)
    let message = commitment.get_tree_hash_root();

    // Sign the message with the same DST used in verification
    const XGA_DST: &[u8] = b"BLS_SIG_XGA_COMMITMENT_COMMIT_BOOST";
    let sig = sk.sign(&message.0, XGA_DST, &[]);
    let sig_bytes = sig.compress();
    let signature = BlsSignature::from(sig_bytes);

    // Verify the signature
    assert!(
        verify_signature(&commitment, &signature, &validator_pubkey),
        "Valid signature should verify successfully"
    );

    // Test with wrong signature
    let wrong_sig = BlsSignature::from([99u8; 96]);
    assert!(
        !verify_signature(&commitment, &wrong_sig, &validator_pubkey),
        "Invalid signature should fail verification"
    );

    // Test with wrong public key
    let wrong_pk = BlsPublicKey::from([88u8; 48]);
    assert!(
        !verify_signature(&commitment, &signature, &wrong_pk),
        "Signature with wrong public key should fail"
    );
}

#[test]
fn test_signature_verification_invalid_key_formats() {
    // Test with malformed public key (wrong length)
    let commitment = XGACommitment::new(
        [1u8; 32],
        BlsPublicKey::from([2u8; 48]),
        "test-relay".to_string(),
        1,
        XGAParameters::default(),
    );

    let signature = BlsSignature::from([3u8; 96]);

    // Test with zero public key
    let zero_pk = BlsPublicKey::from([0u8; 48]);
    assert!(
        !verify_signature(&commitment, &signature, &zero_pk),
        "Verification with zero public key should fail"
    );

    // Test with zero signature
    let valid_pk = BlsPublicKey::from([2u8; 48]);
    let zero_sig = BlsSignature::from([0u8; 96]);
    assert!(
        !verify_signature(&commitment, &zero_sig, &valid_pk),
        "Verification with zero signature should fail"
    );
}

#[test]
fn test_signature_verification_different_commitments() {
    // Generate a test keypair
    let ikm = [42u8; 32];
    let sk = SecretKey::key_gen(&ikm, &[]).expect("Failed to generate secret key for testing");
    let pk = sk.sk_to_pk();

    let pk_bytes = pk.compress();
    let validator_pubkey = BlsPublicKey::from(pk_bytes);

    // Create first commitment and sign it
    let commitment1 = XGACommitment::new(
        [1u8; 32],
        validator_pubkey.clone(),
        "relay1".to_string(),
        1,
        XGAParameters::default(),
    );

    let message1 = commitment1.get_tree_hash_root();
    const XGA_DST: &[u8] = b"BLS_SIG_XGA_COMMITMENT_COMMIT_BOOST";
    let sig1 = sk.sign(&message1.0, XGA_DST, &[]);
    let signature1 = BlsSignature::from(sig1.compress());

    // Create second commitment (different relay)
    let commitment2 = XGACommitment::new(
        [1u8; 32],
        validator_pubkey.clone(),
        "relay2".to_string(),
        1,
        XGAParameters::default(),
    );

    // Verify signatures match their commitments
    assert!(
        verify_signature(&commitment1, &signature1, &validator_pubkey),
        "Signature for commitment1 should verify"
    );

    // Cross-verification should fail
    assert!(
        !verify_signature(&commitment2, &signature1, &validator_pubkey),
        "Signature for commitment1 should not verify commitment2"
    );
}
