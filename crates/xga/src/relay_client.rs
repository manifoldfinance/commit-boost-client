use crate::relay_communication::RelayQueryService;
use crate::state::ReservedGasState;
use crate::state_traits::{ConfigFetchManager, GasReservationManager};
use std::time::Duration;
use tracing::{debug, info, warn};

impl ReservedGasState {
    /// Fetch gas configurations from all relays
    pub async fn fetch_relay_configs(&self) {
        if !self.should_fetch_configs() {
            return;
        }

        info!("Fetching gas configurations from relays");

        let relays = &self.pbs_config.relays;
        let endpoint = &self.config.relay_config_endpoint;
        let query_service = RelayQueryService::production();

        // Fetch from all relays in parallel
        let futures: Vec<_> = relays
            .iter()
            .map(|relay| {
                let query_service = &query_service;
                let endpoint = endpoint.clone();
                async move {
                    let result = query_service.fetch_gas_config(relay, &endpoint).await;
                    (relay, result)
                }
            })
            .collect();

        let results = futures::future::join_all(futures).await;

        // Track if we successfully fetched from at least one relay
        let mut any_success = false;

        // Process results
        for (relay, result) in results {
            match result {
                Ok(Some(config)) => {
                    // Validate relay response before using it
                    if crate::validation::is_valid_relay_response(&config, &relay.id) {
                        self.update_reservation((*relay.id).clone(), config.clone());
                        any_success = true;

                        // Log configuration update event
                        let event = crate::types::ConfigUpdateEvent {
                            relay_id: (*relay.id).clone(),
                            old_reservation: self
                                .reservations
                                .get(&**relay.id)
                                .map(|r| r.reserved_gas),
                            new_reservation: config.reserved_gas_limit,
                            timestamp: std::time::Instant::now(),
                        };
                        debug!("Config update event: {:?}", event);
                    } else {
                        warn!(
                            module = crate::RESERVED_GAS_MODULE_NAME,
                            relay_id = %relay.id,
                            "Invalid gas config from relay, ignoring"
                        );
                    }
                }
                Ok(None) => {
                    debug!(relay_id = %relay.id, "Relay does not support gas config endpoint");
                }
                Err(e) => {
                    warn!(
                        module = crate::RESERVED_GAS_MODULE_NAME,
                        relay_id = %relay.id,
                        error = %e,
                        "Failed to fetch gas config from relay"
                    );
                }
            }
        }

        // Only update the fetch timestamp if we successfully fetched from at least one relay
        if any_success {
            self.mark_fetch_completed();
        }
    }
}

/// Background task to periodically fetch relay configurations
pub async fn config_fetcher_task(state: ReservedGasState) -> ! {
    let mut interval =
        tokio::time::interval(Duration::from_secs(state.config.update_interval_secs));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        interval.tick().await;
        state.fetch_relay_configs().await;
    }
}
