//! Input validation for XGA module

use alloy_rpc_types::beacon::relay::ValidatorRegistration;
use commit_boost::prelude::BlsPublicKey;
use eyre::{ensure, Result};
use url::Url;

use crate::commitment::{XgaCommitment, XgaParameters};

/// Validate a BLS public key
pub fn validate_validator_pubkey(pubkey: &BlsPublicKey) -> Result<()> {
    // BLS public keys must be exactly 48 bytes
    ensure!(pubkey.0.len() == 48, "Invalid BLS public key length: expected 48 bytes, got {}", pubkey.0.len());
    
    // Check if it's not the zero pubkey
    ensure!(!pubkey.0.iter().all(|&b| b == 0), "Invalid BLS public key: zero key not allowed");
    
    Ok(())
}

/// Validate XGA parameters
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

/// Validate an XGA commitment
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

/// Validate a validator registration from relay
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

/// Validate a relay URL
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