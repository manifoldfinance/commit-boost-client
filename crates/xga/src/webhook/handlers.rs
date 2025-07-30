use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use commit_boost::prelude::*;
use tracing::{error, info, warn};

use crate::{
    commitment::RegistrationNotification,
    metrics::{COMMITMENTS_QUEUED, REGISTRATIONS_RECEIVED, VALIDATOR_METRICS},
    relay::check_xga_support,
};

use super::{state::AppState, validation::validate_notification};

/// Handle incoming registration notifications
pub async fn handle_registration(
    State(state): State<AppState>,
    Json(notification): Json<RegistrationNotification>,
) -> impl IntoResponse {
    // Check rate limit
    if !state.rate_limiter.check_rate_limit().await {
        warn!("Rate limit exceeded for registration webhook");
        return (StatusCode::TOO_MANY_REQUESTS, "Rate limit exceeded".to_string());
    }

    REGISTRATIONS_RECEIVED.inc();

    info!(
        validator_pubkey = ?notification.registration.message.pubkey,
        relay_url = %notification.relay_url,
        "Received registration notification"
    );

    // Track validator registration received with fixed label
    VALIDATOR_METRICS.with_label_values(&["validator", "registration_received"]).inc();

    // Validate the notification
    if let Err(e) = validate_notification(&notification, &state.config.extra) {
        warn!("Invalid registration notification: {}", e);
        return (StatusCode::BAD_REQUEST, e.to_string());
    }

    // Check if this is an XGA relay
    if !state.config.extra.is_xga_relay(&notification.relay_url) {
        info!("Relay {} is not XGA-enabled, ignoring", notification.relay_url);
        return (StatusCode::OK, "Not an XGA relay".to_string());
    }

    // Check for duplicate registration
    let pubkey_bytes: [u8; 48] = notification.registration.message.pubkey.0;
    let bls_pubkey = BlsPublicKey::from(pubkey_bytes);
    let dedup_key = (bls_pubkey, notification.timestamp, notification.relay_url.clone());
    {
        let mut dedup = state.dedup_tracker.lock().await;
        if dedup.contains(&dedup_key) {
            info!(
                validator_pubkey = ?notification.registration.message.pubkey,
                "Duplicate registration detected, ignoring"
            );
            return (StatusCode::OK, "Duplicate registration".to_string());
        }
        dedup.insert(dedup_key);
    }

    // Double-check by probing the relay (optional, can be disabled in config)
    if state.config.extra.probe_relay_capabilities
        && !check_xga_support(&notification.relay_url, &state.http_client_factory).await
    {
        warn!(
            relay_url = %notification.relay_url,
            "Relay is configured as XGA but probe failed"
        );
        // Still proceed if configured, just log the warning
    }

    // Queue for processing via channel
    match state.processing_sender.send(notification).await {
        Ok(()) => {
            COMMITMENTS_QUEUED.inc();
            (StatusCode::OK, "Registration queued for XGA commitment".to_string())
        }
        Err(_) => {
            error!("Processing queue full, dropping registration");
            (StatusCode::SERVICE_UNAVAILABLE, "Processing queue full".to_string())
        }
    }
}

/// Health check endpoint handler
pub async fn health_check() -> impl IntoResponse {
    "OK"
}
