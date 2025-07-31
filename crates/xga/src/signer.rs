use std::sync::Arc;

use commit_boost::prelude::*;
use tracing::{debug, error, info, warn};

use crate::{
    commitment::{SignedXgaCommitment, XgaCommitment},
    config::XgaConfig,
    infrastructure::{CircuitBreaker, HttpClientFactory, ValidatorRateLimiter},
    relay::send_to_relay,
    validation::validate_commitment,
};

/// Processes an XGA commitment by signing it and sending to the relay.
///
/// This function orchestrates the complete commitment processing flow:
/// 1. Validates the commitment
/// 2. Checks rate limits per validator
/// 3. Checks circuit breaker status for the relay
/// 4. Signs the commitment using the validator's BLS key
/// 5. Verifies the signature (defensive programming)
/// 6. Sends the signed commitment to the relay
/// 7. Updates circuit breaker status based on success/failure
///
/// # Arguments
///
/// * `commitment` - The XGA commitment to process
/// * `relay_url` - URL of the relay to send the commitment to
/// * `config` - Module configuration including signer client
/// * `circuit_breaker` - Circuit breaker for relay fault tolerance
/// * `http_client_factory` - Factory for creating HTTP clients
/// * `validator_rate_limiter` - Rate limiter for per-validator limits
///
/// # Errors
///
/// Returns an error if:
/// - The commitment fails validation
/// - Rate limit is exceeded for the validator
/// - Circuit breaker is open for the relay
/// - Signing fails
/// - Signature verification fails
/// - Relay submission fails
///
/// # Example
///
/// ```no_run
/// # use xga_commitment::signer::process_commitment;
/// # async fn example() -> eyre::Result<()> {
/// // Assumes you have all the required components initialized
/// # todo!()
/// # }
/// ```
pub async fn process_commitment(
    commitment: XgaCommitment,
    relay_url: &str,
    config: Arc<StartCommitModuleConfig<XgaConfig>>,
    circuit_breaker: &CircuitBreaker,
    http_client_factory: &HttpClientFactory,
    validator_rate_limiter: &ValidatorRateLimiter,
) -> eyre::Result<()> {
    info!(
        validator_pubkey = ?commitment.validator_pubkey,
        relay_id = ?commitment.relay_id,
        "Processing XGA commitment"
    );

    // Validate the commitment first
    validate_commitment(&commitment)?;

    // Check rate limit for this validator
    if !validator_rate_limiter.check_rate_limit(&commitment.validator_pubkey).await {
        warn!(
            validator_pubkey = ?commitment.validator_pubkey,
            "Rate limit exceeded for validator"
        );
        return Err(eyre::eyre!("Rate limit exceeded for validator"));
    }

    // Check circuit breaker before processing
    if circuit_breaker.is_open(relay_url).await {
        warn!(
            relay_url = relay_url,
            "Circuit breaker is open for relay, skipping commitment"
        );
        return Err(eyre::eyre!("Circuit breaker is open for relay"));
    }

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
    let signed_commitment = SignedXgaCommitment { message: commitment, signature };

    // Send to relay with retries
    let result = send_to_relay(
        signed_commitment,
        relay_url,
        &config.extra.retry_config,
        http_client_factory,
    )
    .await;

    // Record success or failure with circuit breaker
    match result {
        Ok(()) => {
            circuit_breaker.record_success(relay_url).await;
            Ok(())
        }
        Err(e) => {
            circuit_breaker.record_failure(relay_url).await;
            Err(e)
        }
    }
}

/// Sign an XGA commitment using the validator's BLS key
async fn sign_commitment(
    commitment: &XgaCommitment,
    config: &StartCommitModuleConfig<XgaConfig>,
) -> eyre::Result<BlsSignature> {
    debug!(
        validator_pubkey = ?commitment.validator_pubkey,
        "Requesting signature for XGA commitment"
    );

    // Build the signing request directly with the commitment
    let request = SignConsensusRequest::builder(commitment.validator_pubkey).with_msg(commitment);

    // Request signature from Commit Boost signer
    match config.signer_client.clone().request_consensus_signature(request).await {
        Ok(signature) => {
            info!(
                validator_pubkey = ?commitment.validator_pubkey,
                "Successfully signed XGA commitment"
            );
            Ok(signature)
        }
        Err(e) => {
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
/// This ensures signatures for XGA commitments cannot be reused for other
/// purposes
const XGA_DST: &[u8] = b"BLS_SIG_XGA_COMMITMENT_COMMIT_BOOST";

/// Verifies a BLS signature on an XGA commitment.
///
/// This function performs cryptographic verification of a BLS signature
/// using the XGA-specific domain separation tag to ensure signatures
/// cannot be reused for other purposes.
///
/// # Arguments
///
/// * `commitment` - The XGA commitment that was signed
/// * `signature` - The BLS signature to verify
/// * `pubkey` - The public key to verify the signature against
///
/// # Returns
///
/// Returns `true` if the signature is valid, `false` otherwise.
/// This function never panics and handles all error cases by returning false.
///
/// # Security Note
///
/// This function uses the XGA-specific domain separation tag `XGA_DST`
/// to prevent signature reuse attacks between different protocols.
///
/// # Example
///
/// ```no_run
/// # use xga_commitment::signer::verify_signature;
/// # use xga_commitment::commitment::XgaCommitment;
/// # use commit_boost::prelude::{BlsSignature, BlsPublicKey};
/// # let commitment: XgaCommitment = todo!();
/// # let signature: BlsSignature = todo!();
/// # let pubkey: BlsPublicKey = todo!();
/// let is_valid = verify_signature(&commitment, &signature, &pubkey);
/// assert!(is_valid);
/// ```
pub fn verify_signature(
    commitment: &XgaCommitment,
    signature: &BlsSignature,
    pubkey: &BlsPublicKey,
) -> bool {
    use blst::{min_pk::*, BLST_ERROR};

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
    use crate::commitment::XgaParameters;

    #[test]
    fn test_verify_signature_with_proper_dst() {
        use blst::min_pk::*;

        // Create test data
        let commitment = XgaCommitment::new(
            [0x42u8; 32],
            BlsPublicKey::from([0x01u8; 48]),
            "test-relay",
            1,
            XgaParameters {
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

        // Test with modified commitment (different timestamp)
        let mut modified_commitment = commitment.clone();
        modified_commitment.timestamp += 1000;
        assert!(
            !verify_signature(&modified_commitment, &signature, &pubkey),
            "Signature for different commitment should not verify"
        );
    }

    #[test]
    fn test_verify_signature_rejects_invalid_formats() {
        let commitment = XgaCommitment::new(
            [0u8; 32],
            BlsPublicKey::default(),
            "test-relay",
            1,
            XgaParameters::default(),
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
