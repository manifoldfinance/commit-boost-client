use std::{sync::Arc, time::Duration};

use commit_boost::prelude::*;
use eyre::Result;
use tracing::{error, info};
use xga_commitment::{
    config::XgaConfig,
    infrastructure::{CircuitBreaker, HttpClientFactory, ValidatorRateLimiter},
    poller::{poll_and_process_relay, RelayPoller},
};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt::init();

    info!("Starting XGA Commitment Module");

    // Load configuration
    let config = load_commit_module_config::<XgaConfig>()?;

    // Validate configuration
    config.extra.validate()?;

    info!(
        module_id = %config.id,
        chain = ?config.chain,
        polling_interval_secs = config.extra.polling_interval_secs,
        relay_count = config.extra.xga_relays.len(),
        "Loaded and validated XGA module configuration"
    );

    // Create infrastructure components
    let http_client_factory = Arc::new(HttpClientFactory::new());
    let circuit_breaker = Arc::new(CircuitBreaker::new(
        3,                       // failure threshold
        Duration::from_secs(60), // reset timeout
    ));
    
    // Create validator rate limiter
    let validator_rate_limiter = Arc::new(ValidatorRateLimiter::new(
        config.extra.validator_rate_limit.max_requests_per_validator,
        Duration::from_secs(config.extra.validator_rate_limit.window_secs),
    ));

    // Create poller
    let config_arc = Arc::new(config);
    let poller = Arc::new(RelayPoller::new(
        config_arc.clone(),
        http_client_factory,
        circuit_breaker,
        validator_rate_limiter,
    )?);

    info!("Starting polling loop");

    // Set up graceful shutdown
    let shutdown = tokio::signal::ctrl_c();
    let polling_task = polling_loop(config_arc, poller);

    tokio::select! {
        _ = shutdown => {
            info!("Received shutdown signal, stopping gracefully...");
        }
        result = polling_task => {
            if let Err(e) = result {
                error!("Polling loop error: {}", e);
                return Err(e);
            }
        }
    }

    info!("XGA module shutdown complete");
    Ok(())
}

async fn polling_loop(
    config: Arc<StartCommitModuleConfig<XgaConfig>>,
    poller: Arc<RelayPoller>,
) -> Result<()> {
    let mut interval = tokio::time::interval(Duration::from_secs(
        config.extra.polling_interval_secs,
    ));

    // Don't wait for the first tick
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        interval.tick().await;

        // Spawn concurrent tasks for all relays
        let mut tasks = vec![];
        for relay_url in &config.extra.xga_relays {
            let relay_url = relay_url.clone();
            let poller = poller.clone();

            let task = tokio::spawn(async move {
                if let Err(e) = poll_and_process_relay(relay_url.clone(), poller).await {
                    error!(
                        relay_url = %relay_url,
                        error = %e,
                        "Error polling relay"
                    );
                }
            });

            tasks.push(task);
        }

        // Wait for all polling tasks to complete
        for task in tasks {
            if let Err(e) = task.await {
                error!("Task join error: {}", e);
            }
        }
    }
}