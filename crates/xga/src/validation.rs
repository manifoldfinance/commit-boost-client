//! Input validation for XGA module

use alloy_rpc_types::beacon::relay::ValidatorRegistration;
use commit_boost::prelude::BlsPublicKey;
use eyre::{ensure, Result};
use url::Url;

use crate::commitment::{XgaCommitment, XgaParameters};

/// Validates a BLS public key for correctness.
///
/// This function ensures that the public key has the correct length (48 bytes)
/// and is not the zero key, which is invalid in BLS cryptography.
///
/// # Arguments
///
/// * `pubkey` - The BLS public key to validate
///
/// # Errors
///
/// Returns an error if:
/// - The public key is not exactly 48 bytes
/// - The public key is all zeros (invalid zero key)
///
/// # Example
///
/// ```no_run
/// # use commit_boost::prelude::BlsPublicKey;
/// # use xga_commitment::validation::validate_validator_pubkey;
/// let pubkey = BlsPublicKey::from([1u8; 48]);
/// assert!(validate_validator_pubkey(&pubkey).is_ok());
/// ```
pub fn validate_validator_pubkey(pubkey: &BlsPublicKey) -> Result<()> {
    // BLS public keys must be exactly 48 bytes
    ensure!(pubkey.0.len() == 48, "Invalid BLS public key length: expected 48 bytes, got {}", pubkey.0.len());
    
    // Check if it's not the zero pubkey
    ensure!(!pubkey.0.iter().all(|&b| b == 0), "Invalid BLS public key: zero key not allowed");
    
    Ok(())
}

/// Validates XGA parameters for correctness and consistency.
///
/// This function ensures that the XGA parameters follow the protocol rules,
/// including version constraints, slot range validity, and flag restrictions.
///
/// # Arguments
///
/// * `params` - The XGA parameters to validate
///
/// # Errors
///
/// Returns an error if:
/// - Version is not between 1 and 100 (reasonable range)
/// - Min inclusion slot is > max inclusion slot
/// - Slot range exceeds 1000 slots (reasonable limit)
///
/// # Example
///
/// ```no_run
/// # use xga_commitment::commitment::XgaParameters;
/// # use xga_commitment::validation::validate_xga_parameters;
/// let params = XgaParameters {
///     version: 1,
///     min_inclusion_slot: 100,
///     max_inclusion_slot: 150,
///     flags: 0,
/// };
/// assert!(validate_xga_parameters(&params).is_ok());
/// ```
pub fn validate_xga_parameters(params: &XgaParameters) -> Result<()> {
    // Version must be reasonable
    ensure!(params.version > 0 && params.version <= 100, "Invalid XGA version: {}", params.version);
    
    // Slot ranges must be valid
    ensure!(
        params.min_inclusion_slot <= params.max_inclusion_slot,
        "Invalid slot range: min_inclusion_slot ({}) > max_inclusion_slot ({})",
        params.min_inclusion_slot,
        params.max_inclusion_slot
    );
    
    // Slot range must be reasonable (not too large)
    let slot_range = params.max_inclusion_slot - params.min_inclusion_slot;
    ensure!(
        slot_range <= 1000,
        "Slot range too large: {} slots",
        slot_range
    );
    
    Ok(())
}

/// Validates a complete XGA commitment for correctness.
///
/// This function performs comprehensive validation of an XGA commitment,
/// including all nested structures and ensuring the commitment follows
/// protocol rules.
///
/// # Arguments
///
/// * `commitment` - The XGA commitment to validate
///
/// # Errors
///
/// Returns an error if:
/// - Validator public key is invalid (see [`validate_validator_pubkey`])
/// - XGA parameters are invalid (see [`validate_xga_parameters`])
/// - Timestamp is more than 5 minutes in the future (prevents replay attacks)
///
/// # Example
///
/// ```no_run
/// # use xga_commitment::commitment::{XgaCommitment, XgaParameters};
/// # use xga_commitment::validation::validate_commitment;
/// # use commit_boost::prelude::BlsPublicKey;
/// let commitment = XgaCommitment::new(
///     [1u8; 32], // registration_hash
///     BlsPublicKey::from([1u8; 48]),
///     "test-relay",
///     1, // xga_version
///     XgaParameters::default(),
/// );
/// assert!(validate_commitment(&commitment).is_ok());
/// ```
pub fn validate_commitment(commitment: &XgaCommitment) -> Result<()> {
    // Validate the validator pubkey
    validate_validator_pubkey(&commitment.validator_pubkey)?;
    
    // Validate XGA parameters
    validate_xga_parameters(&commitment.parameters)?;
    
    // Validate timestamp is not too far in the future (allow 5 minutes)
    let current_time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    let max_future_time = current_time + 300; // 5 minutes
    
    ensure!(
        commitment.timestamp <= max_future_time,
        "Commitment timestamp too far in future: {} > {}",
        commitment.timestamp,
        max_future_time
    );
    
    Ok(())
}

/// Validates a validator registration for the mev-boost protocol.
///
/// This function ensures that the validator registration contains valid
/// data that can be safely processed by relays.
///
/// # Arguments
///
/// * `registration` - The validator registration to validate
///
/// # Errors
///
/// Returns an error if:
/// - Fee recipient is the zero address (0x0...0)
/// - Gas limit is less than 1M or greater than 50M
/// - Public key is invalid BLS format
///
/// # Example
///
/// ```no_run
/// # use alloy::rpc::types::beacon::relay::ValidatorRegistration;
/// # use xga_commitment::validation::validate_registration;
/// // Assumes you have a valid ValidatorRegistration
/// # let registration: ValidatorRegistration = todo!();
/// assert!(validate_registration(&registration).is_ok());
/// ```
pub fn validate_registration(registration: &ValidatorRegistration) -> Result<()> {
    // Validate the validator pubkey
    validate_validator_pubkey(&registration.message.pubkey)?;
    
    // Validate fee recipient is not zero address
    ensure!(
        !registration.message.fee_recipient.is_zero(),
        "Invalid fee recipient: zero address not allowed"
    );
    
    // Validate gas limit is reasonable
    ensure!(
        registration.message.gas_limit >= 1_000_000 && registration.message.gas_limit <= 50_000_000,
        "Invalid gas limit: {} (must be between 1M and 50M)",
        registration.message.gas_limit
    );
    
    Ok(())
}

/// Validates a relay URL for security and correctness.
///
/// This function ensures that relay URLs are properly formatted, use HTTPS,
/// and have a valid host.
///
/// # Arguments
///
/// * `url` - The relay URL string to validate
///
/// # Errors
///
/// Returns an error if:
/// - URL cannot be parsed as a valid URL
/// - URL scheme is not HTTPS (HTTP is not allowed)
/// - URL doesn't have a valid host
///
/// # Example
///
/// ```no_run
/// # use xga_commitment::validation::validate_relay_url;
/// assert!(validate_relay_url("https://relay.example.com").is_ok());
/// assert!(validate_relay_url("http://relay.example.com").is_err()); // Not HTTPS
/// assert!(validate_relay_url("https://").is_err()); // No host
/// ```
pub fn validate_relay_url(url: &str) -> Result<()> {
    // Parse URL
    let parsed_url = Url::parse(url)?;
    
    // Must be HTTPS
    ensure!(
        parsed_url.scheme() == "https",
        "Relay URL must use HTTPS: {}",
        url
    );
    
    // Must have a host
    ensure!(
        parsed_url.host_str().is_some(),
        "Relay URL must have a valid host: {}",
        url
    );
    
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_validate_validator_pubkey() {
        // Valid pubkey
        let valid_pubkey = BlsPublicKey::from([1u8; 48]);
        assert!(validate_validator_pubkey(&valid_pubkey).is_ok());
        
        // Zero pubkey
        let zero_pubkey = BlsPublicKey::from([0u8; 48]);
        assert!(validate_validator_pubkey(&zero_pubkey).is_err());
    }
    
    #[test]
    fn test_validate_xga_parameters() {
        // Valid parameters
        let valid_params = XgaParameters {
            version: 1,
            min_inclusion_slot: 100,
            max_inclusion_slot: 200,
            flags: 0,
        };
        assert!(validate_xga_parameters(&valid_params).is_ok());
        
        // Invalid version
        let invalid_version = XgaParameters {
            version: 0,
            ..valid_params
        };
        assert!(validate_xga_parameters(&invalid_version).is_err());
        
        // Invalid slot range
        let invalid_range = XgaParameters {
            min_inclusion_slot: 200,
            max_inclusion_slot: 100,
            ..valid_params
        };
        assert!(validate_xga_parameters(&invalid_range).is_err());
        
        // Too large slot range
        let large_range = XgaParameters {
            min_inclusion_slot: 0,
            max_inclusion_slot: 2000,
            ..valid_params
        };
        assert!(validate_xga_parameters(&large_range).is_err());
    }
    
    #[test]
    fn test_validate_relay_url() {
        // Valid HTTPS URL
        assert!(validate_relay_url("https://relay.example.com").is_ok());
        
        // HTTP URL (should fail)
        assert!(validate_relay_url("http://relay.example.com").is_err());
        
        // Invalid URL
        assert!(validate_relay_url("not-a-url").is_err());
        
        // URL without host
        assert!(validate_relay_url("https://").is_err());
    }
}