use crate::config::RelayGasConfig;
use crate::error::Result;
use crate::state::GasReservation;
use std::time::Instant;

/// Trait for managing gas reservations
pub trait GasReservationManager: Send + Sync {
    /// Get the current gas reservation for a relay
    fn get_reservation(&self, relay_id: &str) -> u64;

    /// Update gas reservation for a relay
    fn update_reservation(&self, relay_id: String, config: RelayGasConfig);

    /// Apply gas reservation to a block gas limit
    ///
    /// Returns a `GasReservationOutcome` containing the new gas limit and
    /// the amount that was reserved. The new gas limit will be the original
    /// limit minus the reserved amount, subject to validation constraints.
    ///
    /// # Arguments
    ///
    /// * `relay_id` - The identifier of the relay
    /// * `original_gas_limit` - The original gas limit before reservation
    ///
    /// # Returns
    ///
    /// * `Ok(GasReservationOutcome)` - The outcome with new limit and reserved amount
    /// * `Err` - If the gas limit would be too low after reservation
    fn apply_reservation(
        &self,
        relay_id: &str,
        original_gas_limit: u64,
    ) -> Result<crate::types::GasReservationOutcome>;

    /// Get all current reservations for monitoring
    fn get_all_reservations(&self) -> Vec<(String, GasReservation)>;
}

/// Trait for managing configuration fetching
pub trait ConfigFetchManager: Send + Sync {
    /// Check if we should fetch new configs from relays
    fn should_fetch_configs(&self) -> bool;

    /// Get the last fetch time
    fn get_last_fetch_time(&self) -> Instant;
}
