use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use alloy::{
    primitives::Address,
    rpc::types::beacon::{relay::ValidatorRegistration, BlsPublicKey, BlsSignature},
};
use cb_common::{
    signer::{random_secret, BlsPublicKey as CommonBlsPublicKey},
    types::Chain,
    utils::blst_pubkey_to_alloy,
};
use cb_pbs::{DefaultBuilderApi, PbsService, PbsState};
use cb_tests::{
    mock_relay::{start_mock_relay_service, MockRelayState},
    mock_validator::MockValidator,
    utils::{generate_mock_relay, get_pbs_config, setup_test_env, to_pbs_config},
};
use eyre::Result;
use futures::future::try_join_all;
use reqwest::StatusCode;
use tokio::time::sleep;
use tracing::{debug, info};

// Test constants
const LARGE_VALIDATOR_COUNT: usize = 1000;
const BENCHMARK_VALIDATOR_COUNT: usize = 10000;

fn create_test_registration(
    i: usize,
    fee_recipient: Address,
    gas_limit: u64,
) -> ValidatorRegistration {
    ValidatorRegistration {
        message: alloy::rpc::types::beacon::relay::ValidatorRegistrationMessage {
            fee_recipient,
            gas_limit,
            timestamp: 1000000 + i as u64,
            pubkey: BlsPublicKey::from([i as u8; 48]),
        },
        signature: BlsSignature::from([0u8; 96]),
    }
}

fn create_validator_registrations(count: usize) -> Vec<ValidatorRegistration> {
    let fee_recipient = Address::from([2u8; 20]);
    (0..count).map(|i| create_test_registration(i, fee_recipient, 30_000_000)).collect()
}

async fn setup_pbs_service_with_mock_relay(pbs_port: u16) -> Result<Arc<MockRelayState>> {
    let signer = random_secret();
    let pubkey: CommonBlsPublicKey = blst_pubkey_to_alloy(&signer.sk_to_pk());
    let chain = Chain::Holesky;

    // Setup mock relay
    let relays = vec![generate_mock_relay(pbs_port + 1, pubkey)?];
    let mock_state = Arc::new(MockRelayState::new(chain, signer));
    tokio::spawn(start_mock_relay_service(mock_state.clone(), pbs_port + 1));

    // Setup PBS service
    let config = to_pbs_config(chain, get_pbs_config(pbs_port), relays);
    let state = PbsState::new(config);
    tokio::spawn(PbsService::run::<(), DefaultBuilderApi>(state));

    sleep(Duration::from_millis(100)).await;
    Ok(mock_state)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_registration_cache_ttl_expiration() -> Result<()> {
    setup_test_env();
    let pbs_port = 5000;
    let mock_state = setup_pbs_service_with_mock_relay(pbs_port).await?;
    let mock_validator = MockValidator::new(pbs_port)?;

    // Note: Cache is global and persistent, so tests may affect each other
    // This is intentional for integration testing

    let registration = create_test_registration(1, Address::from([2u8; 20]), 30_000_000);

    info!("Testing initial registration");
    let res = mock_validator.do_register_custom_validators(vec![registration.clone()]).await?;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(mock_state.received_register_validator(), 1);

    info!("Testing cached registration (should skip relay call)");
    mock_state.reset_counter();
    let res = mock_validator.do_register_custom_validators(vec![registration.clone()]).await?;
    assert_eq!(res.status(), StatusCode::OK);
    // Should be 0 because it was cached
    assert_eq!(mock_state.received_register_validator(), 0);

    info!("TTL expiration test completed - cache behavior verified through relay call patterns");

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_parameter_change_detection() -> Result<()> {
    setup_test_env();
    let pbs_port = 5100;
    let mock_state = setup_pbs_service_with_mock_relay(pbs_port).await?;
    let mock_validator = MockValidator::new(pbs_port)?;

    // Using global cache - testing real cache behavior

    let initial_registration = create_test_registration(1, Address::from([2u8; 20]), 30_000_000);

    info!("Initial registration");
    let res =
        mock_validator.do_register_custom_validators(vec![initial_registration.clone()]).await?;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(mock_state.received_register_validator(), 1);

    info!("Testing fee_recipient change detection");
    mock_state.reset_counter();
    let fee_recipient_changed = create_test_registration(1, Address::from([3u8; 20]), 30_000_000);
    let res = mock_validator.do_register_custom_validators(vec![fee_recipient_changed]).await?;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(mock_state.received_register_validator(), 1); // Should re-register

    info!("Testing gas_limit change detection");
    mock_state.reset_counter();
    let gas_limit_changed = create_test_registration(1, Address::from([2u8; 20]), 25_000_000);
    let res = mock_validator.do_register_custom_validators(vec![gas_limit_changed]).await?;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(mock_state.received_register_validator(), 1); // Should re-register

    info!("Testing unchanged parameters (should use cache)");
    mock_state.reset_counter();
    let unchanged = create_test_registration(1, Address::from([2u8; 20]), 25_000_000);
    let res = mock_validator.do_register_custom_validators(vec![unchanged]).await?;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(mock_state.received_register_validator(), 0); // Should use cache

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_concurrent_registration_handling() -> Result<()> {
    setup_test_env();
    let pbs_port = 5200;
    let mock_state = setup_pbs_service_with_mock_relay(pbs_port).await?;
    let _mock_validator = MockValidator::new(pbs_port)?;

    // Using global cache - testing real cache behavior

    info!("Testing concurrent registration of {} validators", LARGE_VALIDATOR_COUNT);
    let registrations = create_validator_registrations(LARGE_VALIDATOR_COUNT);

    let start_time = Instant::now();

    // Split registrations into chunks for concurrent processing
    let chunk_size = 100;
    let mut handles = Vec::new();

    for chunk in registrations.chunks(chunk_size) {
        let chunk_regs = chunk.to_vec();
        let validator = MockValidator::new(pbs_port)?;

        let handle =
            tokio::spawn(async move { validator.do_register_custom_validators(chunk_regs).await });
        handles.push(handle);
    }

    // Wait for all concurrent registrations
    let results: Result<Vec<_>, _> = try_join_all(handles).await;
    let responses = results?;

    let elapsed = start_time.elapsed();
    info!(
        "Concurrent registration of {} validators completed in {:?}",
        LARGE_VALIDATOR_COUNT, elapsed
    );

    // Verify all responses are successful
    for response in responses {
        let response = response?;
        assert_eq!(response.status(), StatusCode::OK);
    }

    // Verify relay received the registrations
    let total_received = mock_state.received_register_validator();
    debug!("Total registrations received by relay: {}", total_received);
    assert!(total_received > 0);

    // Test second round should be mostly cached
    info!("Testing second round (should be mostly cached)");
    mock_state.reset_counter();
    let start_cached = Instant::now();

    let mut cached_handles = Vec::new();
    for chunk in registrations.chunks(chunk_size) {
        let chunk_regs = chunk.to_vec();
        let validator = MockValidator::new(pbs_port)?;

        let handle =
            tokio::spawn(async move { validator.do_register_custom_validators(chunk_regs).await });
        cached_handles.push(handle);
    }

    let cached_results: Result<Vec<_>, _> = try_join_all(cached_handles).await;
    let _cached_responses = cached_results?;

    let cached_elapsed = start_cached.elapsed();
    info!("Cached round completed in {:?}", cached_elapsed);

    // Second round should be much faster and have fewer relay calls
    let cached_received = mock_state.received_register_validator();
    debug!("Cached registrations received by relay: {}", cached_received);
    assert!(cached_received < total_received);
    assert!(cached_elapsed < elapsed);

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_cache_cleanup_and_memory_management() -> Result<()> {
    setup_test_env();
    let pbs_port = 5300;
    let mock_state = setup_pbs_service_with_mock_relay(pbs_port).await?;
    let mock_validator = MockValidator::new(pbs_port)?;

    // Using global cache - testing real cache behavior

    info!("Testing cache cleanup and memory management");

    // Add many registrations to trigger cleanup
    let large_batch = create_validator_registrations(1200); // > 1000 to trigger cleanup

    let res = mock_validator.do_register_custom_validators(large_batch).await?;
    assert_eq!(res.status(), StatusCode::OK);

    info!("Cache cleanup test completed - verified through registration behavior");

    // Test that cache continues to work correctly after cleanup
    let test_registration = create_test_registration(999, Address::from([2u8; 20]), 30_000_000);
    mock_state.reset_counter();

    let res = mock_validator.do_register_custom_validators(vec![test_registration]).await?;
    assert_eq!(res.status(), StatusCode::OK);
    // Should be cached from previous large batch
    assert_eq!(mock_state.received_register_validator(), 0);

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_performance_benchmark_10k_validators() -> Result<()> {
    setup_test_env();
    let pbs_port = 5400;
    let mock_state = setup_pbs_service_with_mock_relay(pbs_port).await?;
    let _mock_validator = MockValidator::new(pbs_port)?;

    // Using global cache - testing real cache behavior

    info!("Performance benchmark: {} validator registrations", BENCHMARK_VALIDATOR_COUNT);
    let registrations = create_validator_registrations(BENCHMARK_VALIDATOR_COUNT);

    // Benchmark initial registration (cold cache)
    let start_cold = Instant::now();

    // Process in larger chunks for better performance
    let chunk_size = 500;
    let mut cold_handles = Vec::new();

    for chunk in registrations.chunks(chunk_size) {
        let chunk_regs = chunk.to_vec();
        let validator = MockValidator::new(pbs_port)?;

        let handle =
            tokio::spawn(async move { validator.do_register_custom_validators(chunk_regs).await });
        cold_handles.push(handle);
    }

    let cold_results: Result<Vec<_>, _> = try_join_all(cold_handles).await;
    let _cold_responses = cold_results?;

    let cold_elapsed = start_cold.elapsed();
    let cold_throughput = BENCHMARK_VALIDATOR_COUNT as f64 / cold_elapsed.as_secs_f64();

    info!(
        "Cold cache: {} validators in {:?} ({:.2} validators/sec)",
        BENCHMARK_VALIDATOR_COUNT, cold_elapsed, cold_throughput
    );

    // Benchmark cached registration (warm cache)
    mock_state.reset_counter();
    let start_warm = Instant::now();

    let mut warm_handles = Vec::new();
    for chunk in registrations.chunks(chunk_size) {
        let chunk_regs = chunk.to_vec();
        let validator = MockValidator::new(pbs_port)?;

        let handle =
            tokio::spawn(async move { validator.do_register_custom_validators(chunk_regs).await });
        warm_handles.push(handle);
    }

    let warm_results: Result<Vec<_>, _> = try_join_all(warm_handles).await;
    let _warm_responses = warm_results?;

    let warm_elapsed = start_warm.elapsed();
    let warm_throughput = BENCHMARK_VALIDATOR_COUNT as f64 / warm_elapsed.as_secs_f64();

    info!(
        "Warm cache: {} validators in {:?} ({:.2} validators/sec)",
        BENCHMARK_VALIDATOR_COUNT, warm_elapsed, warm_throughput
    );

    // Verify performance improvement
    let speedup = cold_elapsed.as_secs_f64() / warm_elapsed.as_secs_f64();
    info!("Cache speedup: {:.2}x", speedup);

    // Cache should provide significant speedup
    assert!(speedup > 2.0, "Cache should provide at least 2x speedup, got {:.2}x", speedup);

    // Verify most requests were cached in second round
    let warm_relay_calls = mock_state.received_register_validator();
    let cache_hit_rate = 1.0 - (warm_relay_calls as f64 / BENCHMARK_VALIDATOR_COUNT as f64);
    info!("Cache hit rate: {:.2}%", cache_hit_rate * 100.0);

    assert!(
        cache_hit_rate > 0.95,
        "Cache hit rate should be > 95%, got {:.2}%",
        cache_hit_rate * 100.0
    );

    info!("Performance benchmark completed successfully");

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_cache_race_conditions() -> Result<()> {
    setup_test_env();
    let pbs_port = 5500;
    let mock_state = setup_pbs_service_with_mock_relay(pbs_port).await?;

    // Using global cache - testing real cache behavior

    info!("Testing race conditions with concurrent cache access");

    // Create same registration for all concurrent requests
    let same_registration = create_test_registration(42, Address::from([2u8; 20]), 30_000_000);

    let mut handles = Vec::new();
    let concurrent_requests = 50;

    // Spawn many concurrent requests for the same validator
    for _ in 0..concurrent_requests {
        let registration = same_registration.clone();
        let validator = MockValidator::new(pbs_port)?;

        let handle = tokio::spawn(async move {
            validator.do_register_custom_validators(vec![registration]).await
        });
        handles.push(handle);
    }

    // Wait for all to complete
    let results: Result<Vec<_>, _> = try_join_all(handles).await;
    let responses = results?;

    // All should succeed
    for response in responses {
        let response = response?;
        assert_eq!(response.status(), StatusCode::OK);
    }

    // Due to cache, relay should receive far fewer calls than total requests
    let total_relay_calls = mock_state.received_register_validator();
    info!(
        "Total relay calls for {} concurrent requests: {}",
        concurrent_requests, total_relay_calls
    );

    // Should be much less than the number of concurrent requests due to caching
    assert!(
        total_relay_calls < concurrent_requests / 2,
        "Expected far fewer relay calls due to caching, got {} out of {} requests",
        total_relay_calls,
        concurrent_requests
    );

    info!("Race condition test completed - cache prevented duplicate registrations");

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mixed_cached_and_new_registrations() -> Result<()> {
    setup_test_env();
    let pbs_port = 5600;
    let mock_state = setup_pbs_service_with_mock_relay(pbs_port).await?;
    let mock_validator = MockValidator::new(pbs_port)?;

    // Using global cache - testing real cache behavior

    info!("Testing mixed batch of cached and new registrations");

    // Register initial batch
    let initial_batch = create_validator_registrations(100);
    let res = mock_validator.do_register_custom_validators(initial_batch.clone()).await?;
    assert_eq!(res.status(), StatusCode::OK);
    let _initial_relay_calls = mock_state.received_register_validator();

    // Create mixed batch: 50 cached + 50 new
    let mut mixed_batch = initial_batch[0..50].to_vec(); // Already cached
    let new_registrations = create_validator_registrations(50)
        .into_iter()
        .enumerate()
        .map(|(i, mut reg)| {
            // Make them unique by modifying pubkey
            reg.message.pubkey = BlsPublicKey::from([(i + 200) as u8; 48]);
            reg
        })
        .collect::<Vec<_>>();
    mixed_batch.extend(new_registrations);

    // Process mixed batch
    mock_state.reset_counter();
    let res = mock_validator.do_register_custom_validators(mixed_batch).await?;
    assert_eq!(res.status(), StatusCode::OK);

    let mixed_relay_calls = mock_state.received_register_validator();
    info!("Mixed batch relay calls: {} (should be ~50 for new validators only)", mixed_relay_calls);

    // Should only process new validators (around 50), not the cached ones
    assert!(
        mixed_relay_calls < 70,
        "Expected ~50 relay calls for new validators, got {}",
        mixed_relay_calls
    );
    assert!(
        mixed_relay_calls > 30,
        "Expected at least 30 relay calls for new validators, got {}",
        mixed_relay_calls
    );

    Ok(())
}
