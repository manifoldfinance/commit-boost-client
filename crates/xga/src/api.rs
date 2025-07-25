use crate::error::{ReservedGasError, Result};
use crate::relay_communication::RelayQueryService;
use crate::state_manager::{GasReservationManager, GasReservationManagerWrapper};
use crate::types::{AdjustedBid, RelayQueryResult};
use alloy::rpc::types::beacon::relay::ValidatorRegistration;
use async_trait::async_trait;
use axum::{http::HeaderMap, Router};
use cb_common::config::load_pbs_custom_config;
use cb_common::pbs::{
    GetHeaderParams, GetHeaderResponse, RelayClient, SignedBlindedBeaconBlock,
    SubmitBlindedBlockResponse,
};
use cb_pbs::{BuilderApi, PbsState};
use eyre;
use futures::future::join_all;
use parking_lot::RwLock;
use std::sync::Arc;

type PbsStateGuard<S> = Arc<RwLock<PbsState<S>>>;
use tracing::{debug, info, warn};

pub struct StateManagerReservedGasApi;

// Keep the old API for compatibility
#[allow(dead_code)]
pub struct TypestateReservedGasApi;

#[async_trait]
impl BuilderApi<GasReservationManagerWrapper> for StateManagerReservedGasApi {
    fn extra_routes() -> Option<Router<PbsStateGuard<GasReservationManagerWrapper>>> {
        None // Disable extra routes for now to get it compiling
    }

    async fn get_header(
        params: GetHeaderParams,
        _req_headers: HeaderMap,
        state: PbsState<GasReservationManagerWrapper>,
    ) -> eyre::Result<Option<GetHeaderResponse>> {
        let manager = state.data.0.clone();
        get_best_bid_with_gas_reservation(
            &state.config.relays,
            params,
            manager,
        )
        .await
        .map_err(|e| eyre::eyre!("Failed to get best bid: {}", e))
    }

    async fn register_validator(
        registrations: Vec<ValidatorRegistration>,
        _req_headers: HeaderMap,
        state: PbsState<GasReservationManagerWrapper>,
    ) -> eyre::Result<()> {
        let query_service = RelayQueryService::production();
        register_with_all_relays(&state.config.relays, &registrations, &query_service).await
    }

    async fn submit_block(
        signed_blinded_block: SignedBlindedBeaconBlock,
        _req_headers: HeaderMap,
        state: PbsState<GasReservationManagerWrapper>,
    ) -> eyre::Result<SubmitBlindedBlockResponse> {
        let query_service = RelayQueryService::production();
        submit_to_first_relay(&state.config.relays, &signed_blinded_block, &query_service).await
    }

    async fn get_status(
        _req_headers: HeaderMap,
        state: PbsState<GasReservationManagerWrapper>,
    ) -> eyre::Result<()> {
        let query_service = RelayQueryService::production();
        check_relay_health(&state.config.relays, &query_service).await
    }

    async fn reload(state: PbsState<GasReservationManagerWrapper>) -> eyre::Result<PbsState<GasReservationManagerWrapper>> {
        info!("Reloading reserved gas module configuration");

        // Load the new configuration
        let (pbs_config, extra_config) =
            load_pbs_custom_config::<crate::config::ReservedGasConfig>().await?;

        // Create new state manager
        let new_manager = Arc::new(GasReservationManager::new());
        new_manager.configure(extra_config.clone(), pbs_config.clone()).await?;

        // Sync if enabled
        let query_service = RelayQueryService::production();
        new_manager.sync_if_enabled(&pbs_config.relays, &query_service).await?;

        // Preserve existing reservations from old state
        // This ensures we don't lose runtime state during reload
        let old_reservations = state.data.0.get_all_reservations();
        for (relay_id, reservation) in old_reservations {
            new_manager.update_relay_configuration(
                relay_id,
                crate::config::RelayGasConfig {
                    reserved_gas_limit: reservation.reserved_gas,
                    min_gas_limit: reservation.min_gas_limit,
                    update_interval: None,
                },
            ).await.ok();
        }

        info!(
            default_reserved_gas = extra_config.default_reserved_gas,
            relay_overrides = extra_config.relay_overrides.len(),
            fetch_from_relays = extra_config.fetch_from_relays,
            "Configuration reloaded successfully"
        );

        Ok(PbsState::new(pbs_config).with_data(GasReservationManagerWrapper(new_manager)))
    }
}

/// Register validators with all relays
async fn register_with_all_relays(
    relays: &[RelayClient],
    registrations: &[ValidatorRegistration],
    query_service: &RelayQueryService,
) -> eyre::Result<()> {
    let futures: Vec<_> = relays
        .iter()
        .map(|relay| query_service.register_validators(relay, registrations))
        .collect();

    let results = futures::future::join_all(futures).await;

    let any_success = results.iter().any(|r| r.is_ok());

    if !any_success {
        return Err(ReservedGasError::ValidatorRegistrationFailed.into());
    }

    for (relay, result) in relays.iter().zip(results.iter()) {
        match result {
            Ok(_) => info!(relay_id = %relay.id, "Successfully registered validators"),
            Err(e) => warn!(relay_id = %relay.id, error = %e, "Failed to register validators"),
        }
    }

    Ok(())
}

/// Submit block to first available relay
async fn submit_to_first_relay(
    relays: &[RelayClient],
    block: &SignedBlindedBeaconBlock,
    query_service: &RelayQueryService,
) -> eyre::Result<SubmitBlindedBlockResponse> {
    for relay in relays {
        match query_service.submit_block(relay, block).await {
            Ok(response) => {
                info!(relay_id = %relay.id, "Successfully submitted block");
                return Ok(response);
            }
            Err(e) => {
                warn!(relay_id = %relay.id, error = %e, "Failed to submit block");
            }
        }
    }

    Err(ReservedGasError::BlockSubmissionFailed.into())
}

/// Check if at least one relay is healthy
async fn check_relay_health(
    relays: &[RelayClient],
    query_service: &RelayQueryService,
) -> eyre::Result<()> {
    let futures: Vec<_> = relays.iter().map(|relay| query_service.check_status(relay)).collect();

    let results = futures::future::join_all(futures).await;

    let any_healthy = results.iter().any(|r| r.is_ok());

    if !any_healthy {
        return Err(ReservedGasError::NoHealthyRelays.into());
    }

    Ok(())
}

/// Get the best bid with gas reservation applied
async fn get_best_bid_with_gas_reservation(
    relays: &[RelayClient],
    params: GetHeaderParams,
    manager: Arc<GasReservationManager>,
) -> Result<Option<GetHeaderResponse>> {
    let query_service = RelayQueryService::production();
    
    // Query all relays in parallel
    let futures: Vec<_> = relays
        .iter()
        .map(|relay| {
            let relay_id = relay.id.clone();
            let query_service = &query_service;
            let manager = manager.clone();
            
            async move {
                match query_service.query_header(relay, params).await {
                    Ok(Some(mut response)) => {
                        // Apply gas reservation
                        let original_gas_limit = response.gas_limit();
                        match manager.apply_reservation(&relay_id, original_gas_limit).await {
                            Ok(outcome) => {
                                // Modify the gas limit in the response
                                use cb_common::pbs::GetHeaderResponse;
                                match &mut response {
                                    GetHeaderResponse::Electra(data) => {
                                        data.message.header.gas_limit = outcome.new_gas_limit;
                                    }
                                }
                                debug!(
                                    relay_id = %relay_id,
                                    original_gas = original_gas_limit,
                                    reserved_gas = outcome.reserved_amount,
                                    new_gas = outcome.new_gas_limit,
                                    "Applied gas reservation to bid"
                                );
                                RelayQueryResult::Bid(AdjustedBid {
                                    response,
                                    original_gas_limit,
                                    reserved_gas: outcome.reserved_amount,
                                    relay_id: relay_id.clone(),
                                })
                            }
                            Err(e) => {
                                warn!(relay_id = %relay_id, error = %e, "Failed to apply gas reservation");
                                RelayQueryResult::Error(e.to_string())
                            }
                        }
                    }
                    Ok(None) => RelayQueryResult::NoBid,
                    Err(e) => {
                        warn!(relay_id = %relay_id, error = %e, "Failed to query relay");
                        RelayQueryResult::Error(e.to_string())
                    }
                }
            }
        })
        .collect();
    
    let results = join_all(futures).await;
    
    // Find the best bid
    let mut best_bid: Option<AdjustedBid> = None;
    
    for result in results {
        if let RelayQueryResult::Bid(bid) = result {
            if best_bid.is_none() || bid.response.value() > best_bid.as_ref().unwrap().response.value() {
                best_bid = Some(bid);
            }
        }
    }
    
    Ok(best_bid.map(|bid| bid.response))
}