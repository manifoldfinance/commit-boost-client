mod api;
#[allow(dead_code)]
mod api_handlers;
mod cache;
mod config;
mod error;
mod metrics;
mod metrics_wrapper;
mod relay_communication;
mod state_manager;
mod types;
mod validation;

// Old modules replaced by state_manager
// mod typestate;
// mod service;

// Tests have been updated for typestate
#[cfg(test)]
mod tests;

#[cfg(test)]
mod property_tests;

#[cfg(test)]
mod test_matchers;

use config::ReservedGasConfig;
use state_manager::{GasReservationManager, GasReservationManagerWrapper};
use api::StateManagerReservedGasApi;

use cb_common::{
    config::{load_pbs_custom_config, LogsSettings, PBS_MODULE_NAME},
    utils::initialize_tracing_log,
};
use cb_pbs::{PbsService, PbsState};
use eyre::Result;
use relay_communication::RelayQueryService;
use std::sync::Arc;
use tracing::{info, warn};

/// Module identifier for logging and metrics
pub const RESERVED_GAS_MODULE_NAME: &str = "reserved-gas";

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;

    // Load PBS config and our custom config
    let (pbs_config, extra_config) = load_pbs_custom_config::<ReservedGasConfig>().await?;
    let chain = pbs_config.chain;

    // Validate configuration
    extra_config.validate()?;

    // Initialize tracing
    let _guard = initialize_tracing_log(PBS_MODULE_NAME, LogsSettings::from_env_config()?)?;

    info!("Starting Reserved Gas Limit module (State Manager Implementation)");
    info!(
        default_reserved_gas = extra_config.default_reserved_gas,
        relay_overrides = extra_config.relay_overrides.len(),
        fetch_from_relays = extra_config.fetch_from_relays,
        "Loaded configuration"
    );

    // Initialize state manager
    let manager = Arc::new(GasReservationManager::new());
    manager.configure(extra_config.clone(), pbs_config.clone()).await?;

    // Sync with relays if enabled
    let query_service = RelayQueryService::production();
    manager.sync_if_enabled(&pbs_config.relays, &query_service).await?;

    // Start background config fetcher if enabled
    if extra_config.fetch_from_relays {
        let bg_manager = manager.clone();
        let bg_relays = pbs_config.relays.clone();
        let bg_config = extra_config.clone();
        let bg_query_service = RelayQueryService::production();
        let _fetcher_handle = tokio::spawn(async move {
            info!("Starting background task for automatic config updates");
            let mut interval = tokio::time::interval(
                std::time::Duration::from_secs(bg_config.update_interval_secs)
            );
            loop {
                interval.tick().await;
                
                // Check freshness and refresh if needed
                match bg_manager.check_freshness().await {
                    Ok(false) => {
                        info!("Configuration is stale, refreshing...");
                        if let Err(e) = bg_manager.refresh(&bg_relays, &bg_query_service).await {
                            warn!("Failed to refresh configuration: {}", e);
                        }
                    }
                    Ok(true) => {
                        // Configuration is still fresh
                    }
                    Err(e) => {
                        warn!("Failed to check configuration freshness: {}", e);
                    }
                }
            }
        });
    }

    // Create PBS state with our state manager wrapper
    let wrapper = GasReservationManagerWrapper(manager);
    let state = PbsState::new(pbs_config).with_data(wrapper);

    // Initialize and register metrics
    PbsService::init_metrics(chain)?;

    // Register our custom metrics with PbsService
    let custom_metrics = metrics::init_metrics();
    for metric in custom_metrics {
        PbsService::register_metric(metric);
    }

    // Run the PBS service with state manager
    PbsService::run::<GasReservationManagerWrapper, StateManagerReservedGasApi>(state).await
}