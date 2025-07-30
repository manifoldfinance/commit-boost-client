use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use tokio::sync::Mutex;
use tracing::{debug, error, info};

use super::state::AppState;
use crate::infrastructure::get_current_timestamp;

/// Nonce expiration time (5 minutes)
const NONCE_EXPIRATION: Duration = Duration::from_secs(300);

/// Deduplication expiration time (5 minutes)
const DEDUP_EXPIRATION: Duration = Duration::from_secs(300);

/// EigenLayer monitoring interval (5 minutes)
const MONITORING_INTERVAL: Duration = Duration::from_secs(300);

/// Background task to clean up expired nonces and deduplication entries
pub async fn nonce_cleanup_loop(state: AppState) {
    loop {
        // Clean up expired nonces
        {
            let mut nonce_tracker = state.nonce_tracker.lock().await;
            let now = Instant::now();
            let initial_count = nonce_tracker.len();

            nonce_tracker.retain(|_nonce, entry| {
                let age = now.duration_since(entry.created_at);
                if age >= NONCE_EXPIRATION {
                    info!(
                        "Cleaning up expired nonce for validator: {}",
                        hex::encode(entry.validator_pubkey.0)
                    );
                    false
                } else {
                    true
                }
            });

            let cleaned_count = initial_count - nonce_tracker.len();
            if cleaned_count > 0 {
                info!("Cleaned up {} expired nonces", cleaned_count);
            }
        }

        // Clean up old deduplication entries
        {
            let mut dedup_tracker = state.dedup_tracker.lock().await;
            let current_time = match get_current_timestamp() {
                Ok(t) => t,
                Err(e) => {
                    error!("Failed to get current timestamp: {}", e);
                    continue;
                }
            };

            // Remove entries older than DEDUP_EXPIRATION
            dedup_tracker.retain(|(_, timestamp, _)| {
                current_time.saturating_sub(*timestamp) < DEDUP_EXPIRATION.as_secs()
            });
        }

        // Run cleanup every minute
        tokio::time::sleep(Duration::from_secs(60)).await;
    }
}

/// Background task for periodic EigenLayer shadow mode monitoring
pub async fn eigenlayer_monitoring_loop(
    eigenlayer: Arc<Mutex<crate::eigenlayer::DefaultEigenLayerIntegration>>,
) {
    info!("Starting EigenLayer shadow mode monitoring loop");

    loop {
        // Get a mutable reference to the integration
        let mut integration = eigenlayer.lock().await;

        // Check if monitoring is configured
        if integration.is_monitoring_configured() {
            info!("Running periodic EigenLayer shadow mode monitoring");

            // Run the periodic monitoring
            match integration.run_periodic_monitoring().await {
                Ok(()) => {
                    debug!("EigenLayer monitoring completed successfully");

                    // Log cached status for metrics
                    if let Some(status) = integration.get_cached_status() {
                        info!(
                            "Shadow mode status - Registered: {}, Rewards: {}, Penalty: {}%",
                            status.is_registered, status.pending_rewards, status.penalty_rate
                        );
                    }
                }
                Err(e) => {
                    error!("Failed to run EigenLayer monitoring: {}", e);
                }
            }
        } else {
            debug!("EigenLayer monitoring not configured, skipping");
        }

        // Release the lock before sleeping
        drop(integration);

        // Sleep for the monitoring interval
        tokio::time::sleep(MONITORING_INTERVAL).await;
    }
}
