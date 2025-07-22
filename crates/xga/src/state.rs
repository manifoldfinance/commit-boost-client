use crate::config::{RelayGasConfig, ReservedGasConfig};
use crate::error::Result;
use crate::relay_communication::RelayQueryService;
use crate::service::ReservedGasService;
use crate::state_traits::{ConfigFetchManager, GasReservationManager};
use cb_common::config::PbsModuleConfig;
use cb_pbs::BuilderApiState;
use dashmap::DashMap;
use parking_lot::Mutex;
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, warn};

#[derive(Debug, Clone)]
pub struct GasReservation {
    /// Reserved gas amount for this relay
    pub reserved_gas: u64,

    /// When this reservation was last updated
    pub last_updated: Instant,

    /// The relay ID this reservation is for
    pub relay_id: String,

    /// Optional min gas limit from relay
    pub min_gas_limit: Option<u64>,
}

#[derive(Clone)]
pub struct ReservedGasState {
    /// Module configuration
    pub config: Arc<ReservedGasConfig>,

    /// Per-relay gas reservations (relay_id -> reservation)
    pub reservations: Arc<DashMap<String, GasReservation>>,

    /// Last time we fetched configs from relays
    pub last_fetch: Arc<Mutex<Instant>>,

    /// PBS module config for accessing relays
    pub pbs_config: PbsModuleConfig,
}

impl BuilderApiState for ReservedGasState {}

impl ReservedGasState {
    pub fn new(config: ReservedGasConfig, pbs_config: PbsModuleConfig) -> Self {
        let reservations = Arc::new(DashMap::new());

        // Initialize with configured overrides
        for (relay_id, reserved_gas) in &config.relay_overrides {
            reservations.insert(
                relay_id.clone(),
                GasReservation {
                    reserved_gas: *reserved_gas,
                    last_updated: Instant::now(),
                    relay_id: relay_id.clone(),
                    min_gas_limit: None,
                },
            );

            // Initialize metrics
            let metrics = crate::metrics_wrapper::create_metrics_recorder(false);
            metrics.record_gas_reservation_amount(relay_id, *reserved_gas);
        }

        Self {
            config: Arc::new(config),
            reservations,
            last_fetch: Arc::new(Mutex::new(Instant::now())),
            pbs_config,
        }
    }

    /// Create a ReservedGasService using the stored dependencies
    pub fn create_service(&self) -> ReservedGasService {
        ReservedGasService::with_dependencies(
            Arc::new(self.clone()) as Arc<dyn GasReservationManager>,
            Arc::new(self.clone()) as Arc<dyn ConfigFetchManager>,
            crate::metrics_wrapper::create_metrics_recorder(false),
            RelayQueryService::production(),
        )
    }

    /// Update gas reservation for a relay (internal helper)
    fn update_reservation_internal(&self, relay_id: String, config: RelayGasConfig) {
        // Validate min_gas_limit if provided
        if let Some(min_gas) = config.min_gas_limit {
            if min_gas < 1_000_000 {
                warn!(
                    relay_id = %relay_id,
                    min_gas_limit = min_gas,
                    "Relay specified very low minimum gas limit, ignoring"
                );
                return;
            }
        }

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

        // Update metrics
        let metrics = crate::metrics_wrapper::create_metrics_recorder(false);
        metrics.record_gas_reservation_amount(&relay_id, config.reserved_gas_limit);
        metrics.record_gas_config_update(&relay_id);

        self.reservations.insert(relay_id, reservation);
    }

    /// Mark that config fetch has completed successfully
    pub fn mark_fetch_completed(&self) {
        let mut last_fetch = self.last_fetch.lock();
        *last_fetch = Instant::now();
    }

    /// Apply gas reservation to a block gas limit (internal helper)
    ///
    /// Returns a `GasReservationOutcome` containing the new gas limit and reservation details
    fn apply_reservation_internal(
        &self,
        relay_id: &str,
        original_gas_limit: u64,
    ) -> Result<crate::types::GasReservationOutcome> {
        let reserved = <Self as GasReservationManager>::get_reservation(self, relay_id);

        // Check against relay-specific min if available
        if let Some(reservation) = self.reservations.get(relay_id) {
            if let Some(min_gas) = reservation.min_gas_limit {
                let new_limit = original_gas_limit.saturating_sub(reserved);
                if new_limit < min_gas {
                    warn!(
                        relay_id = %relay_id,
                        original = original_gas_limit,
                        reserved = reserved,
                        min_required = min_gas,
                        "Gas limit after reservation would be below relay minimum"
                    );
                }
            }
        }

        // Always check against our configured minimum
        let new_limit = self
            .config
            .validate_gas_limit(original_gas_limit, reserved)
            .map_err(|e| crate::error::ReservedGasError::ConfigError(e.to_string()))?;

        // Create and return the outcome
        let outcome = crate::types::GasReservationOutcome {
            relay_id: relay_id.to_string(),
            new_gas_limit: new_limit,
            reserved_amount: reserved,
        };

        debug!("Gas reservation outcome: {:?}", outcome);

        Ok(outcome)
    }

    /// Check config staleness using ConfigFetchManager trait
    pub fn check_config_staleness(&self) -> Option<std::time::Duration> {
        use crate::state_traits::ConfigFetchManager;

        let last_fetch = self.get_last_fetch_time();
        let elapsed = last_fetch.elapsed();

        if elapsed.as_secs() > self.config.update_interval_secs * 2 {
            warn!("Config is stale by {} seconds", elapsed.as_secs());
            Some(elapsed)
        } else {
            None
        }
    }
}

impl GasReservationManager for ReservedGasState {
    fn get_reservation(&self, relay_id: &str) -> u64 {
        self.reservations
            .get(relay_id)
            .map(|r| r.reserved_gas)
            .unwrap_or_else(|| self.config.get_reserved_gas(relay_id))
    }

    fn update_reservation(&self, relay_id: String, config: RelayGasConfig) {
        self.update_reservation_internal(relay_id, config)
    }

    fn apply_reservation(
        &self,
        relay_id: &str,
        original_gas_limit: u64,
    ) -> Result<crate::types::GasReservationOutcome> {
        self.apply_reservation_internal(relay_id, original_gas_limit)
    }

    fn get_all_reservations(&self) -> Vec<(String, GasReservation)> {
        self.reservations.iter().map(|entry| (entry.key().clone(), entry.value().clone())).collect()
    }
}

impl ConfigFetchManager for ReservedGasState {
    fn should_fetch_configs(&self) -> bool {
        if !self.config.fetch_from_relays {
            return false;
        }

        let last_fetch = self.last_fetch.lock();
        let elapsed = last_fetch.elapsed().as_secs();

        elapsed >= self.config.update_interval_secs
    }

    fn get_last_fetch_time(&self) -> Instant {
        *self.last_fetch.lock()
    }
}
