use std::time::Duration;

use reqwest::{Client, StatusCode};
use serde::Deserialize;
use tracing::{debug, error, info, warn};

use crate::{
    commitment::SignedXgaCommitment,
    config::RetryConfig,
    infrastructure::HttpClientFactory,
    retry::{execute_with_retry, relay_send_retry_strategy},
};

/// Response from XGA relay
#[derive(Debug, Deserialize)]
struct XgaRelayResponse {
    success: bool,
    message: Option<String>,
    commitment_id: Option<String>,
}

/// Sends a signed XGA commitment to a relay.
///
/// This function handles the HTTP communication with the relay, including
/// automatic retries based on the configured retry policy. It constructs
/// the proper XGA endpoint URL and handles various response scenarios.
///
/// # Arguments
///
/// * `signed_commitment` - The signed XGA commitment to send
/// * `relay_url` - Base URL of the relay (e.g., "https://relay.example.com")
/// * `retry_config` - Configuration for retry behavior
/// * `http_client_factory` - Factory for creating HTTP clients
///
/// # Errors
///
/// Returns an error if:
/// - HTTP client creation fails
/// - All retry attempts fail
/// - Relay returns an error response
/// - Relay response indicates the commitment was rejected
///
/// # Retry Behavior
///
/// The function will retry on:
/// - Network errors
/// - 5xx server errors
/// - Timeouts
///
/// It will NOT retry on:
/// - 4xx client errors (except 429)
/// - Successful responses with `success: false`
///
/// # Example
///
/// ```no_run
/// # use xga_commitment::relay::send_to_relay;
/// # use xga_commitment::commitment::SignedXgaCommitment;
/// # use xga_commitment::config::RetryConfig;
/// # use xga_commitment::infrastructure::HttpClientFactory;
/// # async fn example() -> eyre::Result<()> {
/// # let signed_commitment: SignedXgaCommitment = todo!();
/// # let http_client_factory = HttpClientFactory::new();
/// # let retry_config = RetryConfig::default();
/// send_to_relay(
///     signed_commitment,
///     "https://relay.example.com",
///     &retry_config,
///     &http_client_factory,
/// ).await?;
/// # Ok(())
/// # }
/// ```
pub async fn send_to_relay(
    signed_commitment: SignedXgaCommitment,
    relay_url: &str,
    retry_config: &RetryConfig,
    http_client_factory: &HttpClientFactory,
) -> eyre::Result<()> {
    // Create client using factory
    let client = http_client_factory.create_client()?;

    // Construct XGA endpoint URL
    let xga_endpoint = format!("{}/eth/v1/builder/xga/commitment", relay_url.trim_end_matches('/'));

    let strategy = relay_send_retry_strategy(retry_config);

    let result = execute_with_retry(strategy, || {
        let client = &client;
        let xga_endpoint = &xga_endpoint;
        let signed_commitment = &signed_commitment;
        
        async move {
            use tokio_retry2::RetryError;
            
            debug!(
                validator_pubkey = ?signed_commitment.message.validator_pubkey,
                relay_id = ?signed_commitment.message.relay_id,
                "Sending XGA commitment to relay"
            );

            match send_commitment(client, xga_endpoint, signed_commitment).await {
                Ok(()) => {
                    info!(
                        validator_pubkey = ?signed_commitment.message.validator_pubkey,
                        relay_id = ?signed_commitment.message.relay_id,
                        "Successfully sent XGA commitment to relay"
                    );
                    Ok(())
                }
                Err(e) => {
                    warn!(
                        validator_pubkey = ?signed_commitment.message.validator_pubkey,
                        relay_id = ?signed_commitment.message.relay_id,
                        error = %e,
                        "Failed to send XGA commitment"
                    );
                    // Always retry on send failures
                    Err(RetryError::transient(e))
                }
            }
        }
    })
    .await;

    match result {
        Ok(()) => Ok(()),
        Err(e) => {
            error!(
                validator_pubkey = ?signed_commitment.message.validator_pubkey,
                relay_id = ?signed_commitment.message.relay_id,
                error = %e,
                "Failed to send XGA commitment after all retries"
            );
            Err(e)
        }
    }
}

/// Send a single commitment attempt
async fn send_commitment(
    client: &Client,
    endpoint: &str,
    signed_commitment: &SignedXgaCommitment,
) -> eyre::Result<()> {
    let response = client.post(endpoint).json(signed_commitment).send().await?;

    let status = response.status();

    match status {
        StatusCode::OK | StatusCode::CREATED | StatusCode::ACCEPTED => {
            // Try to parse response
            match response.json::<XgaRelayResponse>().await {
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

/// Checks if a relay supports XGA commitments.
///
/// This function uses multiple approaches to detect XGA support:
/// 1. Checks for a dedicated XGA capabilities endpoint
/// 2. Sends an OPTIONS request to the commitment endpoint
/// 3. Checks for XGA-specific headers in responses
///
/// The function is designed to be resilient and will not fail if the
/// relay is temporarily unavailable. It uses a short timeout (5 seconds)
/// to avoid blocking for too long.
///
/// # Arguments
///
/// * `relay_url` - Base URL of the relay to check
/// * `http_client_factory` - Factory for creating HTTP clients
///
/// # Returns
///
/// Returns `true` if the relay appears to support XGA, `false` otherwise.
/// This includes cases where the relay is unreachable or returns errors.
///
/// # Example
///
/// ```no_run
/// # use xga_commitment::relay::check_xga_support;
/// # use xga_commitment::infrastructure::HttpClientFactory;
/// # async fn example() -> eyre::Result<()> {
/// let http_client_factory = HttpClientFactory::new();
/// let supports_xga = check_xga_support(
///     "https://relay.example.com",
///     &http_client_factory,
/// ).await;
/// 
/// if supports_xga {
///     println!("Relay supports XGA commitments");
/// } else {
///     println!("Relay does not support XGA or is unavailable");
/// }
/// # Ok(())
/// # }
/// ```
pub async fn check_xga_support(relay_url: &str, http_client_factory: &HttpClientFactory) -> bool {
    let client = match http_client_factory.create_client_with_timeout(Duration::from_secs(5)) {
        Ok(c) => c,
        Err(e) => {
            error!("Failed to create HTTP client: {}", e);
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
                        return true;
                    }
                }
            }

            // Also check for custom XGA headers
            if response.headers().contains_key("x-xga-supported") {
                info!(relay_url = relay_url, "Relay advertises XGA support via custom header");
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
                    false
                }
                StatusCode::METHOD_NOT_ALLOWED => {
                    // This actually suggests the endpoint exists but doesn't support HEAD
                    info!(
                        relay_url = relay_url,
                        "XGA endpoint exists (HEAD not allowed) - assuming XGA support"
                    );
                    true
                }
                _ => {
                    info!(
                        relay_url = relay_url,
                        status = ?response.status(),
                        "XGA endpoint responded - assuming XGA support"
                    );
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
        commitment::{XgaCommitment, XgaParameters},
        infrastructure::HttpClientFactory,
    };

    #[tokio::test]
    async fn test_send_commitment_error_handling() {
        let commitment = XgaCommitment::new(
            [0x42u8; 32],
            BlsPublicKey::from([0x01u8; 48]),
            "test-relay",
            1,
            XgaParameters::default(),
        );

        let signed = SignedXgaCommitment {
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
