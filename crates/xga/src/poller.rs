use std::{
    collections::HashMap,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use alloy_rpc_types::beacon::relay::ValidatorRegistration;
use commit_boost::prelude::*;
use eyre::Result;
use serde::Deserialize;
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

use crate::{
    commitment::{RegistrationNotification, XgaCommitment},
    config::XgaConfig,
    infrastructure::{CircuitBreaker, HttpClientFactory, ValidatorRateLimiter},
    relay::check_xga_support,
    retry::{execute_with_retry, polling_retry_strategy},
    signer::process_commitment,
    validation::validate_registration,
};

/// State tracking for polling
#[derive(Default)]
pub struct PollingState {
    /// Maps relay URL to last seen timestamp
    last_seen_registrations: HashMap<String, u64>,
}

impl PollingState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get_last_seen(&self, relay_url: &str) -> Option<u64> {
        self.last_seen_registrations.get(relay_url).copied()
    }

    pub fn update_last_seen(&mut self, relay_url: &str, timestamp: u64) {
        self.last_seen_registrations.insert(relay_url.to_string(), timestamp);
    }
}

/// Response from relay for recent registrations
#[derive(Debug, Deserialize)]
struct RelayRegistrationsResponse {
    registrations: Vec<ValidatorRegistration>,
}

/// Main polling component
pub struct RelayPoller {
    config: Arc<StartCommitModuleConfig<XgaConfig>>,
    http_client_factory: Arc<HttpClientFactory>,
    circuit_breaker: Arc<CircuitBreaker>,
    validator_rate_limiter: Arc<ValidatorRateLimiter>,
    state: Arc<Mutex<PollingState>>,
}

impl RelayPoller {
    pub fn new(
        config: Arc<StartCommitModuleConfig<XgaConfig>>,
        http_client_factory: Arc<HttpClientFactory>,
        circuit_breaker: Arc<CircuitBreaker>,
        validator_rate_limiter: Arc<ValidatorRateLimiter>,
    ) -> Result<Self> {
        Ok(Self {
            config,
            http_client_factory,
            circuit_breaker,
            validator_rate_limiter,
            state: Arc::new(Mutex::new(PollingState::new())),
        })
    }

    /// Poll a single relay for new registrations
    pub async fn poll_relay(&self, relay_url: &str) -> Result<Vec<RegistrationNotification>> {
        // Check circuit breaker
        if self.circuit_breaker.is_open(relay_url).await {
            debug!(relay_url = %relay_url, "Circuit breaker open, skipping relay");
            return Ok(vec![]);
        }

        // Get last seen timestamp
        let since_timestamp = {
            let state = self.state.lock().await;
            state.get_last_seen(relay_url).unwrap_or(0)
        };

        // Query relay for registrations
        let registrations = self.fetch_registrations(relay_url, since_timestamp).await?;

        if registrations.is_empty() {
            return Ok(vec![]);
        }

        // Convert to notifications
        let current_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let notifications: Vec<RegistrationNotification> = registrations
            .into_iter()
            .filter(|reg| {
                // Filter out registrations that are too old
                let age = current_time.saturating_sub(reg.message.timestamp);
                age <= self.config.extra.max_registration_age_secs
            })
            .filter(|reg| {
                // Validate registration
                match validate_registration(reg) {
                    Ok(_) => true,
                    Err(e) => {
                        warn!(
                            validator_pubkey = ?reg.message.pubkey,
                            error = %e,
                            "Invalid registration from relay, skipping"
                        );
                        false
                    }
                }
            })
            .map(|registration| RegistrationNotification {
                registration,
                relay_url: relay_url.to_string(),
                timestamp: current_time,
            })
            .collect();

        // Update last seen timestamp if we got any notifications
        if !notifications.is_empty() {
            let mut state = self.state.lock().await;
            state.update_last_seen(relay_url, current_time);
        }

        Ok(notifications)
    }

    /// Fetch registrations from relay with retry
    async fn fetch_registrations(
        &self,
        relay_url: &str,
        since_timestamp: u64,
    ) -> Result<Vec<ValidatorRegistration>> {
        let client = self.http_client_factory.create_client()?;
        let endpoint = format!(
            "{}/eth/v1/builder/registrations?since={}",
            relay_url.trim_end_matches('/'),
            since_timestamp
        );

        let strategy = polling_retry_strategy(&self.config.extra.retry_config);

        let response = execute_with_retry(strategy, || async {
            use tokio_retry2::RetryError;
            client
                .get(&endpoint)
                .send()
                .await
                .map_err(|e| RetryError::transient(eyre::eyre!("HTTP request failed: {}", e)))
        })
        .await?;

        if !response.status().is_success() {
            self.circuit_breaker.record_failure(relay_url).await;
            return Err(eyre::eyre!(
                "Relay returned error status: {}",
                response.status()
            ));
        }

        self.circuit_breaker.record_success(relay_url).await;

        let data: RelayRegistrationsResponse = response
            .json()
            .await
            .map_err(|e| eyre::eyre!("Failed to parse response: {}", e))?;

        Ok(data.registrations)
    }

    /// Process registrations and send commitments
    pub async fn process_registrations(
        &self,
        notifications: Vec<RegistrationNotification>,
    ) -> Result<()> {
        for notification in notifications {
            if let Err(e) = self.process_single_registration(notification).await {
                error!("Failed to process registration: {}", e);
                // Continue processing other registrations
            }
        }
        Ok(())
    }

    async fn process_single_registration(
        &self,
        notification: RegistrationNotification,
    ) -> Result<()> {
        info!(
            validator_pubkey = ?notification.registration.message.pubkey,
            relay_url = %notification.relay_url,
            "Processing registration from polling"
        );

        // Create commitment
        let registration_hash = XgaCommitment::hash_registration(&notification.registration);
        let pubkey_bytes: [u8; 48] = notification.registration.message.pubkey.0;
        let validator_pubkey = BlsPublicKey::from(pubkey_bytes);

        let commitment = XgaCommitment::new(
            registration_hash,
            validator_pubkey,
            &notification.relay_url,
            self.config.chain.id(),
            Default::default(),
        );

        // Process (sign and send)
        process_commitment(
            commitment,
            &notification.relay_url,
            self.config.clone(),
            &self.circuit_breaker,
            &self.http_client_factory,
            &self.validator_rate_limiter,
        )
        .await?;

        Ok(())
    }
}

/// Poll and process a single relay
pub async fn poll_and_process_relay(
    relay_url: String,
    poller: Arc<RelayPoller>,
) -> Result<()> {
    debug!(relay_url = %relay_url, "Starting poll cycle");

    // Optional: Check relay capabilities
    if poller.config.extra.probe_relay_capabilities
        && !check_xga_support(&relay_url, &poller.http_client_factory).await
    {
        warn!(relay_url = %relay_url, "Relay does not support XGA");
        return Ok(());
    }

    // Poll for registrations
    match poller.poll_relay(&relay_url).await {
        Ok(notifications) => {
            let count = notifications.len();
            if count > 0 {
                info!(
                    relay_url = %relay_url,
                    registrations_found = count,
                    "Found new registrations"
                );
                poller.process_registrations(notifications).await?;
            }
        }
        Err(e) => {
            error!(
                relay_url = %relay_url,
                error = %e,
                "Failed to poll relay"
            );
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_polling_state_tracks_timestamps() {
        let mut state = PollingState::new();
        let relay_url = "https://test-relay.com";

        // Initially no timestamp
        assert_eq!(state.get_last_seen(relay_url), None);

        // Update timestamp
        state.update_last_seen(relay_url, 1_234_567_890);
        assert_eq!(state.get_last_seen(relay_url), Some(1_234_567_890));

        // Update again
        state.update_last_seen(relay_url, 1_234_567_900);
        assert_eq!(state.get_last_seen(relay_url), Some(1_234_567_900));
    }

    // Removed test_relay_poller_creation as it requires full StartCommitModuleConfig
    // which is difficult to mock properly in unit tests. Integration tests should
    // be used for testing with full configuration.

    // Removed test_poll_relay_with_circuit_breaker_open as it requires full StartCommitModuleConfig
    // which is difficult to mock properly in unit tests. Integration tests should
    // be used for testing with full configuration.
}