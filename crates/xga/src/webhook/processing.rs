use commit_boost::prelude::*;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, Mutex};
use tracing::{error, info, warn};

use crate::{
    commitment::{RegistrationNotification, XGACommitment},
    config::XGAConfig,
    eigenlayer::DefaultEigenLayerIntegration,
    infrastructure::{CircuitBreaker, HttpClientFactory},
    metrics::VALIDATOR_METRICS,
    signer::process_commitment,
};

use super::{state::NonceEntry, utils::extract_relay_id};

/// Background task to process queued registrations
pub async fn process_queue_loop(
    mut rx: mpsc::Receiver<RegistrationNotification>,
    config: Arc<StartCommitModuleConfig<XGAConfig>>,
    nonce_tracker: Arc<Mutex<HashMap<[u8; 32], NonceEntry>>>,
    circuit_breaker: Arc<CircuitBreaker>,
    http_client_factory: Arc<HttpClientFactory>,
    eigenlayer: Option<Arc<Mutex<DefaultEigenLayerIntegration>>>,
) {
    while let Some(notification) = rx.recv().await {
        // Add configured delay
        if config.extra.commitment_delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(config.extra.commitment_delay_ms)).await;
        }

        // Process the notification
        if let Err(e) = process_single_notification(
            notification,
            &config,
            &nonce_tracker,
            &circuit_breaker,
            &http_client_factory,
            &eigenlayer,
        )
        .await
        {
            error!("Failed to process notification: {}", e);
        }
    }
}

/// Process a single registration notification
async fn process_single_notification(
    notification: RegistrationNotification,
    config: &Arc<StartCommitModuleConfig<XGAConfig>>,
    nonce_tracker: &Arc<Mutex<HashMap<[u8; 32], NonceEntry>>>,
    circuit_breaker: &Arc<CircuitBreaker>,
    http_client_factory: &Arc<HttpClientFactory>,
    eigenlayer: &Option<Arc<Mutex<DefaultEigenLayerIntegration>>>,
) -> eyre::Result<()> {
    // Create XGA commitment
    let commitment = create_commitment_from_notification(&notification, config)?;

    // Track the nonce to prevent replay
    if !track_nonce(&commitment, nonce_tracker).await {
        warn!(
            "Duplicate nonce detected for validator {}",
            hex::encode(commitment.validator_pubkey.0)
        );
        return Ok(()); // Skip processing
    }

    // Track validator commitment processing
    VALIDATOR_METRICS.with_label_values(&["validator", "commitment_created"]).inc();

    // Handle EigenLayer registration if enabled
    if let Some(ref eigenlayer) = eigenlayer {
        handle_eigenlayer_registration(
            eigenlayer,
            &commitment,
            &notification.registration.message.fee_recipient,
        )
        .await?;
    }

    // Process the commitment (sign and send)
    match process_commitment(
        commitment,
        notification.relay_url,
        config.clone(),
        circuit_breaker,
        http_client_factory,
    )
    .await
    {
        Ok(()) => {
            VALIDATOR_METRICS.with_label_values(&["validator", "commitment_success"]).inc();
        }
        Err(e) => {
            VALIDATOR_METRICS.with_label_values(&["validator", "commitment_failed"]).inc();
            error!(
                validator_pubkey = ?notification.registration.message.pubkey,
                error = %e,
                "Failed to process XGA commitment"
            );
            return Err(e);
        }
    }

    Ok(())
}

/// Create XGA commitment from registration notification
fn create_commitment_from_notification(
    notification: &RegistrationNotification,
    config: &Arc<StartCommitModuleConfig<XGAConfig>>,
) -> eyre::Result<XGACommitment> {
    // Extract relay ID from URL
    let relay_id = extract_relay_id(&notification.relay_url);

    // Create XGA commitment
    let registration_hash = XGACommitment::hash_registration(&notification.registration);

    // Convert the validator pubkey bytes to BlsPublicKey
    let pubkey_bytes: [u8; 48] = notification.registration.message.pubkey.0;
    let validator_pubkey = BlsPublicKey::from(pubkey_bytes);

    let commitment = XGACommitment::new(
        registration_hash,
        validator_pubkey,
        relay_id,
        config.chain.id(),
        Default::default(), // XGA parameters
    );

    Ok(commitment)
}

/// Track nonce to prevent replay attacks
async fn track_nonce(
    commitment: &XGACommitment,
    nonce_tracker: &Arc<Mutex<HashMap<[u8; 32], NonceEntry>>>,
) -> bool {
    let mut tracker = nonce_tracker.lock().await;

    // Check if nonce already exists (potential replay attack)
    let nonce_bytes = commitment.nonce.into_bytes();
    if let Some(existing_entry) = tracker.get(&nonce_bytes) {
        warn!(
            "Duplicate nonce detected! Validator {} attempted to reuse nonce. Original validator: {}",
            hex::encode(commitment.validator_pubkey.0),
            hex::encode(existing_entry.validator_pubkey.0)
        );
        return false;
    }

    tracker.insert(
        nonce_bytes,
        NonceEntry { created_at: Instant::now(), validator_pubkey: commitment.validator_pubkey },
    );

    true
}

/// Handle EigenLayer registration if enabled
async fn handle_eigenlayer_registration(
    eigenlayer: &Arc<Mutex<DefaultEigenLayerIntegration>>,
    commitment: &XGACommitment,
    operator_address: &alloy::primitives::Address,
) -> eyre::Result<()> {
    let mut integration = eigenlayer.lock().await;

    // Check if this operator is registered (lazy registration)
    if !integration.is_registered(*operator_address).await.unwrap_or(false) {
        info!("First validator registration detected, tracking in EigenLayer shadow mode");

        match integration.register_commitment(commitment, commitment.validator_pubkey).await {
            Ok(_) => {
                info!("Successfully tracked XGA commitment in EigenLayer shadow mode");
            }
            Err(e) => {
                error!("Failed to track in EigenLayer: {}", e);
                // Continue processing even if tracking fails
            }
        }
    } else {
        // Check if commitment needs updating
        let commitment_hash = commitment.get_tree_hash_root();
        if integration.requires_update(commitment_hash.0.into()) {
            info!("Commitment change detected, updating EigenLayer tracking");

            match integration.update_commitment(commitment, commitment.validator_pubkey).await {
                Ok(_) => {
                    info!("Successfully updated XGA commitment tracking in EigenLayer");
                }
                Err(e) => {
                    error!("Failed to update EigenLayer tracking: {}", e);
                    // Continue processing even if update fails
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    #[tokio::test]
    async fn test_nonce_replay_protection() {
        // Create a nonce tracker
        let nonce_tracker: Arc<Mutex<HashMap<[u8; 32], NonceEntry>>> =
            Arc::new(Mutex::new(HashMap::new()));

        // Create a test commitment
        let commitment = XGACommitment::new(
            [0x42u8; 32],
            BlsPublicKey::from([0x01u8; 48]),
            "test-relay".to_string(),
            1,
            crate::commitment::XGAParameters::default(),
        );

        // First insertion should succeed
        assert!(track_nonce(&commitment, &nonce_tracker).await);

        // Second insertion with same nonce should fail
        assert!(!track_nonce(&commitment, &nonce_tracker).await);

        // Different commitment with different nonce should succeed
        let commitment2 = XGACommitment::new(
            [0x43u8; 32],
            BlsPublicKey::from([0x02u8; 48]),
            "test-relay".to_string(),
            1,
            crate::commitment::XGAParameters::default(),
        );
        assert!(track_nonce(&commitment2, &nonce_tracker).await);
    }
}
