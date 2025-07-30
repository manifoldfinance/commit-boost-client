use commit_boost::prelude::*;
use mockito::ServerGuard;
use xga_commitment::{
    commitment::{SignedXGACommitment, XGACommitment, XGAParameters},
    config::RetryConfig,
    infrastructure::HttpClientFactory,
    relay::{check_xga_support, send_to_relay},
};

async fn setup_mock_server() -> (ServerGuard, String) {
    let server = mockito::Server::new_async().await;
    let url = server.url();
    (server, url)
}

#[tokio::test]
async fn test_check_xga_support_capabilities_endpoint() {
    let (mut server, url) = setup_mock_server().await;

    // Mock successful capabilities response
    let _m = server
        .mock("GET", "/eth/v1/builder/xga/capabilities")
        .with_status(200)
        .with_body(r#"{"xga_version": "1.0", "supported": true}"#)
        .create_async()
        .await;

    let http_client_factory = HttpClientFactory::new();
    assert!(check_xga_support(&url, &http_client_factory).await);
}

#[tokio::test]
async fn test_check_xga_support_options_method() {
    let (mut server, url) = setup_mock_server().await;

    // Capabilities endpoint returns 404
    let _m1 = server
        .mock("GET", "/eth/v1/builder/xga/capabilities")
        .with_status(404)
        .create_async()
        .await;

    // OPTIONS returns allowed methods
    let _m2 = server
        .mock("OPTIONS", "/eth/v1/builder/xga/commitment")
        .with_status(200)
        .with_header("allow", "POST, GET, OPTIONS")
        .create_async()
        .await;

    let http_client_factory = HttpClientFactory::new();
    assert!(check_xga_support(&url, &http_client_factory).await);
}

#[tokio::test]
async fn test_check_xga_support_custom_header() {
    let (mut server, url) = setup_mock_server().await;

    // Capabilities endpoint returns 404
    let _m1 = server
        .mock("GET", "/eth/v1/builder/xga/capabilities")
        .with_status(404)
        .create_async()
        .await;

    // OPTIONS returns custom header
    let _m2 = server
        .mock("OPTIONS", "/eth/v1/builder/xga/commitment")
        .with_status(200)
        .with_header("x-xga-supported", "true")
        .create_async()
        .await;

    let http_client_factory = HttpClientFactory::new();
    assert!(check_xga_support(&url, &http_client_factory).await);
}

#[tokio::test]
async fn test_check_xga_support_head_request() {
    let (mut server, url) = setup_mock_server().await;

    // All previous methods fail
    let _m1 = server
        .mock("GET", "/eth/v1/builder/xga/capabilities")
        .with_status(404)
        .create_async()
        .await;

    let _m2 = server
        .mock("OPTIONS", "/eth/v1/builder/xga/commitment")
        .with_status(404)
        .create_async()
        .await;

    // HEAD returns 405 (method not allowed) - indicates endpoint exists
    let _m3 =
        server.mock("HEAD", "/eth/v1/builder/xga/commitment").with_status(405).create_async().await;

    let http_client_factory = HttpClientFactory::new();
    assert!(check_xga_support(&url, &http_client_factory).await);
}

#[tokio::test]
async fn test_check_xga_support_not_supported() {
    let (mut server, url) = setup_mock_server().await;

    // All endpoints return 404
    let _m1 = server
        .mock("GET", "/eth/v1/builder/xga/capabilities")
        .with_status(404)
        .create_async()
        .await;

    let _m2 = server
        .mock("OPTIONS", "/eth/v1/builder/xga/commitment")
        .with_status(404)
        .create_async()
        .await;

    let _m3 =
        server.mock("HEAD", "/eth/v1/builder/xga/commitment").with_status(404).create_async().await;

    let http_client_factory = HttpClientFactory::new();
    assert!(!check_xga_support(&url, &http_client_factory).await);
}

#[tokio::test]
async fn test_send_commitment_success() {
    let (mut server, url) = setup_mock_server().await;

    // Mock successful commitment submission
    let _m = server
        .mock("POST", "/eth/v1/builder/xga/commitment")
        .with_status(200)
        .with_body(r#"{"success": true, "commitment_id": "test-123"}"#)
        .create_async()
        .await;

    let commitment = XGACommitment::new(
        [1u8; 32],
        BlsPublicKey::default(),
        "test-relay".to_string(),
        1,
        XGAParameters::default(),
    );

    let signed = SignedXGACommitment { message: commitment, signature: BlsSignature::default() };

    let http_client_factory = HttpClientFactory::new();
    let retry_config = RetryConfig {
        max_retries: 1,
        initial_backoff_ms: 100,
        max_backoff_secs: 5,
    };
    let result = send_to_relay(signed, url, &retry_config, &http_client_factory).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_send_commitment_retry_logic() {
    let (mut server, url) = setup_mock_server().await;

    // First attempt fails
    let _m1 = server
        .mock("POST", "/eth/v1/builder/xga/commitment")
        .with_status(500)
        .expect(1)
        .create_async()
        .await;

    // Second attempt succeeds
    let _m2 = server
        .mock("POST", "/eth/v1/builder/xga/commitment")
        .with_status(200)
        .with_body(r#"{"success": true}"#)
        .expect(1)
        .create_async()
        .await;

    let commitment = XGACommitment::new(
        [1u8; 32],
        BlsPublicKey::default(),
        "test-relay".to_string(),
        1,
        XGAParameters::default(),
    );

    let signed = SignedXGACommitment { message: commitment, signature: BlsSignature::default() };

    let http_client_factory = HttpClientFactory::new();
    let retry_config = RetryConfig {
        max_retries: 3,
        initial_backoff_ms: 10,
        max_backoff_secs: 5,
    };
    let result = send_to_relay(signed, url, &retry_config, &http_client_factory).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_send_commitment_all_retries_fail() {
    let (mut server, url) = setup_mock_server().await;

    // All attempts fail
    let _m = server
        .mock("POST", "/eth/v1/builder/xga/commitment")
        .with_status(500)
        .with_body("Internal Server Error")
        .expect(3)
        .create_async()
        .await;

    let commitment = XGACommitment::new(
        [1u8; 32],
        BlsPublicKey::default(),
        "test-relay".to_string(),
        1,
        XGAParameters::default(),
    );

    let signed = SignedXGACommitment { message: commitment, signature: BlsSignature::default() };

    let http_client_factory = HttpClientFactory::new();
    let retry_config = RetryConfig {
        max_retries: 3,
        initial_backoff_ms: 10,
        max_backoff_secs: 5,
    };
    let result = send_to_relay(signed, url, &retry_config, &http_client_factory).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_check_xga_support_network_errors() {
    let http_client_factory = HttpClientFactory::new();

    // Test with invalid URL
    assert!(!check_xga_support("not-a-valid-url", &http_client_factory).await);

    // Test with URL that points to non-existent server
    assert!(!check_xga_support("http://localhost:99999", &http_client_factory).await);

    // Test with empty URL
    assert!(!check_xga_support("", &http_client_factory).await);
}

#[tokio::test]
async fn test_send_commitment_edge_cases() {
    let (mut server, url) = setup_mock_server().await;

    // Test with server error response
    let _m = server
        .mock("POST", "/eth/v1/builder/xga/commitment")
        .with_status(500)
        .with_body("Internal Server Error")
        .create_async()
        .await;

    let commitment = XGACommitment::new(
        [1u8; 32],
        BlsPublicKey::default(),
        "test-relay".to_string(),
        1,
        XGAParameters::default(),
    );

    let signed = SignedXGACommitment { message: commitment, signature: BlsSignature::default() };

    // Server error response should be handled as an error
    let http_client_factory = HttpClientFactory::new();
    let retry_config = RetryConfig {
        max_retries: 1,
        initial_backoff_ms: 100,
        max_backoff_secs: 5,
    };
    let result = send_to_relay(signed, url, &retry_config, &http_client_factory).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_send_commitment_timeout() {
    // Use a URL that will cause connection to fail/timeout
    let url = "https://localhost:9999".to_string(); // Non-existent server

    let commitment = XGACommitment::new(
        [1u8; 32],
        BlsPublicKey::default(),
        "test-relay".to_string(),
        1,
        XGAParameters::default(),
    );

    let signed = SignedXGACommitment { message: commitment, signature: BlsSignature::default() };

    // Connection should fail/timeout
    let http_client_factory = HttpClientFactory::new();
    let retry_config = RetryConfig {
        max_retries: 1,
        initial_backoff_ms: 50,
        max_backoff_secs: 5,
    };
    let result = send_to_relay(signed, url, &retry_config, &http_client_factory).await;
    assert!(result.is_err());
}
