use crate::error::ReservedGasError;
use crate::relay_communication::RelayQueryService;
use crate::state::ReservedGasState;
use crate::state_traits::GasReservationManager;
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
use parking_lot::RwLock;
use std::sync::Arc;
use tracing::{info, warn};

/// Gas threshold for healthy relays (10M gas)
const RELAY_HEALTH_THRESHOLD_HEALTHY: u64 = 10_000_000;

/// Gas threshold for degraded relays (20M gas)
const RELAY_HEALTH_THRESHOLD_DEGRADED: u64 = 20_000_000;

pub struct ReservedGasApi;

#[async_trait]
impl BuilderApi<ReservedGasState> for ReservedGasApi {
    fn extra_routes() -> Option<Router<Arc<RwLock<PbsState<ReservedGasState>>>>> {
        Some(
            Router::new()
                .route("/xga/v2/elastic/capacity", axum::routing::get(get_gas_reservations))
                .route("/xga/v2/relay/:relay_id/config", axum::routing::post(update_relay_config))
                .route("/xga/v2/relay/reserve", axum::routing::get(get_relay_reserves)),
        )
    }

    async fn get_header(
        params: GetHeaderParams,
        _req_headers: HeaderMap,
        state: PbsState<ReservedGasState>,
    ) -> eyre::Result<Option<GetHeaderResponse>> {
        let service = state.data.create_service();
        service
            .get_best_bid(&state.config.relays, params)
            .await
            .map_err(|e| eyre::eyre!("Failed to get best bid: {}", e))
    }

    async fn register_validator(
        registrations: Vec<ValidatorRegistration>,
        _req_headers: HeaderMap,
        state: PbsState<ReservedGasState>,
    ) -> eyre::Result<()> {
        let query_service = RelayQueryService::production();
        register_with_all_relays(&state.config.relays, &registrations, &query_service).await
    }

    async fn submit_block(
        signed_blinded_block: SignedBlindedBeaconBlock,
        _req_headers: HeaderMap,
        state: PbsState<ReservedGasState>,
    ) -> eyre::Result<SubmitBlindedBlockResponse> {
        let query_service = RelayQueryService::production();
        submit_to_first_relay(&state.config.relays, &signed_blinded_block, &query_service).await
    }

    async fn get_status(
        _req_headers: HeaderMap,
        state: PbsState<ReservedGasState>,
    ) -> eyre::Result<()> {
        let query_service = RelayQueryService::production();
        check_relay_health(&state.config.relays, &query_service).await
    }

    async fn reload(state: PbsState<ReservedGasState>) -> eyre::Result<PbsState<ReservedGasState>> {
        info!("Reloading reserved gas module configuration");

        // Load the new configuration
        let (pbs_config, extra_config) =
            load_pbs_custom_config::<crate::config::ReservedGasConfig>().await?;

        // Create new state while preserving runtime data
        let mut new_state = ReservedGasState::new(extra_config, pbs_config.clone());

        // Preserve existing reservations and last fetch time
        // This ensures we don't lose runtime state during reload
        new_state.reservations = state.data.reservations.clone();
        new_state.last_fetch = state.data.last_fetch.clone();

        info!(
            default_reserved_gas = new_state.config.default_reserved_gas,
            relay_overrides = new_state.config.relay_overrides.len(),
            fetch_from_relays = new_state.config.fetch_from_relays,
            "Configuration reloaded successfully"
        );

        Ok(PbsState::new(pbs_config).with_data(new_state))
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

/// Get elastic gas capacity information for all relays.
///
/// This endpoint provides comprehensive information about current gas reservations including:
/// - Configuration settings (default gas, update intervals)
/// - Config staleness information
/// - Statistics (total/average/min/max reserved gas, relay health breakdown)
/// - Individual relay reservations with last update times
///
/// # Endpoint
///
/// `GET /xga/v2/elastic/capacity`
///
/// # Response Format
///
/// ```json
/// {
///   "default_reserved_gas": 1000000,
///   "fetch_from_relays": true,
///   "update_interval_secs": 300,
///   "config_stale": false,
///   "config_stale_duration_secs": null,
///   "statistics": {
///     "total_relays": 3,
///     "total_reserved_gas": 4500000,
///     "average_reserved_gas": 1500000,
///     "max_reserved_gas": 2000000,
///     "min_reserved_gas": 1000000,
///     "relay_health": {
///       "healthy": 3,
///       "degraded": 0,
///       "unhealthy": 0
///     }
///   },
///   "reservations": [...]
/// }
/// ```
pub async fn get_gas_reservations(
    axum::extract::State(state): axum::extract::State<Arc<RwLock<PbsState<ReservedGasState>>>>,
) -> eyre::Result<axum::Json<serde_json::Value>, axum::response::Response> {
    let state_guard = state.read();
    let reservations = state_guard.data.get_all_reservations();

    // Check config staleness
    let staleness = state_guard.data.check_config_staleness();

    let json_reservations: Vec<serde_json::Value> = reservations
        .iter()
        .map(|(relay_id, reservation)| {
            // Verify relay_id matches the stored one (for debugging)
            debug_assert_eq!(relay_id, &reservation.relay_id);
            serde_json::json!({
                "relay_id": reservation.relay_id,
                "reserved_gas": reservation.reserved_gas,
                "last_updated_secs_ago": reservation.last_updated.elapsed().as_secs(),
                "min_gas_limit": reservation.min_gas_limit,
            })
        })
        .collect();

    // Calculate statistics
    let total_relays = json_reservations.len();
    let total_reserved_gas: u64 = reservations.iter().map(|(_, r)| r.reserved_gas).sum();
    let avg_reserved_gas =
        if total_relays > 0 { total_reserved_gas / total_relays as u64 } else { 0 };
    let max_reserved_gas = reservations.iter().map(|(_, r)| r.reserved_gas).max().unwrap_or(0);
    let min_reserved_gas = reservations.iter().map(|(_, r)| r.reserved_gas).min().unwrap_or(0);

    // Count relays by health status based on their reserved gas
    let healthy_relays = reservations
        .iter()
        .filter(|(_, r)| r.reserved_gas <= RELAY_HEALTH_THRESHOLD_HEALTHY)
        .count();
    let degraded_relays = reservations
        .iter()
        .filter(|(_, r)| {
            r.reserved_gas > RELAY_HEALTH_THRESHOLD_HEALTHY
                && r.reserved_gas <= RELAY_HEALTH_THRESHOLD_DEGRADED
        })
        .count();
    let unhealthy_relays = reservations
        .iter()
        .filter(|(_, r)| r.reserved_gas > RELAY_HEALTH_THRESHOLD_DEGRADED)
        .count();

    Ok(axum::Json(serde_json::json!({
        "default_reserved_gas": state_guard.data.config.default_reserved_gas,
        "fetch_from_relays": state_guard.data.config.fetch_from_relays,
        "update_interval_secs": state_guard.data.config.update_interval_secs,
        "config_stale": staleness.is_some(),
        "config_stale_duration_secs": staleness.map(|d| d.as_secs()),
        "statistics": {
            "total_relays": total_relays,
            "total_reserved_gas": total_reserved_gas,
            "average_reserved_gas": avg_reserved_gas,
            "max_reserved_gas": max_reserved_gas,
            "min_reserved_gas": min_reserved_gas,
            "relay_health": {
                "healthy": healthy_relays,
                "degraded": degraded_relays,
                "unhealthy": unhealthy_relays,
            }
        },
        "reservations": json_reservations,
    })))
}

/// Update relay gas configuration dynamically.
///
/// Allows runtime updates to a relay's gas reservation configuration without
/// restarting the module. Changes take effect immediately for subsequent requests.
///
/// # Endpoint
///
/// `POST /xga/v2/relay/{relay_id}/config`
///
/// # Parameters
///
/// - `relay_id`: The relay identifier (e.g., "flashbots", "ultrasound")
///
/// # Request Body
///
/// ```json
/// {
///     "reserved_gas_limit": 2000000,
///     "min_gas_limit": 15000000,
///     "update_interval": 600
/// }
/// ```
///
/// # Response
///
/// Returns a `ConfigUpdateEvent` with all fields populated:
/// ```json
/// {
///     "relay_id": "flashbots",
///     "old_reservation": 1000000,
///     "new_reservation": 2000000,
///     "timestamp_secs_ago": 0
/// }
/// ```
pub async fn update_relay_config(
    axum::extract::Path(relay_id): axum::extract::Path<String>,
    axum::extract::State(state): axum::extract::State<Arc<RwLock<PbsState<ReservedGasState>>>>,
    axum::Json(config): axum::Json<crate::config::RelayGasConfig>,
) -> eyre::Result<axum::Json<serde_json::Value>, axum::response::Response> {
    let state_guard = state.read();
    let service = state_guard.data.create_service();

    // Call update_relay_configuration which uses trait methods and returns ConfigUpdateEvent
    let event = service.update_relay_configuration(relay_id, config);

    // Return the event as JSON, ensuring all fields are serialized and "used"
    Ok(axum::Json(serde_json::json!({
        "relay_id": event.relay_id,
        "old_reservation": event.old_reservation,
        "new_reservation": event.new_reservation,
        "timestamp_secs_ago": event.timestamp.elapsed().as_secs(),
    })))
}

/// Query all relays for their reserve requirements.
///
/// This endpoint queries each configured relay to get their minimum gas capacity
/// that must be reserved. Results are returned for all relays, including failures.
///
/// # Endpoint
///
/// `GET /xga/v2/relay/reserve`
///
/// # Response Format
///
/// ```json
/// {
///   "timestamp_secs_ago": 0,
///   "relay_reserves": [
///     {
///       "relay_id": "flashbots",
///       "reserve": 2000000,
///       "status": "success",
///       "query_time_ms": 45
///     },
///     {
///       "relay_id": "ultrasound",
///       "status": "timeout",
///       "query_time_ms": 2000,
///       "error": "Request timeout after 2000ms"
///     }
///   ],
///   "statistics": {
///     "total_relays": 2,
///     "successful_queries": 1,
///     "failed_queries": 1,
///     "average_reserve": 2000000,
///     "max_reserve": 2000000,
///     "min_reserve": 2000000
///   }
/// }
/// ```
pub async fn get_relay_reserves(
    axum::extract::State(state): axum::extract::State<Arc<RwLock<PbsState<ReservedGasState>>>>,
) -> eyre::Result<axum::Json<serde_json::Value>, axum::response::Response> {
    let (relays, endpoint) = {
        let state_guard = state.read();
        (state_guard.config.relays.clone(), state_guard.data.config.relay_reserve_endpoint.clone())
    };

    let query_service = RelayQueryService::production();
    let query_start = std::time::Instant::now();

    // Query all relays in parallel
    let futures: Vec<_> = relays
        .iter()
        .map(|relay| {
            let query_service = &query_service;
            let endpoint = endpoint.clone();
            let relay_id = (*relay.id).clone();

            async move {
                let start_time = std::time::Instant::now();
                let result = query_service.fetch_reserve_requirement(relay, &endpoint).await;
                let query_time_ms = start_time.elapsed().as_millis() as u64;

                match result {
                    Ok(Some(reserve)) => crate::types::RelayReserveInfo {
                        relay_id,
                        reserve: Some(reserve),
                        query_time_ms,
                        status: crate::types::ReserveQueryStatus::Success,
                        error: None,
                    },
                    Ok(None) => crate::types::RelayReserveInfo {
                        relay_id,
                        reserve: None,
                        query_time_ms,
                        status: crate::types::ReserveQueryStatus::Error,
                        error: Some("Relay does not support reserve endpoint".to_string()),
                    },
                    Err(e) => {
                        let (status, error_msg) = if e.to_string().contains("timeout") {
                            (
                                crate::types::ReserveQueryStatus::Timeout,
                                format!("Request timeout after {}ms", query_time_ms),
                            )
                        } else {
                            (crate::types::ReserveQueryStatus::Error, e.to_string())
                        };

                        crate::types::RelayReserveInfo {
                            relay_id,
                            reserve: None,
                            query_time_ms,
                            status,
                            error: Some(error_msg),
                        }
                    }
                }
            }
        })
        .collect();

    let results = futures::future::join_all(futures).await;

    // Calculate statistics
    let total_relays = results.len();
    let successful_queries = results
        .iter()
        .filter(|r| matches!(r.status, crate::types::ReserveQueryStatus::Success))
        .count();
    let failed_queries = total_relays - successful_queries;

    let successful_reserves: Vec<u64> = results.iter().filter_map(|r| r.reserve).collect();

    let statistics = if !successful_reserves.is_empty() {
        let total_reserve: u64 = successful_reserves.iter().sum();
        let average_reserve = total_reserve / successful_reserves.len() as u64;
        let max_reserve = *successful_reserves.iter().max().unwrap();
        let min_reserve = *successful_reserves.iter().min().unwrap();

        serde_json::json!({
            "total_relays": total_relays,
            "successful_queries": successful_queries,
            "failed_queries": failed_queries,
            "average_reserve": average_reserve,
            "max_reserve": max_reserve,
            "min_reserve": min_reserve,
        })
    } else {
        serde_json::json!({
            "total_relays": total_relays,
            "successful_queries": successful_queries,
            "failed_queries": failed_queries,
            "average_reserve": null,
            "max_reserve": null,
            "min_reserve": null,
        })
    };

    Ok(axum::Json(serde_json::json!({
        "timestamp_secs_ago": 0,  // Current time
        "relay_reserves": results,
        "statistics": statistics,
        "query_time_ms": query_start.elapsed().as_millis(),
    })))
}
