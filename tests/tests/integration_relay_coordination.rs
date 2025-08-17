use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use alloy::{
    primitives::Address,
    rpc::types::beacon::{relay::ValidatorRegistration, BlsPublicKey, BlsSignature},
};
use cb_common::{
    config::RelayConfig,
    pbs::{RelayClient, RelayEntry},
    signer::random_secret,
    types::Chain,
    utils::blst_pubkey_to_alloy,
};
use cb_pbs::{DefaultBuilderApi, PbsService, PbsState};
use cb_tests::{
    mock_relay::{start_mock_relay_service, MockRelayState},
    mock_validator::MockValidator,
    utils::{
        generate_mock_relay, generate_mock_relay_with_batch_size, get_local_address, get_pbs_config, setup_test_env,
        to_pbs_config,
    },
};
use eyre::Result;
use futures::future::{join_all, try_join_all};
use reqwest::StatusCode;
use tokio::time::sleep;
use tracing::info;

// Helper function to create validator registrations
fn create_validator_registrations(count: usize) -> Vec<ValidatorRegistration> {
    (0..count)
        .map(|i| {
            let mut pubkey_bytes = [0u8; 48];
            pubkey_bytes[0] = (i & 0xFF) as u8;
            pubkey_bytes[1] = ((i >> 8) & 0xFF) as u8;
            
            ValidatorRegistration {
                message: alloy::rpc::types::beacon::relay::ValidatorRegistrationMessage {
                    fee_recipient: Address::from([i as u8; 20]),
                    gas_limit: 30_000_000,
                    timestamp: 1234567890,
                    pubkey: BlsPublicKey::from(pubkey_bytes),
                },
                signature: BlsSignature::from([0u8; 96]),
            }
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_batch_registration_varying_sizes() -> Result<()> {
    setup_test_env();
    let pbs_port = 5000;

    info!("Testing batch registration with varying sizes");

    // Test with multiple batch sizes
    let batch_configs = vec![
        (10, 5),    // 10 validators, batch size 5 (2 batches)
        (100, 25),  // 100 validators, batch size 25 (4 batches)
        (1000, 100), // 1000 validators, batch size 100 (10 batches)
    ];

    for (validator_count, batch_size) in batch_configs {
        info!("Testing {} validators with batch size {}", validator_count, batch_size);
        
        // Create relay with specific batch size
        let signer = random_secret();
        let pubkey = blst_pubkey_to_alloy(&signer.sk_to_pk());
        let relay = generate_mock_relay_with_batch_size(pbs_port + 1, pubkey, batch_size)?;
        
        let mock_state = Arc::new(MockRelayState::new(Chain::Holesky, signer));
        tokio::spawn(start_mock_relay_service(mock_state.clone(), pbs_port + 1));

        let config = to_pbs_config(Chain::Holesky, get_pbs_config(pbs_port), vec![relay]);
        let state = PbsState::new(config);
        tokio::spawn(PbsService::run::<(), DefaultBuilderApi>(state));

        sleep(Duration::from_millis(100)).await;

        // Create MockValidator with unique port to avoid conflicts
        let mock_validator = MockValidator::new(pbs_port + validator_count as u16)?;
        
        // Create and send registrations
        let registrations = create_validator_registrations(validator_count);
        let start_time = Instant::now();
        let res = mock_validator.do_register_custom_validators(registrations).await?;
        let elapsed = start_time.elapsed();

        assert_eq!(res.status(), StatusCode::OK);
        
        // Verify relay received the expected number of batch calls
        let calls = mock_state.received_register_validator();
        let expected_calls = (validator_count + batch_size - 1) / batch_size; // ceiling division
        assert!(
            calls >= expected_calls as u64,
            "Expected at least {} calls for {} validators with batch size {}, got {}",
            expected_calls, validator_count, batch_size, calls
        );

        info!(
            "Batch test completed: {} validators, batch size {}, {} calls, {:?} elapsed",
            validator_count, batch_size, calls, elapsed
        );
    }

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_parallel_relay_submissions_select_ok() -> Result<()> {
    setup_test_env();
    let pbs_port = 5100;

    info!("Testing parallel relay submissions with select_ok pattern");

    // Create multiple relays
    let mut relays = vec![];
    let mut mock_states = vec![];
    
    for i in 0..3 {
        let signer = random_secret();
        let pubkey = blst_pubkey_to_alloy(&signer.sk_to_pk());
        let relay = generate_mock_relay(pbs_port + i + 1, pubkey)?;
        relays.push(relay);
        
        let mock_state = Arc::new(MockRelayState::new(Chain::Holesky, signer));
        mock_states.push(mock_state.clone());
        tokio::spawn(start_mock_relay_service(mock_state, pbs_port + i + 1));
    }

    // Configure PBS with wait_all_registrations = false for select_ok behavior
    let mut pbs_config = get_pbs_config(pbs_port);
    pbs_config.wait_all_registrations = false;
    
    let config = to_pbs_config(Chain::Holesky, pbs_config, relays);
    let state = PbsState::new(config);
    tokio::spawn(PbsService::run::<(), DefaultBuilderApi>(state));

    sleep(Duration::from_millis(100)).await;

    let mock_validator = MockValidator::new(pbs_port)?;
    let registrations = create_validator_registrations(50);

    let start_time = Instant::now();
    let res = mock_validator.do_register_custom_validators(registrations).await?;
    let elapsed = start_time.elapsed();

    assert_eq!(res.status(), StatusCode::OK);

    // With select_ok, we expect faster completion but still successful registration
    let successful_relays = mock_states.iter().filter(|s| s.received_register_validator() > 0).count();
    assert!(successful_relays >= 1, "At least one relay should have received registrations");

    info!(
        "Select OK test completed in {:?}, {} relays participated",
        elapsed, successful_relays
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_relay_failure_recovery_with_timeout() -> Result<()> {
    setup_test_env();
    let pbs_port = 5200;

    info!("Testing relay failure recovery with timeout and retry");

    let chain = Chain::Holesky;
    let mut relays = vec![];
    
    // Relay 1: Working normally
    let signer1 = random_secret();
    let pubkey1 = blst_pubkey_to_alloy(&signer1.sk_to_pk());
    relays.push(generate_mock_relay(pbs_port + 1, pubkey1)?);
    let mock_state1 = Arc::new(MockRelayState::new(chain, signer1));
    tokio::spawn(start_mock_relay_service(mock_state1.clone(), pbs_port + 1));
    
    // Relay 2: Will return 500 errors (simulated failure)
    let signer2 = random_secret();
    let pubkey2 = blst_pubkey_to_alloy(&signer2.sk_to_pk());
    relays.push(generate_mock_relay(pbs_port + 2, pubkey2)?);
    let mock_state2 = Arc::new(MockRelayState::new(chain, signer2));
    mock_state2.set_response_override(StatusCode::INTERNAL_SERVER_ERROR);
    tokio::spawn(start_mock_relay_service(mock_state2.clone(), pbs_port + 2));
    
    // Relay 3: Not started (will timeout)
    let signer3 = random_secret();
    let pubkey3 = blst_pubkey_to_alloy(&signer3.sk_to_pk());
    relays.push(generate_mock_relay(pbs_port + 3, pubkey3)?);
    // Don't start this relay to simulate timeout

    let mut pbs_config = get_pbs_config(pbs_port);
    pbs_config.timeout_register_validator_ms = 1000; // 1 second timeout
    pbs_config.register_validator_retry_limit = 3;
    
    let config = to_pbs_config(chain, pbs_config, relays);
    let state = PbsState::new(config);
    tokio::spawn(PbsService::run::<(), DefaultBuilderApi>(state));

    sleep(Duration::from_millis(100)).await;

    let mock_validator = MockValidator::new(pbs_port)?;
    let registrations = create_validator_registrations(10);

    let start_time = Instant::now();
    let res = mock_validator.do_register_custom_validators(registrations).await?;
    let elapsed = start_time.elapsed();

    // Should succeed because relay 1 works
    assert_eq!(res.status(), StatusCode::OK);

    // Verify that relay 1 received the registrations (working relay)
    assert!(mock_state1.received_register_validator() > 0, "Working relay should receive registrations");
    
    // Verify that relay 2 was attempted (even though it failed)
    assert!(mock_state2.received_register_validator() > 0, "Failed relay should have been attempted");

    info!(
        "Failure recovery test completed in {:?}, working relay handled {} requests",
        elapsed,
        mock_state1.received_register_validator()
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_timing_games_and_bid_boost() -> Result<()> {
    setup_test_env();
    let pbs_port = 5300;

    info!("Testing timing games and bid boost configurations");

    // Create relay with timing games enabled and bid boost
    let signer = random_secret();
    let pubkey = blst_pubkey_to_alloy(&signer.sk_to_pk());
    
    // Create a relay with timing games configuration
    let entry = RelayEntry { 
        id: format!("mock_{}", pbs_port + 1), 
        pubkey, 
        url: get_local_address(pbs_port + 1).parse()? 
    };
    let config = RelayConfig {
        entry,
        id: None,
        headers: None,
        get_params: None,
        enable_timing_games: true,
        target_first_request_ms: Some(100),
        frequency_get_header_ms: Some(50),
        validator_registration_batch_size: None,
        bid_boost: Some(1.05), // 5% boost
    };
    let relay = RelayClient::new(config)?;

    let mock_state = Arc::new(MockRelayState::new(Chain::Holesky, signer));
    tokio::spawn(start_mock_relay_service(mock_state.clone(), pbs_port + 1));

    let config = to_pbs_config(Chain::Holesky, get_pbs_config(pbs_port), vec![relay]);
    let state = PbsState::new(config);
    tokio::spawn(PbsService::run::<(), DefaultBuilderApi>(state));

    sleep(Duration::from_millis(100)).await;

    let mock_validator = MockValidator::new(pbs_port)?;

    // Test get_header request with timing games
    let start_time = Instant::now();
    let res = mock_validator.do_get_header(Some(pubkey)).await?;
    let elapsed = start_time.elapsed();

    // Should succeed and timing should reflect timing games configuration
    assert!(res.status() == StatusCode::OK || res.status() == StatusCode::NO_CONTENT);
    assert!(elapsed >= Duration::from_millis(50), "Timing games should introduce delay");

    info!(
        "Timing games test completed in {:?}, received {} get_header calls",
        elapsed,
        mock_state.received_get_header()
    );

    // Test validator registration works with timing configuration
    let registrations = create_validator_registrations(5);
    let res = mock_validator.do_register_custom_validators(registrations).await?;
    assert_eq!(res.status(), StatusCode::OK);

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_relay_failover_under_load() -> Result<()> {
    setup_test_env();
    let pbs_port = 5400;

    info!("Testing relay failover under load (100 req/sec)");

    // Setup multiple relays with different failure patterns
    let mut relays = vec![];
    let mut mock_states = vec![];
    
    for i in 0..3 {
        let signer = random_secret();
        let pubkey = blst_pubkey_to_alloy(&signer.sk_to_pk());
        let relay = generate_mock_relay(pbs_port + i + 1, pubkey)?;
        relays.push(relay);
        
        let mock_state = Arc::new(MockRelayState::new(Chain::Holesky, signer));
        
        // Configure failure patterns
        if i == 1 {
            // Relay 2 returns 503 (service unavailable)
            mock_state.set_response_override(StatusCode::SERVICE_UNAVAILABLE);
        } else if i == 2 {
            // Relay 3 returns 429 (rate limit)
            mock_state.set_response_override(StatusCode::TOO_MANY_REQUESTS);
        }
        
        mock_states.push(mock_state.clone());
        tokio::spawn(start_mock_relay_service(mock_state, pbs_port + i + 1));
    }

    let config = to_pbs_config(Chain::Holesky, get_pbs_config(pbs_port), relays);
    let state = PbsState::new(config);
    tokio::spawn(PbsService::run::<(), DefaultBuilderApi>(state));

    sleep(Duration::from_millis(100)).await;

    // Simulate 100 req/sec for 2 seconds
    let target_rps = 100;
    let duration_secs = 2;
    let total_requests = target_rps * duration_secs;
    let request_interval = Duration::from_millis(1000 / target_rps as u64);

    let mut handles = vec![];
    let start_time = Instant::now();

    for i in 0..total_requests {
        let port = pbs_port + 2000 + i as u16; // Use unique ports for each request
        let handle = tokio::spawn(async move {
            let validator = MockValidator::new(port)?;
            let registrations = create_validator_registrations(1);
            validator.do_register_custom_validators(registrations).await
        });
        handles.push(handle);
        
        if i < total_requests - 1 {
            sleep(request_interval).await;
        }
    }

    // Wait for all requests to complete
    let results: Vec<_> = join_all(handles).await;
    let elapsed = start_time.elapsed();

    // Count successful requests
    let successful = results.iter().filter(|r| {
        r.as_ref().ok()
            .and_then(|res| res.as_ref().ok())
            .map(|res| res.status() == StatusCode::OK)
            .unwrap_or(false)
    }).count();

    let success_rate = (successful as f64 / total_requests as f64) * 100.0;
    let actual_rps = total_requests as f64 / elapsed.as_secs_f64();

    info!(
        "Load test completed: {} requests in {:?}, {:.1} req/sec, {:.1}% success rate",
        total_requests, elapsed, actual_rps, success_rate
    );

    // Verify failover worked (should have high success rate despite some relays failing)
    assert!(success_rate >= 70.0, "Success rate should be at least 70% with failover");
    assert!(actual_rps >= 50.0, "Should achieve at least 50 req/sec");

    // Check relay distribution
    for (i, state) in mock_states.iter().enumerate() {
        let calls = state.received_register_validator();
        info!("Relay {} received {} calls", i + 1, calls);
    }

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_concurrent_relay_operations() -> Result<()> {
    setup_test_env();
    let pbs_port = 5500;

    info!("Testing concurrent relay operations");

    // Setup relays
    let mut relays = vec![];
    for i in 0..2 {
        let signer = random_secret();
        let pubkey = blst_pubkey_to_alloy(&signer.sk_to_pk());
        let relay = generate_mock_relay(pbs_port + i + 1, pubkey)?;
        relays.push(relay.clone());
        
        let mock_state = Arc::new(MockRelayState::new(Chain::Holesky, signer));
        tokio::spawn(start_mock_relay_service(mock_state, pbs_port + i + 1));
    }

    let config = to_pbs_config(Chain::Holesky, get_pbs_config(pbs_port), relays.clone());
    let state = PbsState::new(config);
    tokio::spawn(PbsService::run::<(), DefaultBuilderApi>(state));

    sleep(Duration::from_millis(100)).await;

    // Run concurrent operations: get_header and register_validator
    let mut handles = vec![];

    // 10 get_header requests
    for i in 0..10 {
        let port = pbs_port + 3000 + i as u16;
        let pubkey = relays[i % 2].pubkey();
        handles.push(tokio::spawn(async move {
            let validator = MockValidator::new(port)?;
            validator.do_get_header(Some(pubkey)).await
        }));
    }

    // 10 register_validator requests
    for i in 0..10 {
        let port = pbs_port + 3100 + i as u16;
        handles.push(tokio::spawn(async move {
            let validator = MockValidator::new(port)?;
            let registrations = create_validator_registrations(5);
            validator.do_register_custom_validators(registrations).await
        }));
    }

    let start_time = Instant::now();
    let results = try_join_all(handles).await?;
    let elapsed = start_time.elapsed();

    // Verify all operations completed successfully
    let get_header_responses = &results[0..10];
    let register_responses = &results[10..20];

    for response in get_header_responses {
        match response {
            Ok(res) => {
                assert!(
                    res.status() == StatusCode::OK || res.status() == StatusCode::NO_CONTENT,
                    "Get header should return 200 or 204"
                );
            }
            Err(e) => {
                panic!("Get header request failed: {}", e);
            }
        }
    }

    for response in register_responses {
        match response {
            Ok(res) => {
                assert_eq!(res.status(), StatusCode::OK, "Register validator should return 200");
            }
            Err(e) => {
                panic!("Register validator request failed: {}", e);
            }
        }
    }

    info!("Concurrent operations test completed in {:?}", elapsed);
    assert!(elapsed < Duration::from_secs(5), "Concurrent operations should complete quickly");

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_relay_coordination_performance_measurements() -> Result<()> {
    setup_test_env();
    let pbs_port = 5600;

    info!("Testing relay coordination performance measurements");
    
    // Setup multiple relays with different batch sizes
    let batch_sizes = vec![10, 20, 50];
    let mut relays = vec![];
    let mut mock_states = vec![];
    
    for (i, batch_size) in batch_sizes.iter().enumerate() {
        let signer = random_secret();
        let pubkey = blst_pubkey_to_alloy(&signer.sk_to_pk());
        let relay = generate_mock_relay_with_batch_size(pbs_port + i as u16 + 1, pubkey, *batch_size)?;
        relays.push(relay);
        
        let mock_state = Arc::new(MockRelayState::new(Chain::Holesky, signer));
        mock_states.push(mock_state.clone());
        tokio::spawn(start_mock_relay_service(mock_state, pbs_port + i as u16 + 1));
    }

    let config = to_pbs_config(Chain::Holesky, get_pbs_config(pbs_port), relays);
    let state = PbsState::new(config);
    tokio::spawn(PbsService::run::<(), DefaultBuilderApi>(state));

    sleep(Duration::from_millis(100)).await;

    let mock_validator = MockValidator::new(pbs_port)?;

    // Create batch for registration
    let registrations = create_validator_registrations(200);
    let res = mock_validator.do_register_custom_validators(registrations.clone()).await?;
    assert_eq!(res.status(), StatusCode::OK);

    // Performance measurement 1: Sequential batching
    let start_seq = Instant::now();
    for batch in registrations.chunks(20) {
        let _ = mock_validator.do_register_custom_validators(batch.to_vec()).await?;
    }
    let seq_duration = start_seq.elapsed();

    // Performance measurement 2: Concurrent batching (using new validators for each)
    let start_concurrent = Instant::now();
    let mut handles = vec![];
    for (i, batch) in registrations.chunks(20).enumerate() {
        let batch = batch.to_vec();
        let port = pbs_port + 1000 + i as u16; // Use unique ports
        handles.push(tokio::spawn(async move {
            let validator = MockValidator::new(port)?;
            validator.do_register_custom_validators(batch).await
        }));
    }
    let _ = try_join_all(handles).await?;
    let concurrent_duration = start_concurrent.elapsed();

    // Verify performance improvements
    let speedup = seq_duration.as_secs_f64() / concurrent_duration.as_secs_f64();
    info!(
        "Performance test: Sequential {:?}, Concurrent {:?}, Speedup {:.2}x",
        seq_duration, concurrent_duration, speedup
    );

    assert!(speedup >= 1.5, "Concurrent batching should be at least 1.5x faster");

    // Check relay load distribution
    for (i, (state, batch_size)) in mock_states.iter().zip(batch_sizes.iter()).enumerate() {
        let calls = state.received_register_validator();
        info!("Relay {} (batch size {}): {} calls", i + 1, batch_size, calls);
        assert!(calls > 0, "Each relay should receive some calls");
    }

    Ok(())
}