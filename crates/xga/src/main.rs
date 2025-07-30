use commit_boost::prelude::*;
use eyre::Result;
use tracing::info;

use xga_commitment::{
    config::XGAConfig, metrics::XGA_METRICS_REGISTRY, webhook::start_webhook_server,
};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt::init();

    info!("Starting XGA Commitment Module");

    // Load configuration
    let config = load_commit_module_config::<XGAConfig>()?;

    // Validate configuration
    config.extra.validate()?;

    info!(
        module_id = %config.id,
        chain = ?config.chain,
        "Loaded and validated XGA module configuration"
    );

    // Metrics are initialized lazily via Lazy statics

    // Start metrics provider
    MetricsProvider::load_and_run(config.chain, XGA_METRICS_REGISTRY.clone())?;
    info!("Started metrics provider");

    // JWT tokens are managed internally by the signer client

    // Start webhook server
    info!(port = config.extra.webhook_port, "Starting webhook server");
    start_webhook_server(config).await?;

    Ok(())
}
