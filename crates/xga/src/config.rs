use eyre;
use serde::Deserialize;

use crate::{eigenlayer::EigenLayerConfig, infrastructure::parse_and_validate_url};

#[derive(Debug, Clone, Deserialize)]
pub struct RetryConfig {
    /// Maximum number of retry attempts
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    
    /// Initial backoff in milliseconds
    #[serde(default = "default_initial_backoff_ms")]
    pub initial_backoff_ms: u64,
    
    /// Maximum backoff in seconds
    #[serde(default = "default_max_backoff_secs")]
    pub max_backoff_secs: u64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: default_max_retries(),
            initial_backoff_ms: default_initial_backoff_ms(),
            max_backoff_secs: default_max_backoff_secs(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct XGAConfig {
    /// Polling interval in seconds
    #[serde(default = "default_polling_interval_secs")]
    pub polling_interval_secs: u64,

    /// List of XGA-enabled relay URLs
    pub xga_relays: Vec<String>,

    /// Maximum age of registration in seconds to process
    #[serde(default = "default_max_registration_age_secs")]
    pub max_registration_age_secs: u64,

    /// Whether to probe relay capabilities at runtime
    #[serde(default = "default_probe_relay_capabilities")]
    pub probe_relay_capabilities: bool,

    /// Retry configuration
    #[serde(default)]
    pub retry_config: RetryConfig,

    /// EigenLayer integration configuration
    #[serde(default)]
    pub eigenlayer: EigenLayerConfig,
}

fn default_polling_interval_secs() -> u64 {
    5
}

fn default_max_registration_age_secs() -> u64 {
    60
}

fn default_probe_relay_capabilities() -> bool {
    false
}

fn default_max_retries() -> u32 {
    3
}

fn default_initial_backoff_ms() -> u64 {
    100
}

fn default_max_backoff_secs() -> u64 {
    5
}

impl XGAConfig {
    /// Check if a relay URL is XGA-enabled
    pub fn is_xga_relay(&self, relay_url: &str) -> bool {
        self.xga_relays.iter().any(|url| url == relay_url)
    }

    /// Validate the configuration
    pub fn validate(&self) -> eyre::Result<()> {
        // Validate polling interval
        if self.polling_interval_secs < 1 || self.polling_interval_secs > 3600 {
            return Err(eyre::eyre!(
                "Polling interval must be between 1 second and 1 hour"
            ));
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

        // Validate retry config
        if self.retry_config.max_retries > 10 {
            return Err(eyre::eyre!("Max retries must be <= 10"));
        }

        if self.retry_config.initial_backoff_ms < 10 || self.retry_config.initial_backoff_ms > 60000 {
            return Err(eyre::eyre!(
                "Initial backoff must be between 10ms and 60 seconds"
            ));
        }

        if self.retry_config.max_backoff_secs > 300 {
            return Err(eyre::eyre!("Max backoff must be <= 5 minutes"));
        }

        Ok(())
    }
}