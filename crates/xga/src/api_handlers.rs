use crate::state_manager::GasReservationManagerWrapper;
use axum::{extract::{Path, State}, Json};
use cb_pbs::PbsState;
use parking_lot::RwLock;
use serde_json::{json, Value};
use std::sync::Arc;
// No tracing imports needed

use crate::relay_communication::RelayQueryService;

// Gas threshold for healthy relays (10M gas)
const RELAY_HEALTH_THRESHOLD_HEALTHY: u64 = 10_000_000;

// Gas threshold for degraded relays (20M gas)
const RELAY_HEALTH_THRESHOLD_DEGRADED: u64 = 20_000_000;

/// Get elastic gas capacity information for all relays
pub async fn get_gas_reservations(
    State(state): State<Arc<RwLock<PbsState<GasReservationManagerWrapper>>>>,
) -> Json<Value> {
    let (manager, reservations) = {
        let state_guard = state.read();
        let manager = state_guard.data.0.clone();
        let reservations = manager.get_all_reservations();
        (manager, reservations)
    };

    // Check if configuration is stale
    let is_stale = !manager.check_freshness().await.unwrap_or(true);

    // Get the config from the manager's state
    let state_info = manager.get_state().await;
    let config = match state_info {
        crate::state_manager::GasReservationState::LocalOnly { config, .. } => config,
        crate::state_manager::GasReservationState::Synced { config, .. } => config,
        crate::state_manager::GasReservationState::Stale { config, .. } => config,
        crate::state_manager::GasReservationState::Uninitialized => {
            return Json(json!({
                "error": "Manager not configured"
            }))
        }
    };

    let json_reservations: Vec<Value> = reservations
        .iter()
        .map(|(relay_id, reservation)| {
            debug_assert_eq!(relay_id, &reservation.relay_id);
            json!({
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

    // Count relays by health status
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

    Json(json!({
        "default_reserved_gas": config.default_reserved_gas,
        "fetch_from_relays": config.fetch_from_relays,
        "update_interval_secs": config.update_interval_secs,
        "config_stale": is_stale,
        "config_stale_duration_secs": if is_stale { Some(config.update_interval_secs) } else { None },
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
    }))
}

/// Update relay gas configuration dynamically
pub async fn update_relay_config(
    Path(relay_id): Path<String>,
    State(state): State<Arc<RwLock<PbsState<GasReservationManagerWrapper>>>>,
    Json(config): Json<crate::config::RelayGasConfig>,
) -> Json<Value> {
    let state_guard = state.read();
    let manager = state_guard.data.0.clone();
    
    // Get old reservation if exists
    let old_reservation = manager.get_all_reservations()
        .into_iter()
        .find(|(id, _)| id == &relay_id)
        .map(|(_, r)| r.reserved_gas);
    
    let new_reservation = config.reserved_gas_limit;
    let timestamp = std::time::Instant::now();
    
    // Update the configuration
    if let Err(e) = manager.update_relay_configuration(relay_id.clone(), config).await {
        return Json(json!({
            "error": e.to_string()
        }));
    }

    Json(json!({
        "relay_id": relay_id,
        "old_reservation": old_reservation,
        "new_reservation": new_reservation,
        "timestamp_secs_ago": timestamp.elapsed().as_secs(),
    }))
}

/// Query all relays for their reserve requirements
pub async fn get_relay_reserves(
    State(state): State<Arc<RwLock<PbsState<GasReservationManagerWrapper>>>>,
) -> Json<Value> {
    let (relays, endpoint, _manager) = {
        let state_guard = state.read();
        let manager = state_guard.data.0.clone();
        
        // Get the config from the manager's state
        let state_info = manager.get_state().await;
        let config = match state_info {
            crate::state_manager::GasReservationState::LocalOnly { config, .. } => config,
            crate::state_manager::GasReservationState::Synced { config, .. } => config,
            crate::state_manager::GasReservationState::Stale { config, .. } => config,
            crate::state_manager::GasReservationState::Uninitialized => {
                return Json(json!({
                    "error": "Manager not configured"
                }))
            }
        };
        (state_guard.config.relays.clone(), config.relay_reserve_endpoint.clone(), manager)
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

        json!({
            "total_relays": total_relays,
            "successful_queries": successful_queries,
            "failed_queries": failed_queries,
            "average_reserve": average_reserve,
            "max_reserve": max_reserve,
            "min_reserve": min_reserve,
        })
    } else {
        json!({
            "total_relays": total_relays,
            "successful_queries": successful_queries,
            "failed_queries": failed_queries,
            "average_reserve": null,
            "max_reserve": null,
            "min_reserve": null,
        })
    };

    Json(json!({
        "timestamp_secs_ago": 0,
        "relay_reserves": results,
        "statistics": statistics,
        "query_time_ms": query_start.elapsed().as_millis(),
    }))
}