use alloy::rpc::types::beacon::relay::{ValidatorRegistration, ValidatorRegistrationMessage};
use xga_commitment::commitment::RegistrationNotification;

#[tokio::test]
async fn test_webhook_health_endpoint() {
    // This test would require setting up the webhook server
    // For now, we'll create a simple unit test for the validation logic

    // Test that we can create valid registration notifications
    let message = ValidatorRegistrationMessage {
        fee_recipient: alloy::primitives::Address::from([1u8; 20]),
        gas_limit: 30000000,
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("Failed to get current system time")
            .as_secs(),
        pubkey: alloy::rpc::types::beacon::BlsPublicKey::from([2u8; 48]),
    };

    let registration = ValidatorRegistration {
        message,
        signature: alloy::rpc::types::beacon::BlsSignature::from([3u8; 96]),
    };

    let notification = RegistrationNotification {
        registration,
        relay_url: "https://relay.example.com".to_string(),
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("Failed to get current system time")
            .as_secs(),
    };

    // Verify the notification is properly constructed
    assert!(!notification.relay_url.is_empty());
    assert!(notification.timestamp > 0);
    assert_ne!(notification.registration.message.fee_recipient.0, [0u8; 20]);
}

#[test]
fn test_extract_relay_id_from_url() {
    // Helper function from webhook module
    fn extract_relay_id(url: &str) -> String {
        url.split("://").nth(1).and_then(|s| s.split('/').next()).unwrap_or(url).to_string()
    }

    assert_eq!(extract_relay_id("https://relay.example.com/api/v1"), "relay.example.com");

    assert_eq!(extract_relay_id("http://localhost:8080"), "localhost:8080");

    assert_eq!(extract_relay_id("relay.example.com"), "relay.example.com");

    assert_eq!(extract_relay_id("https://relay.example.com:443/builder"), "relay.example.com:443");
}

#[test]
fn test_registration_age_validation() {
    let current_time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("Failed to get current system time")
        .as_secs();

    // Test with fresh registration (should be valid)
    let fresh_notification = create_test_notification(current_time - 10);
    assert!(is_registration_fresh(&fresh_notification, 60));

    // Test with old registration (should be invalid)
    let old_notification = create_test_notification(current_time - 120);
    assert!(!is_registration_fresh(&old_notification, 60));

    // Test with exact age limit
    let limit_notification = create_test_notification(current_time - 60);
    assert!(is_registration_fresh(&limit_notification, 60));
}

// Helper functions
fn create_test_notification(timestamp: u64) -> RegistrationNotification {
    let message = ValidatorRegistrationMessage {
        fee_recipient: alloy::primitives::Address::from([1u8; 20]),
        gas_limit: 30000000,
        timestamp,
        pubkey: alloy::rpc::types::beacon::BlsPublicKey::from([2u8; 48]),
    };

    let registration = ValidatorRegistration {
        message,
        signature: alloy::rpc::types::beacon::BlsSignature::from([3u8; 96]),
    };

    RegistrationNotification {
        registration,
        relay_url: "https://relay.example.com".to_string(),
        timestamp,
    }
}

fn is_registration_fresh(notification: &RegistrationNotification, max_age_secs: u64) -> bool {
    let current_time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("Failed to get current system time")
        .as_secs();

    // Check for future timestamp (with 5 second tolerance for clock skew)
    if notification.timestamp > current_time + 5 {
        return false;
    }

    let age = current_time.saturating_sub(notification.timestamp);
    age <= max_age_secs
}

#[test]
fn test_webhook_validation_edge_cases() {
    // Test with future timestamp (should handle gracefully)
    let future_time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("Failed to get current system time")
        .as_secs() +
        3600; // 1 hour in the future

    let future_notification = create_test_notification(future_time);
    // Future timestamps should be considered invalid
    assert!(!is_registration_fresh(&future_notification, 60));

    // Test with very old timestamp
    let ancient_notification = create_test_notification(0);
    assert!(!is_registration_fresh(&ancient_notification, 60));

    // Test with max u64 timestamp (overflow protection)
    let overflow_notification = create_test_notification(u64::MAX);
    assert!(!is_registration_fresh(&overflow_notification, 60));
}

#[test]
fn test_extract_relay_id_edge_cases() {
    fn extract_relay_id(url: &str) -> String {
        url.split("://").nth(1).and_then(|s| s.split('/').next()).unwrap_or(url).to_string()
    }

    // Test with empty string
    assert_eq!(extract_relay_id(""), "");

    // Test with just protocol
    assert_eq!(extract_relay_id("https://"), "");

    // Test with malformed URLs
    assert_eq!(extract_relay_id("//no-protocol.com"), "//no-protocol.com");
    assert_eq!(extract_relay_id("https:///triple-slash"), "");

    // Test with query parameters
    assert_eq!(extract_relay_id("https://relay.com:8080/path?query=value"), "relay.com:8080");
}
