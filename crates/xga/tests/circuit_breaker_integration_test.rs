use std::time::Duration;

use xga_commitment::infrastructure::{CircuitBreaker, ValidatorRateLimiter};

#[tokio::test]
async fn test_circuit_breaker_basic_functionality() {
    let cb = CircuitBreaker::new(3, Duration::from_millis(100));
    let relay_id = "test-relay";

    // Initially closed
    assert!(!cb.is_open(relay_id).await);

    // Record failures
    cb.record_failure(relay_id).await;
    cb.record_failure(relay_id).await;
    assert!(!cb.is_open(relay_id).await);

    // Third failure opens the circuit
    cb.record_failure(relay_id).await;
    assert!(cb.is_open(relay_id).await);

    // Success resets the circuit
    cb.record_success(relay_id).await;
    assert!(!cb.is_open(relay_id).await);
}

#[tokio::test]
async fn test_circuit_breaker_timeout_reset() {
    let cb = CircuitBreaker::new(1, Duration::from_millis(50));
    let relay_id = "timeout-test-relay";

    // Open the circuit
    cb.record_failure(relay_id).await;
    assert!(cb.is_open(relay_id).await);

    // Wait for reset timeout
    tokio::time::sleep(Duration::from_millis(60)).await;

    // Circuit should be closed after timeout
    assert!(!cb.is_open(relay_id).await);
}

#[tokio::test]
async fn test_circuit_breaker_multiple_relays() {
    let cb = CircuitBreaker::new(2, Duration::from_secs(5));
    
    let relay1 = "relay1";
    let relay2 = "relay2";
    let relay3 = "relay3";
    
    // Record failures for different relays
    cb.record_failure(relay1).await;
    cb.record_failure(relay1).await;
    cb.record_failure(relay2).await;
    
    // Only relay1 should be open (reached threshold)
    assert!(cb.is_open(relay1).await);
    assert!(!cb.is_open(relay2).await);
    assert!(!cb.is_open(relay3).await);
    
    // Record success for relay1
    cb.record_success(relay1).await;
    assert!(!cb.is_open(relay1).await);
    
    // Record more failures for relay2
    cb.record_failure(relay2).await;
    assert!(cb.is_open(relay2).await);
}

#[tokio::test]
async fn test_circuit_breaker_rapid_requests() {
    let cb = CircuitBreaker::new(5, Duration::from_secs(1));
    let relay_id = "rapid-relay";
    
    // Rapidly record failures
    for _ in 0..4 {
        cb.record_failure(relay_id).await;
        assert!(!cb.is_open(relay_id).await);
    }
    
    // Fifth failure opens circuit
    cb.record_failure(relay_id).await;
    assert!(cb.is_open(relay_id).await);
    
    // Multiple checks while open should remain consistent
    for _ in 0..10 {
        assert!(cb.is_open(relay_id).await);
    }
}

#[tokio::test]
async fn test_circuit_breaker_mixed_success_failure() {
    let cb = CircuitBreaker::new(3, Duration::from_secs(1));
    let relay_id = "mixed-relay";
    
    // Mix of successes and failures
    cb.record_failure(relay_id).await;
    cb.record_success(relay_id).await; // Resets counter
    cb.record_failure(relay_id).await;
    cb.record_failure(relay_id).await;
    
    // Should not be open yet (reset happened)
    assert!(!cb.is_open(relay_id).await);
    
    // One more failure opens it
    cb.record_failure(relay_id).await;
    assert!(cb.is_open(relay_id).await);
}

#[tokio::test]
async fn test_circuit_breaker_concurrent_operations() {
    let cb = CircuitBreaker::new(10, Duration::from_secs(5));
    let relay_id = "concurrent-relay";
    
    // Spawn multiple tasks recording failures concurrently
    let mut handles = vec![];
    for _ in 0..20 {
        let cb_clone = cb.clone();
        let relay_id = relay_id.to_string();
        handles.push(tokio::spawn(async move {
            cb_clone.record_failure(&relay_id).await;
        }));
    }
    
    // Wait for all tasks
    for handle in handles {
        handle.await.unwrap();
    }
    
    // Circuit should be open after 10+ failures
    assert!(cb.is_open(relay_id).await);
}

#[tokio::test]
async fn test_circuit_breaker_edge_cases() {
    // Zero threshold - actually opens immediately
    let cb = CircuitBreaker::new(0, Duration::from_secs(1));
    let relay_id = "zero-threshold";
    
    // With zero threshold, any failure opens the circuit
    cb.record_failure(relay_id).await;
    assert!(cb.is_open(relay_id).await);
    
    // Very high threshold
    let cb = CircuitBreaker::new(u32::MAX, Duration::from_secs(1));
    let relay_id = "high-threshold";
    
    for _ in 0..100 {
        cb.record_failure(relay_id).await;
    }
    assert!(!cb.is_open(relay_id).await);
}

#[tokio::test]
async fn test_circuit_breaker_with_rate_limiter_integration() {
    // Test that circuit breaker and rate limiter can work together
    let cb = CircuitBreaker::new(3, Duration::from_secs(5));
    let rl = ValidatorRateLimiter::new(5, Duration::from_secs(1));
    
    let relay_id = "integrated-relay";
    let validator = commit_boost::prelude::BlsPublicKey::from([1u8; 48]);
    
    let mut failures_recorded = 0;
    
    // Simulate requests that check both rate limit and circuit breaker
    for i in 0..10 {
        let rate_limited = !rl.check_rate_limit(&validator).await;
        let circuit_open = cb.is_open(relay_id).await;
        
        if !rate_limited && !circuit_open {
            // Simulate a relay failure
            cb.record_failure(relay_id).await;
            failures_recorded += 1;
        }
        
        // After 5 requests, rate limiter should block
        if i >= 5 {
            assert!(rate_limited, "Rate limit should be exceeded after 5 requests, but wasn't at request {}", i);
        }
        
        // After 3 failures recorded, circuit should be open on next check
        if failures_recorded >= 3 {
            // Check circuit state after recording failure
            let circuit_now_open = cb.is_open(relay_id).await;
            assert!(circuit_now_open, "Circuit should be open after {} failures", failures_recorded);
        }
    }
    
    // Verify final state
    assert!(cb.is_open(relay_id).await, "Circuit should be open at the end");
    assert_eq!(failures_recorded, 3, "Should have recorded exactly 3 failures before rate limiting kicked in");
}