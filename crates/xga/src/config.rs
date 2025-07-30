use crate::eigenlayer::EigenLayerConfig;
use crate::infrastructure::parse_and_validate_url;
use eyre;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct XGAConfig {
    /// Port for the webhook server
    pub webhook_port: u16,

    /// List of XGA-enabled relay URLs
    pub xga_relays: Vec<String>,

    /// Delay in milliseconds before sending commitment after registration
    #[serde(default = "default_commitment_delay_ms")]
    pub commitment_delay_ms: u64,

    /// Number of retry attempts for failed commitments
    #[serde(default = "default_retry_attempts")]
    pub retry_attempts: u32,

    /// Delay between retry attempts in milliseconds
    #[serde(default = "default_retry_delay_ms")]
    pub retry_delay_ms: u64,

    /// Maximum age of registration in seconds to process
    #[serde(default = "default_max_registration_age_secs")]
    pub max_registration_age_secs: u64,

    /// Whether to probe relay capabilities at runtime
    #[serde(default = "default_probe_relay_capabilities")]
    pub probe_relay_capabilities: bool,

    /// EigenLayer integration configuration
    #[serde(default)]
    pub eigenlayer: EigenLayerConfig,
}

fn default_commitment_delay_ms() -> u64 {
    100
}

fn default_retry_attempts() -> u32 {
    3
}

fn default_retry_delay_ms() -> u64 {
    1000
}

fn default_max_registration_age_secs() -> u64 {
    60
}

fn default_probe_relay_capabilities() -> bool {
    false
}

impl XGAConfig {
    /// Check if a relay URL is XGA-enabled
    pub fn is_xga_relay(&self, relay_url: &str) -> bool {
        self.xga_relays.iter().any(|url| url == relay_url)
    }

    /// Validate the configuration
    pub fn validate(&self) -> eyre::Result<()> {
        // Validate port range
        if self.webhook_port < 1024 {
            return Err(eyre::eyre!("Webhook port must be >= 1024 for non-root operation"));
        }

        // Validate delays and timeouts
        if self.commitment_delay_ms < 1 || self.commitment_delay_ms > 60000 {
            return Err(eyre::eyre!("Commitment delay must be between 1ms and 60 seconds"));
        }

        if self.retry_delay_ms < 100 || self.retry_delay_ms > 60000 {
            return Err(eyre::eyre!("Retry delay must be between 100ms and 60 seconds"));
        }

        if self.retry_attempts < 1 || self.retry_attempts > 10 {
            return Err(eyre::eyre!("Retry attempts must be between 1 and 10"));
        }

        if self.max_registration_age_secs < 1 || self.max_registration_age_secs > 600 {
            return Err(eyre::eyre!(
                "Max registration age must be between 1 second and 10 minutes"
            ));
        }

        // Validate at least one XGA relay is configured
        if self.xga_relays.is_empty() {
            return Err(eyre::eyre!("At least one XGA relay must be configured"));
        }

        // Validate all relay URLs using infrastructure module
        for relay_url in &self.xga_relays {
            parse_and_validate_url(relay_url)
                .map_err(|e| eyre::eyre!("Invalid XGA relay URL: {}", e))?;
        }

        Ok(())
    }
}
