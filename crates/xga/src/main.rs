mod api;
mod config;
mod error;
mod metrics;
mod metrics_wrapper;
mod relay_client;
mod relay_communication;
mod service;
mod state;
mod state_traits;
mod types;
mod validation;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod property_tests;

#[cfg(test)]
mod test_matchers;

use api::ReservedGasApi;
use config::ReservedGasConfig;
use relay_client::config_fetcher_task;
use state::ReservedGasState;

use cb_common::{
    config::{load_pbs_custom_config, LogsSettings, PBS_MODULE_NAME},
    utils::initialize_tracing_log,
};
use cb_pbs::{PbsService, PbsState};
use eyre::Result;
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

    info!("Starting Reserved Gas Limit module");
    info!(
        default_reserved_gas = extra_config.default_reserved_gas,
        relay_overrides = extra_config.relay_overrides.len(),
        fetch_from_relays = extra_config.fetch_from_relays,
        "Loaded configuration"
    );

    // Initialize state
    let reserved_gas_state = ReservedGasState::new(extra_config.clone(), pbs_config.clone());

    // Start background config fetcher if enabled
    if extra_config.fetch_from_relays {
        let fetcher_state = reserved_gas_state.clone();
        let _fetcher_handle = tokio::spawn(async move {
            info!("Starting relay config fetcher task");
            loop {
                if let Err(e) = tokio::time::timeout(
                    std::time::Duration::from_secs(600), // 10 minute timeout
                    config_fetcher_task(fetcher_state.clone()),
                )
                .await
                {
                    warn!("Config fetcher task timed out or panicked: {:?}", e);
                    // Continue with a new instance
                }
                // Small delay before restarting to avoid tight loop
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
        });

        // Fetch initial configs with error handling
        let initial_state = reserved_gas_state.clone();
        tokio::spawn(async move {
            info!("Fetching initial relay configurations");
            initial_state.fetch_relay_configs().await;
        });
    }

    // Create PBS state with our custom data
    let state = PbsState::new(pbs_config).with_data(reserved_gas_state);

    // Initialize and register metrics
    PbsService::init_metrics(chain)?;

    // Register our custom metrics with PbsService
    let custom_metrics = metrics::init_metrics();
    for metric in custom_metrics {
        PbsService::register_metric(metric);
    }

    // Run the PBS service
    PbsService::run::<ReservedGasState, ReservedGasApi>(state).await
}
