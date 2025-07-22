use eyre::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ReservedGasConfig {
    /// Default reserved gas limit to apply to all relays (in gas units)
    #[serde(default = "default_reserved_gas")]
    pub default_reserved_gas: u64,

    /// Per-relay gas limit overrides (relay_id -> reserved_gas)
    #[serde(default)]
    pub relay_overrides: HashMap<String, u64>,

    /// Interval in seconds to fetch gas config updates from relays
    #[serde(default = "default_update_interval")]
    pub update_interval_secs: u64,

    /// Minimum block gas limit to enforce (safety check)
    #[serde(default = "default_min_gas_limit")]
    pub min_block_gas_limit: u64,

    /// Optional endpoint path for fetching gas config from relays
    #[serde(default = "default_config_endpoint")]
    pub relay_config_endpoint: String,

    /// Whether to fetch gas config from relays
    #[serde(default = "default_fetch_from_relays")]
    pub fetch_from_relays: bool,

    /// Optional endpoint path for fetching reserve requirements from relays (default: /xga/v2/relay/reserve)
    #[serde(default = "default_reserve_endpoint")]
    pub relay_reserve_endpoint: String,
}

fn default_reserved_gas() -> u64 {
    1_000_000 // 1M gas reserved by default
}

fn default_update_interval() -> u64 {
    60 // Update every minute
}

fn default_min_gas_limit() -> u64 {
    10_000_000 // 10M gas minimum
}

fn default_config_endpoint() -> String {
    "/relay/v1/gas_config".to_string()
}

fn default_fetch_from_relays() -> bool {
    true
}

fn default_reserve_endpoint() -> String {
    "/xga/v2/relay/reserve".to_string()
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RelayGasConfig {
    /// Reserved gas limit advertised by the relay
    pub reserved_gas_limit: u64,

    /// Optional update interval hint from relay
    pub update_interval: Option<u64>,

    /// Optional minimum gas limit requirement from relay
    pub min_gas_limit: Option<u64>,
}

impl ReservedGasConfig {
    /// Get the reserved gas for a specific relay
    pub fn get_reserved_gas(&self, relay_id: &str) -> u64 {
        self.relay_overrides.get(relay_id).copied().unwrap_or(self.default_reserved_gas)
    }

    /// Validate that a gas limit after reservation meets minimum requirements
    pub fn validate_gas_limit(&self, original_gas_limit: u64, reserved: u64) -> Result<u64> {
        // Use the validation function from validation module
        crate::validation::validate_gas_after_reservation(
            original_gas_limit,
            reserved,
            self.min_block_gas_limit,
        )
        .map_err(|e| eyre::eyre!(e))
    }

    /// Validate the configuration on startup
    pub fn validate(&self) -> Result<()> {
        // Validate update interval is reasonable (at least 10 seconds, at most 1 hour)
        if self.update_interval_secs < 10 {
            return Err(eyre::eyre!("Update interval must be at least 10 seconds"));
        }
        if self.update_interval_secs > 3600 {
            return Err(eyre::eyre!("Update interval must not exceed 1 hour (3600 seconds)"));
        }

        // Validate endpoints are valid paths
        if !self.relay_config_endpoint.starts_with('/') {
            return Err(eyre::eyre!("Relay config endpoint must start with '/'"));
        }
        if !self.relay_reserve_endpoint.starts_with('/') {
            return Err(eyre::eyre!("Relay reserve endpoint must start with '/'"));
        }

        // Validate gas limits
        crate::validation::validate_gas_limit(self.default_reserved_gas, "default reserved gas")?;

        // Validate relay overrides
        for (relay_id, reserved_gas) in &self.relay_overrides {
            crate::validation::validate_gas_limit(
                *reserved_gas,
                &format!("reserved gas for relay {}", relay_id),
            )?;
        }

        // Validate minimum block gas limit
        if self.min_block_gas_limit < crate::validation::MIN_BLOCK_GAS_LIMIT {
            return Err(eyre::eyre!(
                "Minimum block gas limit {} is below required minimum {}",
                self.min_block_gas_limit,
                crate::validation::MIN_BLOCK_GAS_LIMIT
            ));
        }

        Ok(())
    }
}
