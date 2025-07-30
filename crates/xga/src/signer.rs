use commit_boost::prelude::*;
use std::sync::Arc;
use tracing::{debug, error, info, warn};

use crate::{
    commitment::{SignedXGACommitment, XGACommitment},
    config::XGAConfig,
    infrastructure::{CircuitBreaker, HttpClientFactory},
    metrics::{
        SIGNATURES_RECEIVED, SIGNATURES_REQUESTED, SIGNATURE_ERRORS, SIGNATURE_VERIFICATIONS,
        VALIDATOR_METRICS,
    },
    relay::send_to_relay,
};

/// Process a commitment by signing it and sending to the relay
pub async fn process_commitment(
    commitment: XGACommitment,
    relay_url: String,
    config: Arc<StartCommitModuleConfig<XGAConfig>>,
    circuit_breaker: &CircuitBreaker,
    http_client_factory: &HttpClientFactory,
) -> eyre::Result<()> {
    info!(
        validator_pubkey = ?commitment.validator_pubkey,
        relay_id = ?commitment.relay_id,
        "Processing XGA commitment"
    );

    // Get the signature
    let signature = sign_commitment(&commitment, &config).await?;

    // Verify the signature before sending (defensive programming)
    if !verify_signature(&commitment, &signature, &commitment.validator_pubkey) {
        error!(
            validator_pubkey = ?commitment.validator_pubkey,
            "Failed to verify our own signature - this should not happen"
        );
        return Err(eyre::eyre!("Signature verification failed"));
    }

    // Create signed commitment
    let signed_commitment = SignedXGACommitment { message: commitment, signature };

    // Send to relay with retries
    send_to_relay(
        signed_commitment,
        relay_url,
        config.extra.retry_attempts,
        config.extra.retry_delay_ms,
        circuit_breaker,
        http_client_factory,
    )
    .await?;

    Ok(())
}

/// Sign an XGA commitment using the validator's BLS key
async fn sign_commitment(
    commitment: &XGACommitment,
    config: &StartCommitModuleConfig<XGAConfig>,
) -> eyre::Result<BlsSignature> {
    SIGNATURES_REQUESTED.inc();

    // Track validator signature request
    VALIDATOR_METRICS.with_label_values(&["validator", "signature_requested"]).inc();

    debug!(
        validator_pubkey = ?commitment.validator_pubkey,
        "Requesting signature for XGA commitment"
    );

    // Build the signing request directly with the commitment
    let request = SignConsensusRequest::builder(commitment.validator_pubkey).with_msg(commitment);

    // Request signature from Commit Boost signer
    match config.signer_client.clone().request_consensus_signature(request).await {
        Ok(signature) => {
            SIGNATURES_RECEIVED.inc();
            info!(
                validator_pubkey = ?commitment.validator_pubkey,
                "Successfully signed XGA commitment"
            );
            Ok(signature)
        }
        Err(e) => {
            SIGNATURE_ERRORS.inc();
            error!(
                validator_pubkey = ?commitment.validator_pubkey,
                error = %e,
                "Failed to sign XGA commitment"
            );
            Err(eyre::eyre!("Signing failed: {}", e))
        }
    }
}

/// Domain separation tag for XGA commitments
/// This ensures signatures for XGA commitments cannot be reused for other purposes
const XGA_DST: &[u8] = b"BLS_SIG_XGA_COMMITMENT_COMMIT_BOOST";

/// Verify a BLS signature on an XGA commitment
pub fn verify_signature(
    commitment: &XGACommitment,
    signature: &BlsSignature,
    pubkey: &BlsPublicKey,
) -> bool {
    use blst::{min_pk::*, BLST_ERROR};

    SIGNATURE_VERIFICATIONS.inc();

    // Get the message to verify (TreeHash root of the commitment)
    let message = commitment.get_tree_hash_root();

    // Convert signature bytes to blst signature
    let sig_result = match Signature::from_bytes(&signature.0) {
        Ok(sig) => sig,
        Err(e) => {
            warn!(
                error = ?e,
                "Failed to deserialize BLS signature"
            );
            return false;
        }
    };

    // Convert public key bytes to blst public key
    let pk_result = match PublicKey::from_bytes(&pubkey.0) {
        Ok(pk) => pk,
        Err(e) => {
            warn!(
                error = ?e,
                "Failed to deserialize BLS public key"
            );
            return false;
        }
    };

    // Perform the verification with proper domain separation
    let result = sig_result.verify(
        true, // Use signature aggregation
        &message.0,
        XGA_DST, // Use XGA-specific domain separation tag
        &[],
        &pk_result,
        true, // Use pk_validate
    );

    // Use constant-time comparison for the result
    let is_success = matches!(result, BLST_ERROR::BLST_SUCCESS);

    if is_success {
        debug!(
            validator_pubkey = ?pubkey,
            "Successfully verified BLS signature"
        );
    } else {
        warn!(
            validator_pubkey = ?pubkey,
            error = ?result,
            "BLS signature verification failed"
        );
    }

    is_success
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commitment::XGAParameters;

    #[test]
    fn test_verify_signature_with_proper_dst() {
        use blst::min_pk::*;

        // Create test data
        let commitment = XGACommitment::new(
            [0x42u8; 32],
            BlsPublicKey::from([0x01u8; 48]),
            "test-relay".to_string(),
            1,
            XGAParameters {
                version: 2,
                min_inclusion_slot: 100,
                max_inclusion_slot: 200,
                flags: 0x1,
            },
        );

        // Generate a test keypair
        let ikm = b"test key material for BLS signature testing";
        let sk = SecretKey::key_gen(ikm, &[]).expect("Test key generation should succeed");
        let pk = sk.sk_to_pk();

        // Get the message to sign (tree hash root)
        let message = commitment.get_tree_hash_root();

        // Sign the message with our DST
        let sig = sk.sign(&message.0, XGA_DST, &[]);

        // Convert to our types
        let pubkey = BlsPublicKey::from(pk.to_bytes());
        let signature = BlsSignature::from(sig.to_bytes());

        // Verify the signature
        assert!(
            verify_signature(&commitment, &signature, &pubkey),
            "Valid signature should verify"
        );

        // Test with wrong public key
        let wrong_pubkey = BlsPublicKey::from([0x02u8; 48]);
        assert!(
            !verify_signature(&commitment, &signature, &wrong_pubkey),
            "Signature with wrong pubkey should not verify"
        );

        // Test with modified commitment (different nonce)
        let mut modified_commitment = commitment.clone();
        modified_commitment.nonce = crate::types::Nonce::from_bytes([0xFFu8; 32]);
        assert!(
            !verify_signature(&modified_commitment, &signature, &pubkey),
            "Signature for different commitment should not verify"
        );
    }

    #[test]
    fn test_verify_signature_rejects_invalid_formats() {
        let commitment = XGACommitment::new(
            [0u8; 32],
            BlsPublicKey::default(),
            "test-relay".to_string(),
            1,
            XGAParameters::default(),
        );

        // Test with invalid signature bytes (all zeros)
        let invalid_sig = BlsSignature::from([0u8; 96]);
        let pubkey = BlsPublicKey::from([0x01u8; 48]);
        assert!(
            !verify_signature(&commitment, &invalid_sig, &pubkey),
            "Invalid signature format should not verify"
        );

        // Test with invalid public key bytes (all zeros)
        let valid_sig = BlsSignature::from([0x01u8; 96]);
        let invalid_pubkey = BlsPublicKey::from([0u8; 48]);
        assert!(
            !verify_signature(&commitment, &valid_sig, &invalid_pubkey),
            "Invalid pubkey format should not verify"
        );
    }
}
