use std::time::Duration;

use reqwest::{Client, StatusCode};
use serde::Deserialize;
use tracing::{debug, error, info, warn};

use crate::{
    commitment::SignedXGACommitment,
    infrastructure::{CircuitBreaker, Error as XGAError, HttpClientFactory},
    metrics::{
        COMMITMENTS_ACCEPTED, COMMITMENTS_REJECTED, COMMITMENTS_SENT, RELAY_ERRORS, RELAY_METRICS,
    },
};

/// Response from XGA relay
#[derive(Debug, Deserialize)]
struct XGARelayResponse {
    success: bool,
    message: Option<String>,
    commitment_id: Option<String>,
}

/// Send signed XGA commitment to relay
pub async fn send_to_relay(
    signed_commitment: SignedXGACommitment,
    relay_url: String,
    max_retries: u32,
    retry_delay_ms: u64,
    circuit_breaker: &CircuitBreaker,
    http_client_factory: &HttpClientFactory,
) -> eyre::Result<()> {
    // Check circuit breaker first
    if circuit_breaker.is_open(&relay_url).await {
        return Err(XGAError::CircuitBreakerOpen(relay_url.clone()).into());
    }

    // Create client using factory
    let client = http_client_factory.create_client()?;

    // Construct XGA endpoint URL
    let xga_endpoint = format!("{}/eth/v1/builder/xga/commitment", relay_url.trim_end_matches('/'));

    let mut attempts = 0;
    let mut last_error = None;

    while attempts < max_retries {
        attempts += 1;

        debug!(
            validator_pubkey = ?signed_commitment.message.validator_pubkey,
            relay_id = ?signed_commitment.message.relay_id,
            attempt = attempts,
            "Sending XGA commitment to relay"
        );

        match send_commitment(&client, &xga_endpoint, &signed_commitment).await {
            Ok(()) => {
                COMMITMENTS_SENT.inc();
                COMMITMENTS_ACCEPTED.inc();

                // Track relay-specific metrics
                RELAY_METRICS.with_label_values(&["relay", "commitment_sent", "success"]).inc();

                // Record success in circuit breaker
                circuit_breaker.record_success(&relay_url).await;

                info!(
                    validator_pubkey = ?signed_commitment.message.validator_pubkey,
                    relay_id = ?signed_commitment.message.relay_id,
                    "Successfully sent XGA commitment to relay"
                );
                return Ok(());
            }
            Err(e) => {
                last_error = Some(e);

                // Record failure in circuit breaker
                circuit_breaker.record_failure(&relay_url).await;

                if attempts < max_retries {
                    if let Some(ref err) = last_error {
                        warn!(
                            validator_pubkey = ?signed_commitment.message.validator_pubkey,
                            relay_id = ?signed_commitment.message.relay_id,
                            attempt = attempts,
                            error = %err,
                            "Failed to send XGA commitment, retrying"
                        );
                    }

                    tokio::time::sleep(Duration::from_millis(retry_delay_ms)).await;
                }
            }
        }
    }

    // All retries failed
    RELAY_ERRORS.inc();
    COMMITMENTS_REJECTED.inc();

    // Track relay-specific failure metrics
    RELAY_METRICS.with_label_values(&["relay", "commitment_sent", "failed"]).inc();

    match last_error {
        Some(err) => {
            error!(
                validator_pubkey = ?signed_commitment.message.validator_pubkey,
                relay_id = ?signed_commitment.message.relay_id,
                attempts = attempts,
                error = %err,
                "Failed to send XGA commitment after all retries"
            );
            Err(err)
        }
        None => {
            error!(
                validator_pubkey = ?signed_commitment.message.validator_pubkey,
                relay_id = ?signed_commitment.message.relay_id,
                attempts = attempts,
                "Failed to send XGA commitment after all retries (no error details)"
            );
            Err(eyre::eyre!("Failed to send XGA commitment after {} retries", attempts))
        }
    }
}

/// Send a single commitment attempt
async fn send_commitment(
    client: &Client,
    endpoint: &str,
    signed_commitment: &SignedXGACommitment,
) -> eyre::Result<()> {
    let response = client.post(endpoint).json(signed_commitment).send().await?;

    let status = response.status();

    match status {
        StatusCode::OK | StatusCode::CREATED | StatusCode::ACCEPTED => {
            // Try to parse response
            match response.json::<XGARelayResponse>().await {
                Ok(relay_response) => {
                    if relay_response.success {
                        debug!(
                            commitment_id = ?relay_response.commitment_id,
                            "Relay accepted XGA commitment"
                        );
                        Ok(())
                    } else {
                        Err(eyre::eyre!(
                            "Relay rejected commitment: {}",
                            relay_response.message.unwrap_or_default()
                        ))
                    }
                }
                Err(_) => {
                    // If we can't parse the response but got a success status, assume success
                    Ok(())
                }
            }
        }
        StatusCode::BAD_REQUEST => {
            let error_text = response.text().await.unwrap_or_default();
            Err(eyre::eyre!("Bad request: {}", error_text))
        }
        StatusCode::UNAUTHORIZED => Err(eyre::eyre!("Unauthorized: relay requires authentication")),
        StatusCode::NOT_FOUND => {
            Err(eyre::eyre!("XGA endpoint not found - relay may not support XGA"))
        }
        StatusCode::TOO_MANY_REQUESTS => Err(eyre::eyre!("Rate limited by relay")),
        _ => {
            let error_text = response.text().await.unwrap_or_default();
            Err(eyre::eyre!("Relay returned {}: {}", status, error_text))
        }
    }
}

/// Check if a relay supports XGA by probing the capability endpoint
pub async fn check_xga_support(relay_url: &str, http_client_factory: &HttpClientFactory) -> bool {
    let client = match http_client_factory.create_client_with_timeout(Duration::from_secs(5)) {
        Ok(c) => c,
        Err(e) => {
            error!("Failed to create HTTP client: {}", e);
            RELAY_METRICS.with_label_values(&["relay", "capability_check", "error"]).inc();
            return false;
        }
    };

    // Try multiple approaches to detect XGA support

    // Approach 1: Check for dedicated XGA capabilities endpoint
    let capabilities_endpoint =
        format!("{}/eth/v1/builder/xga/capabilities", relay_url.trim_end_matches('/'));

    debug!(relay_url = relay_url, "Checking XGA support via capabilities endpoint");

    match client.get(&capabilities_endpoint).send().await {
        Ok(response) => {
            if response.status().is_success() {
                info!(
                    relay_url = relay_url,
                    "Relay advertises XGA support via capabilities endpoint"
                );
                RELAY_METRICS.with_label_values(&["relay", "capability_check", "supported"]).inc();
                return true;
            }
        }
        Err(_) => {
            // Continue to next approach
        }
    }

    // Approach 2: Try OPTIONS request on the commitment endpoint
    let commitment_endpoint =
        format!("{}/eth/v1/builder/xga/commitment", relay_url.trim_end_matches('/'));

    match client.request(reqwest::Method::OPTIONS, &commitment_endpoint).send().await {
        Ok(response) => {
            // Check if XGA methods are allowed
            if let Some(allow_header) = response.headers().get("allow") {
                if let Ok(allow_str) = allow_header.to_str() {
                    if allow_str.contains("POST") {
                        info!(
                            relay_url = relay_url,
                            "Relay supports POST to XGA commitment endpoint"
                        );
                        RELAY_METRICS
                            .with_label_values(&["relay", "capability_check", "supported"])
                            .inc();
                        return true;
                    }
                }
            }

            // Also check for custom XGA headers
            if response.headers().contains_key("x-xga-supported") {
                info!(relay_url = relay_url, "Relay advertises XGA support via custom header");
                RELAY_METRICS.with_label_values(&["relay", "capability_check", "supported"]).inc();
                return true;
            }
        }
        Err(_) => {
            // Continue to next approach
        }
    }

    // Approach 3: Try a HEAD request to check endpoint existence
    match client.head(&commitment_endpoint).send().await {
        Ok(response) => {
            // If we get anything other than 404/405, assume the endpoint exists
            match response.status() {
                StatusCode::NOT_FOUND => {
                    debug!(
                        relay_url = relay_url,
                        "XGA endpoint not found - relay does not support XGA"
                    );
                    RELAY_METRICS
                        .with_label_values(&["relay", "capability_check", "not_supported"])
                        .inc();
                    false
                }
                StatusCode::METHOD_NOT_ALLOWED => {
                    // This actually suggests the endpoint exists but doesn't support HEAD
                    info!(
                        relay_url = relay_url,
                        "XGA endpoint exists (HEAD not allowed) - assuming XGA support"
                    );
                    RELAY_METRICS
                        .with_label_values(&["relay", "capability_check", "supported"])
                        .inc();
                    true
                }
                _ => {
                    info!(
                        relay_url = relay_url,
                        status = ?response.status(),
                        "XGA endpoint responded - assuming XGA support"
                    );
                    RELAY_METRICS
                        .with_label_values(&["relay", "capability_check", "supported"])
                        .inc();
                    true
                }
            }
        }
        Err(e) => {
            warn!(
                relay_url = relay_url,
                error = %e,
                "Failed to check XGA support - assuming not supported"
            );
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use commit_boost::prelude::*;

    use super::*;
    use crate::{
        commitment::{XGACommitment, XGAParameters},
        infrastructure::HttpClientFactory,
    };

    #[tokio::test]
    async fn test_send_commitment_error_handling() {
        let commitment = XGACommitment::new(
            [0x42u8; 32],
            BlsPublicKey::from([0x01u8; 48]),
            "test-relay".to_string(),
            1,
            XGAParameters::default(),
        );

        let signed = SignedXGACommitment {
            message: commitment,
            signature: BlsSignature::from([0x02u8; 96]),
        };

        // Test 400 Bad Request
        let mut server = mockito::Server::new_async().await;
        let _m400 = server
            .mock("POST", "/eth/v1/builder/xga/commitment")
            .with_status(400)
            .with_body("Invalid commitment format")
            .create();

        let result = send_commitment(
            &Client::new(),
            &format!("{}/eth/v1/builder/xga/commitment", server.url()),
            &signed,
        )
        .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Bad request"));

        // Test 401 Unauthorized
        let mut server = mockito::Server::new_async().await;
        let _m401 = server.mock("POST", "/eth/v1/builder/xga/commitment").with_status(401).create();

        let result = send_commitment(
            &Client::new(),
            &format!("{}/eth/v1/builder/xga/commitment", server.url()),
            &signed,
        )
        .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Unauthorized"));

        // Test 404 Not Found
        let mut server = mockito::Server::new_async().await;
        let _m404 = server.mock("POST", "/eth/v1/builder/xga/commitment").with_status(404).create();

        let result = send_commitment(
            &Client::new(),
            &format!("{}/eth/v1/builder/xga/commitment", server.url()),
            &signed,
        )
        .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));

        // Test 429 Rate Limited
        let mut server = mockito::Server::new_async().await;
        let _m429 = server.mock("POST", "/eth/v1/builder/xga/commitment").with_status(429).create();

        let result = send_commitment(
            &Client::new(),
            &format!("{}/eth/v1/builder/xga/commitment", server.url()),
            &signed,
        )
        .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Rate limited"));
    }

    #[tokio::test]
    async fn test_check_xga_support_methods() {
        let http_client_factory = HttpClientFactory::new();

        // Test capabilities endpoint success
        let mut server = mockito::Server::new_async().await;
        let _m_cap = server
            .mock("GET", "/eth/v1/builder/xga/capabilities")
            .with_status(200)
            .with_body(r#"{"supported": true}"#)
            .create();

        let supported = check_xga_support(&server.url(), &http_client_factory).await;
        assert!(supported);

        // Test OPTIONS method with Allow header
        let mut server = mockito::Server::new_async().await;
        let _m_opt = server
            .mock("OPTIONS", "/eth/v1/builder/xga/commitment")
            .with_status(200)
            .with_header("Allow", "POST, OPTIONS")
            .create();

        let supported = check_xga_support(&server.url(), &http_client_factory).await;
        assert!(supported);

        // Test custom header detection
        let mut server = mockito::Server::new_async().await;
        let _m_custom = server
            .mock("OPTIONS", "/eth/v1/builder/xga/commitment")
            .with_status(200)
            .with_header("x-xga-supported", "true")
            .create();

        let supported = check_xga_support(&server.url(), &http_client_factory).await;
        assert!(supported);

        // Test HEAD request with 405 (method not allowed = endpoint exists)
        let mut server = mockito::Server::new_async().await;
        let _m_head =
            server.mock("HEAD", "/eth/v1/builder/xga/commitment").with_status(405).create();

        let supported = check_xga_support(&server.url(), &http_client_factory).await;
        assert!(supported);

        // Test HEAD request with 404 (not supported)
        let mut server = mockito::Server::new_async().await;
        let _m_404 =
            server.mock("HEAD", "/eth/v1/builder/xga/commitment").with_status(404).create();

        let supported = check_xga_support(&server.url(), &http_client_factory).await;
        assert!(!supported);
    }
}
