use crate::config::RelayGasConfig;
use crate::error::{ReservedGasError, Result};
use crate::validation::validate_relay_gas_config;
use alloy::rpc::types::beacon::relay::ValidatorRegistration;
use async_trait::async_trait;
use axum::http::StatusCode;
use cb_common::pbs::{
    GetHeaderParams, GetHeaderResponse, RelayClient, SignedBlindedBeaconBlock,
    SubmitBlindedBlockResponse,
};
use std::time::Duration;
use tracing::{debug, warn};

const DEFAULT_TIMEOUT: Duration = Duration::from_millis(950);
const REGISTER_TIMEOUT: Duration = Duration::from_millis(3000);
const SUBMIT_TIMEOUT: Duration = Duration::from_millis(4000);
const STATUS_TIMEOUT: Duration = Duration::from_millis(1000);
const CONFIG_TIMEOUT: Duration = Duration::from_secs(5);
const RESERVE_TIMEOUT: Duration = Duration::from_secs(2);

/// Trait for relay communication operations
#[async_trait]
pub trait RelayQuerier: Send + Sync {
    async fn query_header(
        &self,
        relay: &RelayClient,
        params: GetHeaderParams,
    ) -> Result<Option<GetHeaderResponse>>;

    async fn register_validators(
        &self,
        relay: &RelayClient,
        registrations: &[ValidatorRegistration],
    ) -> Result<()>;

    async fn submit_block(
        &self,
        relay: &RelayClient,
        block: &SignedBlindedBeaconBlock,
    ) -> Result<SubmitBlindedBlockResponse>;

    async fn check_status(&self, relay: &RelayClient) -> Result<()>;

    async fn fetch_gas_config(
        &self,
        relay: &RelayClient,
        endpoint: &str,
    ) -> Result<Option<RelayGasConfig>>;

    async fn fetch_reserve_requirement(
        &self,
        relay: &RelayClient,
        endpoint: &str,
    ) -> Result<Option<u64>>;
}

/// Production implementation of RelayQuerier
pub struct HttpRelayQuerier;

#[async_trait]
impl RelayQuerier for HttpRelayQuerier {
    async fn query_header(
        &self,
        relay: &RelayClient,
        params: GetHeaderParams,
    ) -> Result<Option<GetHeaderResponse>> {
        let url = relay
            .get_header_url(params.slot, params.parent_hash, params.pubkey)
            .map_err(|e| ReservedGasError::ConfigError(e.to_string()))?;

        debug!(relay_id = %relay.id, url = %url, "Fetching header from relay");

        let response = relay.client.get(url).timeout(DEFAULT_TIMEOUT).send().await?;

        match response.status() {
            status if status.is_success() => {
                let header: GetHeaderResponse = response.json().await?;
                Ok(Some(header))
            }
            status if status.as_u16() == 204 => {
                // No content - relay has no bid
                Ok(None)
            }
            status => {
                warn!(
                    relay_id = %relay.id,
                    status = %status,
                    "Unexpected status from relay get_header"
                );
                Err(ReservedGasError::ConfigError(format!(
                    "Unexpected status from relay: {}",
                    status
                )))
            }
        }
    }

    async fn register_validators(
        &self,
        relay: &RelayClient,
        registrations: &[ValidatorRegistration],
    ) -> Result<()> {
        let url = relay
            .register_validator_url()
            .map_err(|e| ReservedGasError::ConfigError(e.to_string()))?;

        debug!(
            relay_id = %relay.id,
            count = registrations.len(),
            "Registering validators with relay"
        );

        let response =
            relay.client.post(url).json(registrations).timeout(REGISTER_TIMEOUT).send().await?;

        if !response.status().is_success() {
            let status = response.status();
            let _body = response.text().await.unwrap_or_default();
            return Err(ReservedGasError::ConfigError(format!(
                "Failed to register validators: {}",
                status
            )));
        }

        Ok(())
    }

    async fn submit_block(
        &self,
        relay: &RelayClient,
        block: &SignedBlindedBeaconBlock,
    ) -> Result<SubmitBlindedBlockResponse> {
        let url =
            relay.submit_block_url().map_err(|e| ReservedGasError::ConfigError(e.to_string()))?;

        debug!(
            relay_id = %relay.id,
            "Submitting blinded block to relay"
        );

        let response = relay.client.post(url).json(block).timeout(SUBMIT_TIMEOUT).send().await?;

        if !response.status().is_success() {
            let status = response.status();
            let _body = response.text().await.unwrap_or_default();
            return Err(ReservedGasError::ConfigError(format!(
                "Failed to submit block: {}",
                status
            )));
        }

        let result: SubmitBlindedBlockResponse = response.json().await?;
        Ok(result)
    }

    async fn check_status(&self, relay: &RelayClient) -> Result<()> {
        let url =
            relay.get_status_url().map_err(|e| ReservedGasError::ConfigError(e.to_string()))?;

        debug!(relay_id = %relay.id, "Checking relay status");

        let response = relay.client.get(url).timeout(STATUS_TIMEOUT).send().await?;

        if response.status().is_success() {
            Ok(())
        } else {
            Err(ReservedGasError::ConfigError(format!(
                "Relay unhealthy: status {}",
                response.status()
            )))
        }
    }

    async fn fetch_gas_config(
        &self,
        relay: &RelayClient,
        endpoint: &str,
    ) -> Result<Option<RelayGasConfig>> {
        let base_url = &relay.config.entry.url;
        let url = base_url
            .join(endpoint)
            .map_err(|e| ReservedGasError::ConfigError(format!("Invalid URL: {}", e)))?;

        debug!(relay_id = %relay.id, url = %url, "Fetching gas config from relay");

        let response = relay.client.get(url).timeout(CONFIG_TIMEOUT).send().await?;

        match response.status() {
            StatusCode::OK => {
                let config = response.json::<RelayGasConfig>().await?;

                // Validate the response
                validate_relay_gas_config(&config, &relay.id)?;

                Ok(Some(config))
            }
            StatusCode::NOT_FOUND => {
                // Relay doesn't support this endpoint
                Ok(None)
            }
            status => {
                warn!(
                    relay_id = %relay.id,
                    status = %status,
                    "Unexpected status from relay gas config endpoint"
                );
                Err(ReservedGasError::ConfigError(format!(
                    "Unexpected status from relay gas config endpoint: {}",
                    status
                )))
            }
        }
    }

    async fn fetch_reserve_requirement(
        &self,
        relay: &RelayClient,
        endpoint: &str,
    ) -> Result<Option<u64>> {
        let base_url = &relay.config.entry.url;
        let url = base_url
            .join(endpoint)
            .map_err(|e| ReservedGasError::ConfigError(format!("Invalid URL: {}", e)))?;

        debug!(relay_id = %relay.id, url = %url, "Fetching reserve requirement from relay");

        let response = relay.client.get(url).timeout(RESERVE_TIMEOUT).send().await?;

        match response.status() {
            StatusCode::OK => {
                // Parse simple JSON response: {"reserve": 2000000}
                let json: serde_json::Value = response.json().await?;

                let reserve = json.get("reserve").and_then(|v| v.as_u64()).ok_or_else(|| {
                    ReservedGasError::ConfigError(
                        "Invalid reserve response: missing or invalid 'reserve' field".to_string(),
                    )
                })?;

                // Validate the reserve value is reasonable
                if reserve > crate::validation::MAX_REASONABLE_GAS {
                    return Err(ReservedGasError::ReservedGasTooHigh {
                        amount: reserve,
                        max: crate::validation::MAX_REASONABLE_GAS,
                    });
                }

                Ok(Some(reserve))
            }
            StatusCode::NOT_FOUND => {
                // Relay doesn't support this endpoint
                Ok(None)
            }
            status => {
                warn!(
                    relay_id = %relay.id,
                    status = %status,
                    "Unexpected status from relay reserve endpoint"
                );
                Err(ReservedGasError::ConfigError(format!(
                    "Unexpected status from relay reserve endpoint: {}",
                    status
                )))
            }
        }
    }
}

/// Service for managing relay queries
pub struct RelayQueryService {
    querier: Box<dyn RelayQuerier>,
}

impl RelayQueryService {
    pub fn new(querier: Box<dyn RelayQuerier>) -> Self {
        Self { querier }
    }

    pub fn production() -> Self {
        Self::new(Box::new(HttpRelayQuerier))
    }

    pub async fn query_header(
        &self,
        relay: &RelayClient,
        params: GetHeaderParams,
    ) -> Result<Option<GetHeaderResponse>> {
        self.querier.query_header(relay, params).await
    }

    pub async fn register_validators(
        &self,
        relay: &RelayClient,
        registrations: &[ValidatorRegistration],
    ) -> Result<()> {
        self.querier.register_validators(relay, registrations).await
    }

    pub async fn submit_block(
        &self,
        relay: &RelayClient,
        block: &SignedBlindedBeaconBlock,
    ) -> Result<SubmitBlindedBlockResponse> {
        self.querier.submit_block(relay, block).await
    }

    pub async fn check_status(&self, relay: &RelayClient) -> Result<()> {
        self.querier.check_status(relay).await
    }

    pub async fn fetch_gas_config(
        &self,
        relay: &RelayClient,
        endpoint: &str,
    ) -> Result<Option<RelayGasConfig>> {
        self.querier.fetch_gas_config(relay, endpoint).await
    }

    pub async fn fetch_reserve_requirement(
        &self,
        relay: &RelayClient,
        endpoint: &str,
    ) -> Result<Option<u64>> {
        self.querier.fetch_reserve_requirement(relay, endpoint).await
    }
}
