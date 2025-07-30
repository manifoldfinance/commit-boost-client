use alloy_rpc_types::beacon::relay::ValidatorRegistration;
use commit_boost::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{
    infrastructure::get_current_timestamp,
    types::{CommitmentHash, RelayId},
};

/// XGA module signing domain - "XGA_COMMITMENT" as bytes
pub const XGA_SIGNING_DOMAIN: [u8; 32] = [
    0x58, 0x47, 0x41, 0x5f, 0x43, 0x4f, 0x4d, 0x4d, 0x49, 0x54, 0x4d, 0x45, 0x4e, 0x54, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

/// Custom serde for relay_id
mod relay_id_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use super::RelayId;

    pub fn serialize<S>(relay_id: &RelayId, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        hex::encode(relay_id.as_bytes()).serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<RelayId, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        // If it's already hex and 32 bytes, use it directly
        if s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit()) {
            let bytes = hex::decode(&s).map_err(serde::de::Error::custom)?;
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&bytes);
            Ok(RelayId::from_bytes(arr))
        } else {
            // Otherwise create RelayId from URL
            Ok(RelayId::from_url(&s))
        }
    }
}

/// Custom serde for commitment_hash
mod commitment_hash_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use super::CommitmentHash;

    pub fn serialize<S>(hash: &CommitmentHash, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        hex::encode(hash.as_bytes()).serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<CommitmentHash, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let bytes = hex::decode(&s).map_err(serde::de::Error::custom)?;
        if bytes.len() != 32 {
            return Err(serde::de::Error::custom("CommitmentHash must be 32 bytes"));
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        Ok(CommitmentHash::from_bytes(arr))
    }
}

/// XGA-specific parameters for the commitment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XGAParameters {
    /// Version of the XGA protocol
    pub version: u64,
    /// Minimum guaranteed inclusion slot
    pub min_inclusion_slot: u64,
    /// Maximum guaranteed inclusion slot
    pub max_inclusion_slot: u64,
    /// Additional flags for future extensions
    pub flags: u64,
}

impl Default for XGAParameters {
    fn default() -> Self {
        Self { version: 1, min_inclusion_slot: 0, max_inclusion_slot: 0, flags: 0 }
    }
}

/// XGA commitment that cryptographically ties to a validator registration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XGACommitment {
    /// Hash of the validator registration this commitment is tied to
    #[serde(with = "commitment_hash_serde")]
    pub registration_hash: CommitmentHash,

    /// Validator's BLS public key
    pub validator_pubkey: BlsPublicKey,

    /// Relay identifier this commitment is for (fixed size for SSZ)
    #[serde(with = "relay_id_serde")]
    pub relay_id: RelayId,

    /// XGA protocol version
    pub xga_version: u64,

    /// XGA-specific parameters
    pub parameters: XGAParameters,

    /// Unix timestamp when this commitment was created
    pub timestamp: u64,

    /// Chain ID for replay protection across chains
    pub chain_id: u64,

    /// Module signing domain for XGA
    pub signing_domain: [u8; 32],
}

/// Signed XGA commitment ready to be sent to relay
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedXGACommitment {
    pub message: XGACommitment,
    pub signature: BlsSignature,
}

/// Registration data we receive via webhook
#[derive(Debug, Clone, Deserialize)]
pub struct RegistrationNotification {
    /// The validator registration data
    pub registration: ValidatorRegistration,

    /// The relay this registration was sent to
    pub relay_url: String,

    /// Timestamp when the registration was sent
    pub timestamp: u64,
}

impl XGACommitment {
    /// Get the tree hash root for this commitment
    pub fn get_tree_hash_root(&self) -> tree_hash::Hash256 {
        // Call the manually implemented tree_hash_root directly
        <Self as tree_hash::TreeHash>::tree_hash_root(self)
    }

    /// Create a new XGA commitment from registration data
    pub fn new(
        registration_hash: [u8; 32],
        validator_pubkey: BlsPublicKey,
        relay_id: String,
        chain_id: u64,
        parameters: XGAParameters,
    ) -> Self {
        Self {
            registration_hash: CommitmentHash::from_bytes(registration_hash),
            validator_pubkey,
            relay_id: RelayId::from_url(&relay_id),
            xga_version: 1,
            parameters,
            timestamp: get_current_timestamp().unwrap_or_else(|e| {
                tracing::error!("Failed to get current timestamp: {}", e);
                0
            }),
            chain_id,
            signing_domain: XGA_SIGNING_DOMAIN,
        }
    }

    /// Compute the hash of a registration for linking
    pub fn hash_registration(registration: &ValidatorRegistration) -> [u8; 32] {
        use sha2::{Digest, Sha256};

        // SSZ encode the registration data manually
        // SSZ encoding for fixed-size container with fields:
        // - pubkey: 48 bytes
        // - fee_recipient: 20 bytes
        // - gas_limit: 8 bytes (little-endian)
        // - timestamp: 8 bytes (little-endian)
        // - signature: 96 bytes
        let mut ssz_bytes = Vec::with_capacity(48 + 20 + 8 + 8 + 96);

        // Append fields in order
        ssz_bytes.extend_from_slice(&registration.message.pubkey.0);
        ssz_bytes.extend_from_slice(registration.message.fee_recipient.as_slice());
        ssz_bytes.extend_from_slice(&registration.message.gas_limit.to_le_bytes());
        ssz_bytes.extend_from_slice(&registration.message.timestamp.to_le_bytes());
        ssz_bytes.extend_from_slice(&registration.signature.0);

        // Hash the SSZ encoded bytes
        let mut hasher = Sha256::new();
        hasher.update(&ssz_bytes);

        let result = hasher.finalize();
        let mut hash = [0u8; 32];
        hash.copy_from_slice(&result);
        hash
    }
}

// Manual TreeHash implementation for XGACommitment
impl tree_hash::TreeHash for XGACommitment {
    fn tree_hash_root(&self) -> tree_hash::Hash256 {
        // Create a vector with all the leaf hashes
        let leaves = vec![
            tree_hash::Hash256::from_slice(self.registration_hash.as_bytes()),
            self.validator_pubkey.tree_hash_root(),
            tree_hash::Hash256::from_slice(self.relay_id.as_bytes()),
            self.xga_version.tree_hash_root(),
            // Hash parameters fields individually
            self.parameters.version.tree_hash_root(),
            self.parameters.min_inclusion_slot.tree_hash_root(),
            self.parameters.max_inclusion_slot.tree_hash_root(),
            self.parameters.flags.tree_hash_root(),
            self.timestamp.tree_hash_root(),
            self.chain_id.tree_hash_root(),
            tree_hash::Hash256::from_slice(&self.signing_domain),
        ];

        // Calculate the merkle root using tree_hash utilities
        // Convert leaves to bytes for merkle_root function
        let mut bytes = Vec::with_capacity(leaves.len() * 32);
        for leaf in &leaves {
            bytes.extend_from_slice(&leaf.0);
        }

        tree_hash::merkle_root(&bytes, leaves.len())
    }

    fn tree_hash_type() -> tree_hash::TreeHashType {
        tree_hash::TreeHashType::Container
    }

    fn tree_hash_packed_encoding(&self) -> tree_hash::PackedEncoding {
        unreachable!("Container types are not packed")
    }

    fn tree_hash_packing_factor() -> usize {
        unreachable!("Container types are not packed")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_commitment_creation_with_relay_id_hashing() {
        let registration_hash = [0x42u8; 32];
        let validator_pubkey = BlsPublicKey::from([0x01u8; 48]);

        // Test that same relay URL produces same relay_id
        let commitment1 = XGACommitment::new(
            registration_hash,
            validator_pubkey,
            "https://relay.example.com".to_string(),
            1,
            XGAParameters::default(),
        );

        let commitment2 = XGACommitment::new(
            registration_hash,
            validator_pubkey,
            "https://relay.example.com".to_string(),
            1,
            XGAParameters::default(),
        );

        assert_eq!(commitment1.relay_id, commitment2.relay_id);

        // Test that different relay URLs produce different relay_ids
        let commitment3 = XGACommitment::new(
            registration_hash,
            validator_pubkey,
            "https://different-relay.example.com".to_string(),
            1,
            XGAParameters::default(),
        );

        assert_ne!(commitment1.relay_id, commitment3.relay_id);

        // Verify timestamps exist
        assert!(commitment1.timestamp > 0);
        assert!(commitment2.timestamp > 0);
        assert!(commitment3.timestamp > 0);
    }

    #[test]
    fn test_hash_registration_ssz_encoding() {
        use alloy_rpc_types::beacon::relay::{ValidatorRegistration, ValidatorRegistrationMessage};

        // Create a test registration
        let pubkey_bytes = [0x01u8; 48];
        let fee_recipient = alloy::primitives::FixedBytes::from([0x02u8; 20]);
        let gas_limit = 30_000_000u64;
        let timestamp = 1_700_000_000u64;
        let signature_bytes = [0x03u8; 96];

        let registration = ValidatorRegistration {
            message: ValidatorRegistrationMessage {
                pubkey: alloy_rpc_types::beacon::BlsPublicKey::from(pubkey_bytes),
                fee_recipient: alloy::primitives::Address::from(fee_recipient),
                gas_limit,
                timestamp,
            },
            signature: alloy_rpc_types::beacon::BlsSignature::from(signature_bytes),
        };

        let hash1 = XGACommitment::hash_registration(&registration);

        // Hash should be deterministic
        let hash2 = XGACommitment::hash_registration(&registration);
        assert_eq!(hash1, hash2);

        // Different registration should produce different hash
        let mut different_registration = registration.clone();
        different_registration.message.gas_limit = 25_000_000;
        let hash3 = XGACommitment::hash_registration(&different_registration);
        assert_ne!(hash1, hash3);

        // Verify SSZ encoding length
        let expected_ssz_len = 48 + 20 + 8 + 8 + 96; // 180 bytes
        let mut ssz_bytes = Vec::with_capacity(expected_ssz_len);
        ssz_bytes.extend_from_slice(&registration.message.pubkey.0);
        ssz_bytes.extend_from_slice(registration.message.fee_recipient.as_slice());
        ssz_bytes.extend_from_slice(&registration.message.gas_limit.to_le_bytes());
        ssz_bytes.extend_from_slice(&registration.message.timestamp.to_le_bytes());
        ssz_bytes.extend_from_slice(&registration.signature.0);
        assert_eq!(ssz_bytes.len(), expected_ssz_len);
    }

    #[test]
    fn test_tree_hash_deterministic() {
        let commitment = XGACommitment::new(
            [0x42u8; 32],
            BlsPublicKey::from([0x01u8; 48]),
            "test-relay".to_string(),
            1,
            XGAParameters {
                version: 1,
                min_inclusion_slot: 100,
                max_inclusion_slot: 200,
                flags: 0,
            },
        );

        // Tree hash should be deterministic
        let hash1 = commitment.get_tree_hash_root();
        let hash2 = commitment.get_tree_hash_root();
        assert_eq!(hash1, hash2);

        // Changing any field should change the tree hash
        let mut modified = commitment.clone();
        modified.xga_version = 2;
        let hash3 = modified.get_tree_hash_root();
        assert_ne!(hash1, hash3);

        // Changing the timestamp should change the hash
        let mut modified2 = commitment.clone();
        modified2.timestamp += 1;
        let hash4 = modified2.get_tree_hash_root();
        assert_ne!(hash1, hash4);
    }
}
