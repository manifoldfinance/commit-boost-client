use crate::error::Result;
use crate::metrics_wrapper::MetricsRecorder;
use crate::relay_communication::RelayQueryService;
use crate::state_traits::{ConfigFetchManager, GasReservationManager};
use crate::types::{AdjustedBid, RelayId, RelayQueryResult};
use cb_common::pbs::{GetHeaderParams, GetHeaderResponse, RelayClient};
use std::sync::Arc;
use tracing::{debug, info, warn};

/// Service for handling reserved gas operations
pub struct ReservedGasService {
    state: Arc<dyn GasReservationManager>,
    config_manager: Arc<dyn ConfigFetchManager>,
    metrics: Arc<dyn MetricsRecorder>,
    query_service: RelayQueryService,
}

impl ReservedGasService {
    /// Create a new service with custom implementations (for testing)
    pub fn with_dependencies(
        state: Arc<dyn GasReservationManager>,
        config_manager: Arc<dyn ConfigFetchManager>,
        metrics: Arc<dyn MetricsRecorder>,
        query_service: RelayQueryService,
    ) -> Self {
        Self { state, config_manager, metrics, query_service }
    }

    /// Query relays and get the best bid with gas reservations applied
    pub async fn get_best_bid(
        &self,
        relays: &[RelayClient],
        params: GetHeaderParams,
    ) -> Result<Option<GetHeaderResponse>> {
        // Log current reservations using trait method
        let reservations = self.state.get_all_reservations();
        debug!("Current gas reservations: {} relays configured", reservations.len());

        // Query all relays
        let relay_results = self.query_all_relays(relays, params.clone()).await;

        // Process results and apply gas reservations
        let adjusted_bids = self.process_relay_results(relay_results);

        // Select the best bid
        self.select_best_bid(adjusted_bids, params.slot)
    }

    /// Query all relays in parallel
    async fn query_all_relays(
        &self,
        relays: &[RelayClient],
        params: GetHeaderParams,
    ) -> Vec<(RelayId, RelayQueryResult)> {
        let futures: Vec<_> = relays
            .iter()
            .map(|relay| {
                let relay_id: RelayId = Arc::new((*relay.id).clone());
                let params = params.clone();
                let query_service = &self.query_service;
                async move {
                    let result = match query_service.query_header(relay, params).await {
                        Ok(Some(response)) => RelayQueryResult::Bid(AdjustedBid {
                            response,
                            original_gas_limit: 0, // Will be filled in processing
                            reserved_gas: 0,
                            relay_id: relay_id.clone(),
                        }),
                        Ok(None) => RelayQueryResult::NoBid,
                        Err(e) => RelayQueryResult::Error(e.to_string()),
                    };
                    (relay_id, result)
                }
            })
            .collect();

        futures::future::join_all(futures).await
    }

    /// Process relay results and apply gas reservations
    fn process_relay_results(&self, results: Vec<(RelayId, RelayQueryResult)>) -> Vec<AdjustedBid> {
        results
            .into_iter()
            .filter_map(|(relay_id, result)| match result {
                RelayQueryResult::Bid(mut bid) => {
                    bid.original_gas_limit = bid.response.gas_limit();
                    match self.apply_gas_reservation(&mut bid, &relay_id) {
                        Ok(_) => Some(bid),
                        Err(e) => {
                            warn!(
                                module = crate::RESERVED_GAS_MODULE_NAME,
                                relay_id = %relay_id,
                                error = %e,
                                "Failed to apply gas reservation"
                            );
                            None
                        }
                    }
                }
                RelayQueryResult::NoBid => None,
                RelayQueryResult::Error(e) => {
                    warn!(relay_id = %relay_id, error = %e, "Relay query failed");
                    None
                }
            })
            .collect()
    }

    /// Apply gas reservation to a bid
    fn apply_gas_reservation(&self, bid: &mut AdjustedBid, relay_id: &str) -> Result<()> {
        // Check if config may be stale
        if self.config_manager.should_fetch_configs() {
            warn!("Config may be stale, consider updating");
        }

        let original_gas_limit = bid.original_gas_limit;
        let outcome = self.state.apply_reservation(relay_id, original_gas_limit)?;

        // Additional validation using MIN_GAS_AFTER_RESERVATION
        use crate::validation::MIN_GAS_AFTER_RESERVATION;
        if outcome.new_gas_limit < MIN_GAS_AFTER_RESERVATION {
            return Err(crate::error::ReservedGasError::GasLimitTooLow {
                actual: outcome.new_gas_limit,
                minimum: MIN_GAS_AFTER_RESERVATION,
                reserved: outcome.reserved_amount,
            });
        }

        // Update the response with new gas limit
        match &mut bid.response {
            cb_common::pbs::VersionedResponse::Electra(ref mut data) => {
                data.message.header.gas_limit = outcome.new_gas_limit;
            }
        }

        bid.reserved_gas = outcome.reserved_amount;

        // Record metrics using outcome.relay_id to ensure the field is used
        self.metrics.record_gas_reservation_applied(&outcome.relay_id, bid.reserved_gas);

        info!(
            relay_id = %outcome.relay_id,
            original_gas_limit,
            new_gas_limit = outcome.new_gas_limit,
            reserved = bid.reserved_gas,
            "Applied gas reservation to header for relay {}", outcome.relay_id
        );

        Ok(())
    }

    /// Select the best bid based on value
    fn select_best_bid(
        &self,
        bids: Vec<AdjustedBid>,
        slot: u64,
    ) -> Result<Option<GetHeaderResponse>> {
        let best_bid = bids.into_iter().max_by_key(|bid| bid.response.value());

        match best_bid {
            Some(bid) => {
                info!(
                    slot,
                    relay_id = %bid.relay_id,
                    value = %bid.response.value(),
                    reserved_gas = bid.reserved_gas,
                    "Selected best bid"
                );
                Ok(Some(bid.response))
            }
            None => Ok(None),
        }
    }

    /// Update relay configuration dynamically
    ///
    /// This method allows updating a relay's gas reservation configuration
    /// at runtime, useful for responding to relay configuration changes.
    ///
    /// # Arguments
    ///
    /// * `relay_id` - The identifier of the relay to update
    /// * `config` - The new gas configuration for the relay
    ///
    /// # Returns
    ///
    /// A `ConfigUpdateEvent` containing details of the configuration change
    ///
    /// # Example
    ///
    /// ```ignore
    /// let event = service.update_relay_configuration(
    ///     "flashbots".to_string(),
    ///     RelayGasConfig {
    ///         reserved_gas_limit: 2_000_000,
    ///         min_gas_limit: Some(15_000_000),
    ///         update_interval: None,
    ///     }
    /// );
    /// ```
    pub fn update_relay_configuration(
        &self,
        relay_id: String,
        config: crate::config::RelayGasConfig,
    ) -> crate::types::ConfigUpdateEvent {
        // Use trait method to get current reservation before update
        let old_reservation = Some(self.state.get_reservation(&relay_id));

        // Use trait method to update the reservation
        self.state.update_reservation(relay_id.clone(), config.clone());

        // Record the update in metrics
        self.metrics.record_gas_config_update(&relay_id);

        // Return the configuration update event
        crate::types::ConfigUpdateEvent {
            relay_id,
            old_reservation,
            new_reservation: config.reserved_gas_limit,
            timestamp: std::time::Instant::now(),
        }
    }
}
