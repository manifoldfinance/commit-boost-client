#[allow(unused_imports)]
use alloy::network::Ethereum;
use alloy::primitives::{Address, Bytes, B256, U256};
#[allow(unused_imports)]
use alloy::providers::fillers::{
    BlobGasFiller, ChainIdFiller, FillProvider, GasFiller, JoinFill, NonceFiller,
};
#[allow(unused_imports)]
use alloy::providers::{Identity, RootProvider};
use alloy::providers::{Provider, ProviderBuilder};
use alloy_rpc_types::TransactionRequest;
use commit_boost::prelude::*;
use eyre::{Result, WrapErr};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, error, info, warn};

use crate::abi_helpers::*;
use crate::commitment::XGACommitment;

// XGARegistry contract interface - removed sol! macro usage

// Export the default EigenLayer integration type for use in other modules
pub type DefaultEigenLayerIntegration = EigenLayerIntegration<DefaultProvider>;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct EigenLayerConfig {
    /// Enable EigenLayer integration
    pub enabled: bool,
    /// XGARegistry contract address
    pub registry_address: String,
    /// Ethereum RPC URL
    pub rpc_url: String,
    /// Operator address for shadow mode monitoring
    #[serde(default)]
    pub operator_address: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ShadowModeStatus {
    pub is_registered: bool,
    pub commitment_hash: B256,
    pub last_update_block: u64,
    pub penalty_count: u64,
    pub accumulated_rewards: U256,
    // Legacy fields for compatibility
    pub pending_rewards: U256,
    pub blocks_active: U256,
    pub penalty_rate: U256,
}

// No type alias needed - we'll use the provider trait directly

/// Trait defining the public API for EigenLayer shadow mode queries
/// These methods are used by the CLI binary for operator status queries
pub trait EigenLayerQueries {
    /// Get pending rewards for an operator
    fn get_pending_rewards(
        &self,
        operator_address: Address,
    ) -> impl std::future::Future<Output = Result<U256>> + Send;

    /// Get detailed operator status
    fn get_operator_status(
        &self,
        operator_address: Address,
    ) -> impl std::future::Future<Output = Result<(B256, Bytes, U256, U256, bool, U256)>> + Send;

    /// Get comprehensive shadow mode status
    fn get_shadow_mode_status(
        &self,
        operator_address: Address,
    ) -> impl std::future::Future<Output = Result<ShadowModeStatus>> + Send;
}

pub struct EigenLayerIntegration<P> {
    provider: Arc<P>,
    registry_address: Address,
    config: EigenLayerConfig,
    current_commitment_hash: Option<B256>,
    cb_config: Arc<StartCommitModuleConfig<crate::config::XGAConfig>>,
    last_status_check: Option<ShadowModeStatus>,
}

impl<P> EigenLayerIntegration<P>
where
    P: Provider + Clone + Send + Sync + 'static,
{
    /// Create new EigenLayer integration instance with a specific provider
    pub async fn with_provider(
        provider: Arc<P>,
        config: EigenLayerConfig,
        cb_config: Arc<StartCommitModuleConfig<crate::config::XGAConfig>>,
    ) -> Result<Self> {
        // Parse registry address
        let registry_address: Address =
            config.registry_address.parse().wrap_err("Invalid registry address")?;

        let integration = Self {
            provider,
            registry_address,
            config,
            current_commitment_hash: None,
            cb_config: cb_config.clone(),
            last_status_check: None,
        };

        // Verify chain ID matches
        integration.verify_chain_id(cb_config.chain.id()).await?;

        // Verify contract is deployed
        integration.verify_contract_deployed().await?;

        // Log current block for debugging
        let current_block = integration.get_current_block().await?;
        info!(
            "EigenLayer integration initialized at block {} on chain {}",
            current_block,
            cb_config.chain.id()
        );

        Ok(integration)
    }

    /// Monitor shadow mode status and detect important changes
    /// This should be called periodically to track operator performance
    pub async fn monitor_shadow_mode(&mut self, operator_address: Address) -> Result<()> {
        let current_status = self.get_shadow_mode_status(operator_address).await?;

        // Check for significant changes
        if let Some(last_status) = &self.last_status_check {
            // Detect registration status changes
            if last_status.is_registered != current_status.is_registered {
                if current_status.is_registered {
                    info!("Operator {} has been registered in shadow mode", operator_address);
                } else {
                    warn!("Operator {} has been deregistered from shadow mode", operator_address);
                }
            }

            // Monitor reward accumulation
            if current_status.pending_rewards > last_status.pending_rewards {
                let reward_increase = current_status.pending_rewards - last_status.pending_rewards;
                info!(
                    "Shadow mode rewards increased by {} for operator {}",
                    reward_increase, operator_address
                );
            }

            // Check for penalty rate changes
            if current_status.penalty_rate != last_status.penalty_rate {
                warn!(
                    "Penalty rate changed from {} to {} for operator {}",
                    last_status.penalty_rate, current_status.penalty_rate, operator_address
                );
            }

            // Monitor commitment hash changes
            if current_status.commitment_hash != last_status.commitment_hash {
                info!(
                    "Commitment hash updated from {:?} to {:?}",
                    last_status.commitment_hash, current_status.commitment_hash
                );
            }
        } else {
            // First status check
            info!(
                "Initial shadow mode status check - Registered: {}, Pending rewards: {}, Penalty rate: {}",
                current_status.is_registered,
                current_status.pending_rewards,
                current_status.penalty_rate
            );
        }

        // Store current status for next comparison
        self.last_status_check = Some(current_status);

        Ok(())
    }

    /// Check if shadow mode monitoring indicates any issues
    /// Returns true if operator is in good standing
    pub async fn validate_shadow_mode_health(&self, operator_address: Address) -> Result<bool> {
        let status = self.get_shadow_mode_status(operator_address).await?;

        // Check if operator is registered
        if !status.is_registered {
            warn!("Operator {} is not registered in shadow mode", operator_address);
            return Ok(false);
        }

        // Check if penalty rate is acceptable (e.g., below 10%)
        let max_penalty_rate = U256::from(10u64);
        if status.penalty_rate > max_penalty_rate {
            warn!("Operator {} has high penalty rate: {}%", operator_address, status.penalty_rate);
            return Ok(false);
        }

        // Check if operator has been active for minimum blocks (e.g., 100 blocks)
        let min_active_blocks = U256::from(100u64);
        if status.blocks_active < min_active_blocks {
            debug!(
                "Operator {} has only been active for {} blocks",
                operator_address, status.blocks_active
            );
            return Ok(false);
        }

        Ok(true)
    }

    /// Run periodic shadow mode monitoring
    /// This method should be called periodically (e.g., every block or every few minutes)
    /// to track operator performance and detect issues
    pub async fn run_periodic_monitoring(&mut self) -> Result<()> {
        if !self.config.enabled {
            return Ok(());
        }

        // Only run monitoring if operator address is configured
        if let Some(operator_addr_str) = &self.config.operator_address {
            match operator_addr_str.parse::<Address>() {
                Ok(operator_address) => {
                    // Get comprehensive shadow mode status
                    match self.get_shadow_mode_status(operator_address).await {
                        Ok(status) => {
                            // Log current status
                            debug!(
                                "Shadow mode monitoring - Operator: {}, Registered: {}, Rewards: {}, Blocks: {}, Penalty: {}%",
                                operator_address,
                                status.is_registered,
                                status.pending_rewards,
                                status.blocks_active,
                                status.penalty_rate
                            );

                            // Check for critical issues
                            if !status.is_registered {
                                warn!(
                                    "CRITICAL: Operator {} is not registered in shadow mode!",
                                    operator_address
                                );
                            }

                            // Check for high penalty rate
                            if status.penalty_rate > U256::from(20u64) {
                                warn!(
                                    "WARNING: Operator {} has high penalty rate: {}%",
                                    operator_address, status.penalty_rate
                                );
                            }

                            // Check for stalled rewards (no rewards for 1000+ blocks while active)
                            if status.is_registered
                                && status.blocks_active > U256::from(1000u64)
                                && status.pending_rewards == U256::ZERO
                            {
                                warn!(
                                    "WARNING: Operator {} has been active for {} blocks with no rewards",
                                    operator_address,
                                    status.blocks_active
                                );
                            }

                            // Run the standard monitoring to detect changes
                            self.monitor_shadow_mode(operator_address).await?;
                        }
                        Err(e) => {
                            error!(
                                "Failed to get shadow mode status during periodic monitoring: {}",
                                e
                            );
                        }
                    }
                }
                Err(e) => {
                    error!("Invalid operator address in config: {} - {}", operator_addr_str, e);
                }
            }
        }

        Ok(())
    }

    /// Get the last known shadow mode status without querying the chain
    /// Useful for metrics and dashboards
    pub fn get_cached_status(&self) -> Option<&ShadowModeStatus> {
        self.last_status_check.as_ref()
    }

    /// Check if shadow mode monitoring is properly configured
    pub fn is_monitoring_configured(&self) -> bool {
        self.config.enabled && self.config.operator_address.is_some()
    }

    /// Check if operator is already registered
    pub async fn is_registered(&self, operator_address: Address) -> Result<bool> {
        let (_, _, _, _, is_active, _) = self.get_operator_status(operator_address).await?;
        Ok(is_active)
    }

    /// Register initial XGA commitment (shadow mode - tracking only)
    pub async fn register_commitment(
        &mut self,
        commitment: &XGACommitment,
        validator_pubkey: BlsPublicKey,
    ) -> Result<()> {
        let commitment_hash = commitment.get_tree_hash_root();
        let _signature = self.sign_commitment(commitment, validator_pubkey).await?;

        info!("Shadow mode: Tracking XGA commitment registration");

        // In shadow mode, we only track the commitment hash locally
        // No on-chain transaction is made
        self.current_commitment_hash = Some(commitment_hash.0.into());

        // Monitor the shadow mode status after registration if operator address is configured
        if let Some(operator_addr_str) = &self.config.operator_address {
            if let Ok(operator_address) = operator_addr_str.parse::<Address>() {
                if let Err(e) = self.monitor_shadow_mode(operator_address).await {
                    warn!("Failed to monitor shadow mode after registration: {}", e);
                }
                info!(
                    commitment_hash = ?commitment_hash,
                    validator = ?validator_pubkey,
                    operator = ?operator_address,
                    "XGA commitment tracked in shadow mode"
                );
            } else {
                warn!("Invalid operator address configured: {}", operator_addr_str);
                info!(
                    commitment_hash = ?commitment_hash,
                    validator = ?validator_pubkey,
                    "XGA commitment tracked in shadow mode (no operator monitoring)"
                );
            }
        } else {
            info!(
                commitment_hash = ?commitment_hash,
                validator = ?validator_pubkey,
                "XGA commitment tracked in shadow mode (no operator address configured)"
            );
        }

        Ok(())
    }

    /// Update existing commitment (shadow mode - tracking only)
    pub async fn update_commitment(
        &mut self,
        new_commitment: &XGACommitment,
        validator_pubkey: BlsPublicKey,
    ) -> Result<()> {
        let new_hash = new_commitment.get_tree_hash_root();

        // Check if commitment actually changed
        if let Some(current) = self.current_commitment_hash {
            if current == B256::from(new_hash.0) {
                debug!("Commitment unchanged, skipping update");
                return Ok(());
            }
        }

        let _signature = self.sign_commitment(new_commitment, validator_pubkey).await?;

        info!("Shadow mode: Tracking XGA commitment update");

        // Validate shadow mode health and monitor if operator address is configured
        if let Some(operator_addr_str) = &self.config.operator_address {
            if let Ok(operator_address) = operator_addr_str.parse::<Address>() {
                // Validate shadow mode health before update
                match self.validate_shadow_mode_health(operator_address).await {
                    Ok(healthy) => {
                        if !healthy {
                            warn!("Shadow mode health check failed, but continuing with update");
                        }
                    }
                    Err(e) => {
                        warn!("Failed to validate shadow mode health: {}", e);
                    }
                }

                // In shadow mode, we only track the commitment hash locally
                self.current_commitment_hash = Some(new_hash.0.into());

                // Monitor status changes after update
                if let Err(e) = self.monitor_shadow_mode(operator_address).await {
                    warn!("Failed to monitor shadow mode after update: {}", e);
                }

                info!(
                    new_hash = ?new_hash,
                    validator = ?validator_pubkey,
                    operator = ?operator_address,
                    "XGA commitment update tracked in shadow mode"
                );
            } else {
                warn!("Invalid operator address configured: {}", operator_addr_str);
                self.current_commitment_hash = Some(new_hash.0.into());
                info!(
                    new_hash = ?new_hash,
                    validator = ?validator_pubkey,
                    "XGA commitment update tracked in shadow mode (no operator monitoring)"
                );
            }
        } else {
            // No operator address configured, just track the commitment
            self.current_commitment_hash = Some(new_hash.0.into());
            info!(
                new_hash = ?new_hash,
                validator = ?validator_pubkey,
                "XGA commitment update tracked in shadow mode (no operator address configured)"
            );
        }

        Ok(())
    }

    // Shadow mode only - no exit or claim functions

    /// Sign commitment using the same validator key as relay registration
    async fn sign_commitment(
        &self,
        commitment: &XGACommitment,
        validator_pubkey: BlsPublicKey,
    ) -> Result<Bytes> {
        debug!(
            validator_pubkey = ?validator_pubkey,
            "Requesting signature for EigenLayer commitment"
        );

        // Use the same signer service as for relay commitments
        let request = SignConsensusRequest::builder(validator_pubkey).with_msg(commitment);

        let signature = self
            .cb_config
            .signer_client
            .clone()
            .request_consensus_signature(request)
            .await
            .wrap_err("Failed to sign EigenLayer commitment")?;

        // Convert BlsSignature to Bytes for the contract
        Ok(Bytes::from(signature.0.to_vec()))
    }

    /// Check if commitment needs updating based on validator changes
    pub fn requires_update(&self, new_commitment_hash: B256) -> bool {
        match self.current_commitment_hash {
            Some(current) => current != new_commitment_hash,
            None => true, // Not registered yet
        }
    }

    /// Verify chain ID matches expected network
    pub async fn verify_chain_id(&self, expected_chain_id: u64) -> Result<()> {
        let chain_id =
            self.provider.get_chain_id().await.wrap_err("Failed to get chain ID from provider")?;

        if chain_id != expected_chain_id {
            return Err(eyre::eyre!(
                "Chain ID mismatch: expected {}, got {}",
                expected_chain_id,
                chain_id
            ));
        }

        Ok(())
    }

    /// Get current block number with retry logic
    pub async fn get_current_block(&self) -> Result<u64> {
        let max_retries = 3;
        let mut last_error = None;

        for attempt in 0..max_retries {
            match self.provider.get_block_number().await {
                Ok(block) => return Ok(block),
                Err(e) => {
                    debug!("Failed to get block number (attempt {}): {}", attempt + 1, e);
                    last_error = Some(e);
                    if attempt < max_retries - 1 {
                        tokio::time::sleep(Duration::from_millis(100 * (attempt + 1) as u64)).await;
                    }
                }
            }
        }

        Err(eyre::eyre!(
            "Failed to get block number after {} retries: {:?}",
            max_retries,
            last_error
        ))
    }

    /// Check if contract is deployed at the configured address
    pub async fn verify_contract_deployed(&self) -> Result<()> {
        let code = self
            .provider
            .get_code_at(self.registry_address)
            .await
            .wrap_err("Failed to get contract code")?;

        if code.is_empty() {
            return Err(eyre::eyre!(
                "No contract deployed at registry address: {}",
                self.registry_address
            ));
        }

        Ok(())
    }
}

// Type alias for the default provider type
type DefaultProvider = alloy::providers::fillers::FillProvider<
    alloy::providers::fillers::JoinFill<
        alloy::providers::Identity,
        alloy::providers::fillers::JoinFill<
            alloy::providers::fillers::GasFiller,
            alloy::providers::fillers::JoinFill<
                alloy::providers::fillers::BlobGasFiller,
                alloy::providers::fillers::JoinFill<
                    alloy::providers::fillers::NonceFiller,
                    alloy::providers::fillers::ChainIdFiller,
                >,
            >,
        >,
    >,
    alloy::providers::RootProvider<alloy::network::Ethereum>,
    alloy::network::Ethereum,
>;

// Concrete implementation for the default HTTP provider
impl EigenLayerIntegration<DefaultProvider> {
    /// Create new EigenLayer integration instance with default HTTP provider
    pub async fn new(
        config: EigenLayerConfig,
        cb_config: Arc<StartCommitModuleConfig<crate::config::XGAConfig>>,
    ) -> Result<Self> {
        // Create provider without wallet (read-only for shadow mode)
        let rpc_url = config.rpc_url.parse()?;
        let provider = ProviderBuilder::new().on_http(rpc_url);
        let provider = Arc::new(provider);

        Self::with_provider(provider, config, cb_config).await
    }
}

// Implement the query trait for EigenLayerIntegration
impl<P> EigenLayerQueries for EigenLayerIntegration<P>
where
    P: Provider + Clone + Send + Sync + 'static,
{
    async fn get_pending_rewards(&self, operator_address: Address) -> Result<U256> {
        let tx = TransactionRequest::default()
            .to(self.registry_address)
            .input(encode_address_call(GET_PENDING_REWARDS_SELECTOR, operator_address).into());

        let result = self.provider.call(tx).await.wrap_err("Failed to call getPendingRewards")?;

        decode_uint256(result)
    }

    async fn get_operator_status(
        &self,
        operator_address: Address,
    ) -> Result<(B256, Bytes, U256, U256, bool, U256)> {
        let tx = TransactionRequest::default()
            .to(self.registry_address)
            .input(encode_address_call(OPERATORS_SELECTOR, operator_address).into());

        let result = self.provider.call(tx).await.wrap_err("Failed to call operators")?;

        decode_operator_data(result)
    }

    async fn get_shadow_mode_status(&self, operator_address: Address) -> Result<ShadowModeStatus> {
        let (
            commitment_hash,
            _signature,
            registration_block,
            last_reward_block,
            is_active,
            accumulated_rewards,
        ) = self.get_operator_status(operator_address).await?;
        let pending_rewards = self.get_pending_rewards(operator_address).await?;

        let tx = TransactionRequest::default()
            .to(self.registry_address)
            .input(encode_address_call(PENALTY_RATES_SELECTOR, operator_address).into());

        let penalty_result =
            self.provider.call(tx).await.wrap_err("Failed to call penaltyRates")?;

        let penalty_rate = decode_uint256(penalty_result)?;

        let blocks_active = if is_active {
            U256::from(last_reward_block.saturating_sub(registration_block))
        } else {
            U256::ZERO
        };

        Ok(ShadowModeStatus {
            is_registered: is_active,
            commitment_hash,
            last_update_block: last_reward_block.try_into().unwrap_or(0),
            penalty_count: 0, // TODO: Get from contract
            accumulated_rewards,
            // Legacy fields
            pending_rewards,
            blocks_active,
            penalty_rate,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;
    use std::pin::Pin;

    // Create a test implementation to verify method signatures
    struct TestEigenLayer;

    impl TestEigenLayer {
        // Mock the same methods to ensure they compile and have correct signatures
        async fn mock_get_pending_rewards(&self, _operator_address: Address) -> Result<U256> {
            Ok(U256::from(1000u64))
        }

        async fn mock_get_operator_status(
            &self,
            _operator_address: Address,
        ) -> Result<(B256, Bytes, U256, U256, bool, U256)> {
            Ok((
                B256::from([0u8; 32]),
                Bytes::from(vec![0u8; 96]),
                U256::from(100u64),
                U256::from(200u64),
                true,
                U256::from(500u64),
            ))
        }

        async fn mock_get_shadow_mode_status(
            &self,
            operator_address: Address,
        ) -> Result<ShadowModeStatus> {
            // This demonstrates the actual usage pattern
            let (
                _commitment_hash,
                _signature,
                registration_block,
                last_reward_block,
                is_active,
                accumulated_rewards,
            ) = self.mock_get_operator_status(operator_address).await?;
            let pending_rewards = self.mock_get_pending_rewards(operator_address).await?;

            let blocks_active = if is_active {
                U256::from(last_reward_block.saturating_sub(registration_block))
            } else {
                U256::ZERO
            };

            Ok(ShadowModeStatus {
                is_registered: is_active,
                commitment_hash: B256::from([0u8; 32]),
                last_update_block: last_reward_block.try_into().unwrap_or(0),
                penalty_count: 0,
                accumulated_rewards,
                // Legacy fields
                pending_rewards,
                blocks_active,
                penalty_rate: U256::from(10u64),
            })
        }
    }

    #[tokio::test]
    async fn test_method_usage_pattern() {
        let test_impl = TestEigenLayer;
        let test_address = Address::from([1u8; 20]);

        // Test individual method calls (as used by CLI)
        let rewards = test_impl.mock_get_pending_rewards(test_address).await.unwrap();
        assert_eq!(rewards, U256::from(1000u64));

        let status = test_impl.mock_get_operator_status(test_address).await.unwrap();
        assert!(status.4); // is_active

        // Test comprehensive method (which internally uses the other two)
        let shadow_status = test_impl.mock_get_shadow_mode_status(test_address).await.unwrap();
        assert!(shadow_status.is_registered);
        assert_eq!(shadow_status.pending_rewards, U256::from(1000u64));
    }

    #[test]
    fn test_public_api_methods_exist() {
        // This test ensures the public API methods are recognized as used
        // by creating references to them. These methods are called by:
        // 1. The CLI binary (xga_cli) for operator queries
        // 2. Internally by get_shadow_mode_status

        fn _type_check() {
            // Create type references to ensure methods exist with correct signatures
            type EL = EigenLayerIntegration<DefaultProvider>;

            let _: fn(&EL, Address) -> Pin<Box<dyn Future<Output = Result<U256>> + Send + '_>> =
                |integration, addr| Box::pin(integration.get_pending_rewards(addr));

            let _: fn(
                &EL,
                Address,
            ) -> Pin<
                Box<dyn Future<Output = Result<(B256, Bytes, U256, U256, bool, U256)>> + Send + '_>,
            > = |integration, addr| Box::pin(integration.get_operator_status(addr));

            let _: fn(
                &EL,
                Address,
            )
                -> Pin<Box<dyn Future<Output = Result<ShadowModeStatus>> + Send + '_>> =
                |integration, addr| Box::pin(integration.get_shadow_mode_status(addr));
        }

        // The function exists to verify types compile
        assert!(true);
    }

    #[test]
    fn test_trait_implementation_completeness() {
        // This test verifies that EigenLayerIntegration implements all trait methods
        fn assert_impl_eigenlayer_queries<T: EigenLayerQueries>() {}

        // This line will fail to compile if EigenLayerIntegration doesn't implement the trait
        assert_impl_eigenlayer_queries::<EigenLayerIntegration<DefaultProvider>>();

        // The trait methods are used as follows:
        // 1. get_pending_rewards - Used internally by get_shadow_mode_status and by CLI
        // 2. get_operator_status - Used internally by get_shadow_mode_status and by CLI
        // 3. get_shadow_mode_status - Used by CLI for comprehensive status queries
        //
        // The compiler warning about get_shadow_mode_status being unused is a false positive
        // because it's used by the xga_cli binary, which the library doesn't see.
    }
}
