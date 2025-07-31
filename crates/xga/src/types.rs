//! Type-safe wrappers for common byte arrays used throughout the XGA module

use std::fmt;

use sha2::{Digest, Sha256};

/// Type-safe wrapper for relay identifiers
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RelayId([u8; 32]);

impl RelayId {
    /// Create a new RelayId from raw bytes
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Create a RelayId from a URL string
    pub fn from_url(url: &str) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(url.as_bytes());
        Self(hasher.finalize().into())
    }

    /// Get the raw bytes
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Convert to owned byte array
    pub const fn into_bytes(self) -> [u8; 32] {
        self.0
    }
}

impl fmt::Display for RelayId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", hex::encode(self.0))
    }
}

impl From<[u8; 32]> for RelayId {
    fn from(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

impl AsRef<[u8]> for RelayId {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

/// Type-safe wrapper for commitment hashes
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CommitmentHash([u8; 32]);

impl CommitmentHash {
    /// Create a new CommitmentHash from raw bytes
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Get the raw bytes
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Convert to owned byte array
    pub const fn into_bytes(self) -> [u8; 32] {
        self.0
    }
}

impl fmt::Display for CommitmentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", hex::encode(self.0))
    }
}

impl From<[u8; 32]> for CommitmentHash {
    fn from(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

impl AsRef<[u8]> for CommitmentHash {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_relay_id_from_url() {
        let url1 = "https://relay.example.com";
        let url2 = "https://different-relay.example.com";

        let id1 = RelayId::from_url(url1);
        let id2 = RelayId::from_url(url2);

        // Same URL should produce same ID
        assert_eq!(id1, RelayId::from_url(url1));

        // Different URLs should produce different IDs
        assert_ne!(id1, id2);
    }


    #[test]
    fn test_type_conversions() {
        let bytes = [0x42u8; 32];

        let relay_id = RelayId::from_bytes(bytes);
        assert_eq!(relay_id.as_bytes(), &bytes);
        assert_eq!(relay_id.into_bytes(), bytes);

        let commitment_hash = CommitmentHash::from(bytes);
        assert_eq!(commitment_hash.as_bytes(), &bytes);
    }
}
