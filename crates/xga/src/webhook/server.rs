use axum::{routing::post, Router};
use commit_boost::prelude::*;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Mutex};
use tower::ServiceBuilder;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::trace::TraceLayer;
use tracing::{error, info};

use crate::{
    commitment::RegistrationNotification,
    config::XGAConfig,
    eigenlayer::EigenLayerIntegration,
    infrastructure::{CircuitBreaker, HttpClientFactory, RateLimiter, MAX_REQUEST_SIZE},
};

use super::{
    handlers::{handle_registration, health_check},
    monitoring::{eigenlayer_monitoring_loop, nonce_cleanup_loop},
    processing::process_queue_loop,
    state::AppState,
};

/// Start the webhook server
pub async fn start_webhook_server(config: StartCommitModuleConfig<XGAConfig>) -> eyre::Result<()> {
    let port = config.extra.webhook_port;
    let config_arc = Arc::new(config);

    // Create channel for processing queue with backpressure
    let (tx, rx) = mpsc::channel::<RegistrationNotification>(100);

    // Initialize infrastructure components
    let circuit_breaker = Arc::new(CircuitBreaker::new(
        3,                       // failure threshold
        Duration::from_secs(60), // reset timeout
    ));
    let http_client_factory = Arc::new(HttpClientFactory::new());
    let rate_limiter = Arc::new(RateLimiter::new(
        100,                     // max 100 requests
        Duration::from_secs(60), // per minute
    ));

    // Initialize EigenLayer integration if enabled
    let eigenlayer = if config_arc.extra.eigenlayer.enabled {
        match EigenLayerIntegration::new(config_arc.extra.eigenlayer.clone(), config_arc.clone())
            .await
        {
            Ok(integration) => {
                info!("EigenLayer integration initialized (shadow mode)");
                Some(Arc::new(Mutex::new(integration)))
            }
            Err(e) => {
                error!("Failed to initialize EigenLayer integration: {}", e);
                return Err(e);
            }
        }
    } else {
        info!("EigenLayer integration disabled");
        None
    };

    // Create application state
    let app_state = AppState::new(
        config_arc.clone(),
        tx,
        circuit_breaker.clone(),
        http_client_factory.clone(),
        rate_limiter.clone(),
        eigenlayer.clone(),
    );

    // Spawn background processor
    let processor_config = config_arc.clone();
    let processor_nonce_tracker = app_state.nonce_tracker.clone();
    let processor_circuit_breaker = app_state.circuit_breaker.clone();
    let processor_http_client_factory = app_state.http_client_factory.clone();
    let processor_eigenlayer = app_state.eigenlayer.clone();
    tokio::spawn(async move {
        process_queue_loop(
            rx,
            processor_config,
            processor_nonce_tracker,
            processor_circuit_breaker,
            processor_http_client_factory,
            processor_eigenlayer,
        )
        .await;
    });

    // Spawn nonce cleanup task
    let cleanup_state = app_state.clone();
    tokio::spawn(async move {
        nonce_cleanup_loop(cleanup_state).await;
    });

    // Spawn EigenLayer monitoring task if enabled
    if let Some(eigenlayer_arc) = app_state.eigenlayer.clone() {
        tokio::spawn(async move {
            eigenlayer_monitoring_loop(eigenlayer_arc).await;
        });
    }

    // Create the router
    let app = create_router(app_state);

    // Start the server
    let addr = format!("0.0.0.0:{}", port);
    info!("Webhook server listening on {}", addr);

    axum::serve(tokio::net::TcpListener::bind(&addr).await?, app).await?;

    Ok(())
}

/// Create the Axum router with all routes and middleware
fn create_router(state: AppState) -> Router {
    Router::new()
        .route("/registration", post(handle_registration))
        .route("/health", axum::routing::get(health_check))
        .layer(
            ServiceBuilder::new()
                .layer(RequestBodyLimitLayer::new(MAX_REQUEST_SIZE))
                .layer(TraceLayer::new_for_http()),
        )
        .with_state(state)
}
