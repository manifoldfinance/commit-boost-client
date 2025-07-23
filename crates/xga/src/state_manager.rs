use crate::config::{RelayGasConfig, ReservedGasConfig};
use crate::error::Result;
use crate::relay_communication::RelayQueryService;
use crate::types::{GasReservation, GasReservationOutcome, ConfigUpdateEvent};
use cb_common::config::PbsModuleConfig;
use cb_common::pbs::RelayClient;
use dashmap::DashMap;
use futures::future::join_all;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Represents the different states of the gas reservation system
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum GasReservationState {
    /// Initial state with no configuration loaded
    Uninitialized,
    
    /// State with only local configuration loaded
    LocalOnly {
        config: Arc<ReservedGasConfig>,
        configured_at: Instant,
    },
    
    /// State with configuration synced from relays
    Synced {
        config: Arc<ReservedGasConfig>,
        synced_at: Instant,
        next_sync: Instant,
    },
    
    /// State with stale configuration
    Stale {
        config: Arc<ReservedGasConfig>,
        last_sync: Instant,
    },
}

/// Gas reservation manager that encapsulates all state logic
pub struct GasReservationManager {
    /// Current state of the manager
    state: Arc<RwLock<GasReservationState>>,
    
    /// Per-relay gas reservations (relay_id -> reservation)
    reservations: Arc<DashMap<String, GasReservation>>,
    
    /// PBS module config for accessing relays (set after initialization)
    pbs_config: Arc<RwLock<Option<PbsModuleConfig>>>,
}

impl GasReservationManager {
    /// Create a new uninitialized gas reservation manager
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(GasReservationState::Uninitialized)),
            reservations: Arc::new(DashMap::new()),
            pbs_config: Arc::new(RwLock::new(None)),
        }
    }
    
    /// Configure the gas reservation system with local settings
    pub async fn configure(
        &self,
        config: ReservedGasConfig,
        pbs_config: PbsModuleConfig,
    ) -> Result<()> {
        let mut state = self.state.write().await;
        
        // Ensure we're in the uninitialized state
        match &*state {
            GasReservationState::Uninitialized => {},
            _ => return Err(crate::error::ReservedGasError::ConfigError(
                "Cannot configure: already initialized".to_string()
            )),
        }
        
        // Initialize with configured overrides
        for (relay_id, reserved_gas) in &config.relay_overrides {
            self.reservations.insert(
                relay_id.clone(),
                GasReservation {
                    reserved_gas: *reserved_gas,
                    last_updated: Instant::now(),
                    relay_id: relay_id.clone(),
                    min_gas_limit: None,
                },
            );
            self.init_metrics_for_relay(relay_id, *reserved_gas);
        }
        
        // Update PBS config
        *self.pbs_config.write().await = Some(pbs_config);
        
        // Transition to LocalOnly state
        *state = GasReservationState::LocalOnly {
            config: Arc::new(config),
            configured_at: Instant::now(),
        };
        
        Ok(())
    }
    
    /// Sync configuration with relays if enabled
    pub async fn sync_if_enabled(
        &self,
        relays: &[RelayClient],
        query_service: &RelayQueryService,
    ) -> Result<()> {
        let state_guard = self.state.read().await;
        
        let config = match &*state_guard {
            GasReservationState::LocalOnly { config, .. } => config.clone(),
            GasReservationState::Synced { config, .. } => config.clone(),
            GasReservationState::Stale { config, .. } => config.clone(),
            GasReservationState::Uninitialized => {
                return Err(crate::error::ReservedGasError::ConfigError(
                    "Cannot sync: not configured".to_string()
                ));
            }
        };
        
        if !config.fetch_from_relays {
            info!("Relay fetching disabled, using local configuration only");
            return Ok(());
        }
        
        drop(state_guard); // Release read lock before acquiring write lock
        self.sync(relays, query_service).await
    }
    
    /// Force sync with relays
    pub async fn sync(
        &self,
        relays: &[RelayClient],
        query_service: &RelayQueryService,
    ) -> Result<()> {
        let state_guard = self.state.read().await;
        
        let config = match &*state_guard {
            GasReservationState::LocalOnly { config, .. } => config.clone(),
            GasReservationState::Synced { config, .. } => config.clone(),
            GasReservationState::Stale { config, .. } => config.clone(),
            GasReservationState::Uninitialized => {
                return Err(crate::error::ReservedGasError::ConfigError(
                    "Cannot sync: not configured".to_string()
                ));
            }
        };
        
        drop(state_guard); // Release read lock
        
        info!("Syncing gas configuration with {} relays", relays.len());
        
        let futures: Vec<_> = relays
            .iter()
            .map(|relay| {
                let relay_id = relay.id.clone();
                async move {
                    match query_service.fetch_gas_config(relay, "/eth/v1/reserve/gas").await {
                        Ok(Some(gas_config)) => {
                            info!(relay_id = %relay_id, "Fetched gas config from relay");
                            Some((relay_id, gas_config))
                        }
                        Ok(None) => {
                            info!(relay_id = %relay_id, "Relay does not support gas config endpoint");
                            None
                        }
                        Err(e) => {
                            warn!(relay_id = %relay_id, error = %e, "Failed to fetch gas config");
                            None
                        }
                    }
                }
            })
            .collect();
        
        let results = join_all(futures).await;
        
        // Update reservations with fetched configs
        for result in results.into_iter().flatten() {
            let (relay_id, gas_config) = result;
            self.update_reservation_internal((*relay_id).clone(), gas_config);
        }
        
        let now = Instant::now();
        let next_sync = now + Duration::from_secs(config.update_interval_secs);
        
        // Update state to Synced
        let mut state = self.state.write().await;
        *state = GasReservationState::Synced {
            config,
            synced_at: now,
            next_sync,
        };
        
        Ok(())
    }
    
    /// Check if the configuration is still fresh
    pub async fn check_freshness(&self) -> Result<bool> {
        let mut state = self.state.write().await;
        
        match &*state {
            GasReservationState::Synced { config, synced_at, next_sync } => {
                let now = Instant::now();
                
                if now >= *next_sync {
                    // Transition to stale
                    *state = GasReservationState::Stale {
                        config: config.clone(),
                        last_sync: *synced_at,
                    };
                    Ok(false)
                } else {
                    Ok(true)
                }
            }
            GasReservationState::Stale { .. } => Ok(false),
            GasReservationState::LocalOnly { .. } => Ok(true), // Always considered fresh when not syncing
            GasReservationState::Uninitialized => {
                Err(crate::error::ReservedGasError::ConfigError(
                    "Cannot check freshness: not configured".to_string()
                ))
            }
        }
    }
    
    /// Refresh stale configuration
    pub async fn refresh(
        &self,
        relays: &[RelayClient],
        query_service: &RelayQueryService,
    ) -> Result<()> {
        let state_guard = self.state.read().await;
        
        match &*state_guard {
            GasReservationState::Stale { .. } => {
                drop(state_guard); // Release read lock
                warn!("Refreshing stale gas configuration");
                self.sync(relays, query_service).await
            }
            _ => Ok(()),
        }
    }
    
    /// Get the gas reservation amount for a specific relay
    pub async fn get_reservation(&self, relay_id: &str) -> u64 {
        let state = self.state.read().await;
        
        let config = match &*state {
            GasReservationState::LocalOnly { config, .. } => config,
            GasReservationState::Synced { config, .. } => config,
            GasReservationState::Stale { config, .. } => config,
            GasReservationState::Uninitialized => {
                return 0; // Return 0 for uninitialized state
            }
        };
        
        if let Some(reservation) = self.reservations.get(relay_id) {
            reservation.reserved_gas
        } else {
            config.default_reserved_gas
        }
    }
    
    /// Apply gas reservation to a gas limit
    pub async fn apply_reservation(
        &self,
        relay_id: &str,
        original_gas_limit: u64,
    ) -> Result<GasReservationOutcome> {
        let state = self.state.read().await;
        
        let config = match &*state {
            GasReservationState::LocalOnly { config, .. } => config,
            GasReservationState::Synced { config, .. } => config,
            GasReservationState::Stale { config, .. } => config,
            GasReservationState::Uninitialized => {
                return Err(crate::error::ReservedGasError::ConfigError(
                    "Cannot apply reservation: not configured".to_string()
                ));
            }
        };
        
        let reserved_amount = if let Some(reservation) = self.reservations.get(relay_id) {
            reservation.reserved_gas
        } else {
            config.default_reserved_gas
        };
        
        // Validate the gas limit after reservation
        let new_gas_limit = config.validate_gas_limit(original_gas_limit, reserved_amount)?;
        
        // Record metrics
        let metrics = crate::metrics_wrapper::create_metrics_recorder(false);
        metrics.record_gas_reservation_applied(relay_id, reserved_amount);
        
        Ok(GasReservationOutcome {
            new_gas_limit,
            reserved_amount,
            relay_id: relay_id.to_string(),
        })
    }
    
    /// Update relay configuration manually
    pub async fn update_relay_configuration(
        &self,
        relay_id: String,
        gas_config: RelayGasConfig,
    ) -> Result<ConfigUpdateEvent> {
        let state = self.state.read().await;
        
        // Ensure we're configured
        let current_config = match &*state {
            GasReservationState::LocalOnly { config, .. } => config,
            GasReservationState::Synced { config, .. } => config,
            GasReservationState::Stale { config, .. } => config,
            GasReservationState::Uninitialized => {
                return Err(crate::error::ReservedGasError::ConfigError(
                    "Cannot update relay configuration: not configured".to_string()
                ));
            }
        };
        
        // Get old reservation before update
        let old_reservation = self.reservations
            .get(&relay_id)
            .map(|r| r.reserved_gas)
            .or(Some(current_config.default_reserved_gas));
        
        // Validate the new config
        if gas_config.reserved_gas_limit == 0 || gas_config.reserved_gas_limit > crate::validation::MAX_REASONABLE_GAS {
            // Return event showing no change
            return Ok(ConfigUpdateEvent {
                relay_id,
                old_reservation,
                new_reservation: old_reservation.unwrap_or(0),
                timestamp: Instant::now(),
            });
        }
        
        let new_reservation = gas_config.reserved_gas_limit;
        self.update_reservation_internal(relay_id.clone(), gas_config);
        
        Ok(ConfigUpdateEvent {
            relay_id,
            old_reservation,
            new_reservation,
            timestamp: Instant::now(),
        })
    }
    
    /// Get all current reservations
    pub fn get_all_reservations(&self) -> Vec<(String, GasReservation)> {
        self.reservations
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect()
    }
    
    /// Get the current state (for debugging/monitoring)
    pub async fn get_state(&self) -> GasReservationState {
        self.state.read().await.clone()
    }
    
    /// Check if the manager is configured
    pub async fn is_configured(&self) -> bool {
        !matches!(*self.state.read().await, GasReservationState::Uninitialized)
    }
    
    // Helper methods
    
    fn init_metrics_for_relay(&self, relay_id: &str, reserved_gas: u64) {
        let metrics = crate::metrics_wrapper::create_metrics_recorder(false);
        metrics.record_gas_reservation_amount(relay_id, reserved_gas);
    }
    
    fn update_reservation_internal(&self, relay_id: String, config: RelayGasConfig) {
        let reservation = GasReservation {
            reserved_gas: config.reserved_gas_limit,
            last_updated: Instant::now(),
            relay_id: relay_id.clone(),
            min_gas_limit: config.min_gas_limit,
        };
        
        debug!(
            relay_id = %relay_id,
            reserved_gas = config.reserved_gas_limit,
            "Updated gas reservation for relay"
        );
        
        self.init_metrics_for_relay(&relay_id, config.reserved_gas_limit);
        let metrics = crate::metrics_wrapper::create_metrics_recorder(false);
        metrics.record_gas_config_update(&relay_id);
        
        self.reservations.insert(relay_id, reservation);
    }
}

// Compatibility types for maintaining the same public API

/// Type alias for backward compatibility
/// The S parameter is consumed but not used
pub struct ReservedGasState<S> {
    _phantom: std::marker::PhantomData<S>,
}

impl<S> ReservedGasState<S> {
    /// This type is deprecated - use GasReservationManager directly
    #[allow(unused)]
    fn _marker() {}
}

/// Uninitialized state marker for backward compatibility
pub struct Uninitialized;

/// LocalOnly state marker for backward compatibility
#[allow(dead_code)]
pub struct LocalOnly {
    pub configured_at: Instant,
}

/// Synced state marker for backward compatibility
#[allow(dead_code)]
pub struct Synced {
    pub synced_at: Instant,
    pub next_sync: Instant,
}

/// Stale state marker for backward compatibility
#[allow(dead_code)]
pub struct Stale {
    pub last_sync: Instant,
}

// Provide convenience constructors for backward compatibility

impl ReservedGasState<Uninitialized> {
    /// Create a new uninitialized gas reservation state
    pub fn new() -> GasReservationManager {
        GasReservationManager::new()
    }
    
    /// Create the phantom type (not used)
    #[allow(dead_code)]
    fn phantom() -> Self {
        Self { _phantom: std::marker::PhantomData }
    }
}

impl Default for GasReservationManager {
    fn default() -> Self {
        Self::new()
    }
}

// Wrapper type to implement BuilderApiState
#[derive(Clone)]
pub struct GasReservationManagerWrapper(pub Arc<GasReservationManager>);

// Implement BuilderApiState trait for compatibility with PBS
impl cb_pbs::BuilderApiState for GasReservationManagerWrapper {}

// Provide convenient access methods
impl std::ops::Deref for GasReservationManagerWrapper {
    type Target = GasReservationManager;
    
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ReservedGasConfig;
    use cb_common::config::{PbsModuleConfig, PbsConfig};
    
    #[tokio::test]
    async fn test_state_transitions() {
        let manager = GasReservationManager::new();
        
        // Initially uninitialized
        assert!(matches!(
            manager.get_state().await,
            GasReservationState::Uninitialized
        ));
        
        // Configure with local settings
        let config = ReservedGasConfig {
            default_reserved_gas: 1_000_000,
            relay_overrides: Default::default(),
            update_interval_secs: 60,
            min_block_gas_limit: 10_000_000,
            relay_config_endpoint: "/relay/v1/gas_config".to_string(),
            fetch_from_relays: false,
            relay_reserve_endpoint: "/xga/v2/relay/reserve".to_string(),
        };
        
        use std::net::{Ipv4Addr, SocketAddr};
        use alloy::primitives::U256;
        use cb_common::types::Chain;
        
        let pbs_cfg = PbsConfig {
            host: Ipv4Addr::new(127, 0, 0, 1),
            port: 18550,
            relay_check: true,
            wait_all_registrations: true,
            timeout_get_header_ms: 1000,
            timeout_get_payload_ms: 1000,
            timeout_register_validator_ms: 1000,
            register_validator_retry_limit: 3,
            skip_sigverify: false,
            min_bid_wei: U256::ZERO,
            late_in_slot_time_ms: 4000,
            extra_validation_enabled: false,
            rpc_url: None,
            http_timeout_seconds: 30,
        };
        
        let pbs_config = PbsModuleConfig {
            chain: Chain::Mainnet,
            endpoint: SocketAddr::from(([127, 0, 0, 1], 18550)),
            pbs_config: Arc::new(pbs_cfg),
            relays: vec![],
            all_relays: vec![],
            signer_client: None,
            event_publisher: None,
            muxes: None,
        };
        
        manager.configure(config, pbs_config).await.unwrap();
        
        // Should now be in LocalOnly state
        assert!(matches!(
            manager.get_state().await,
            GasReservationState::LocalOnly { .. }
        ));
    }
    
    #[tokio::test]
    async fn test_gas_reservation_application() {
        let manager = GasReservationManager::new();
        
        let config = ReservedGasConfig {
            default_reserved_gas: 2_000_000,
            relay_overrides: Default::default(),
            update_interval_secs: 60,
            min_block_gas_limit: 10_000_000,
            relay_config_endpoint: "/relay/v1/gas_config".to_string(),
            fetch_from_relays: false,
            relay_reserve_endpoint: "/xga/v2/relay/reserve".to_string(),
        };
        
        use std::net::{Ipv4Addr, SocketAddr};
        use alloy::primitives::U256;
        use cb_common::types::Chain;
        
        let pbs_cfg = PbsConfig {
            host: Ipv4Addr::new(127, 0, 0, 1),
            port: 18550,
            relay_check: true,
            wait_all_registrations: true,
            timeout_get_header_ms: 1000,
            timeout_get_payload_ms: 1000,
            timeout_register_validator_ms: 1000,
            register_validator_retry_limit: 3,
            skip_sigverify: false,
            min_bid_wei: U256::ZERO,
            late_in_slot_time_ms: 4000,
            extra_validation_enabled: false,
            rpc_url: None,
            http_timeout_seconds: 30,
        };
        
        let pbs_config = PbsModuleConfig {
            chain: Chain::Mainnet,
            endpoint: SocketAddr::from(([127, 0, 0, 1], 18550)),
            pbs_config: Arc::new(pbs_cfg),
            relays: vec![],
            all_relays: vec![],
            signer_client: None,
            event_publisher: None,
            muxes: None,
        };
        
        manager.configure(config, pbs_config).await.unwrap();
        
        // Apply reservation
        let outcome = manager.apply_reservation("test-relay", 30_000_000).await.unwrap();
        assert_eq!(outcome.reserved_amount, 2_000_000);
        assert_eq!(outcome.new_gas_limit, 28_000_000);
    }
}