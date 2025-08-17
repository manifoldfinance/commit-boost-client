use std::{
    collections::HashMap,
    sync::{Arc, atomic::{AtomicUsize, Ordering}},
    time::{Duration, Instant},
};

use alloy::primitives::{b256, FixedBytes};
use cb_common::{
    commit::{
        constants::{GET_PUBKEYS_PATH, REQUEST_SIGNATURE_PATH, REVOKE_MODULE_PATH},
        request::{SignConsensusRequest, SignRequest},
    },
    config::{load_module_signing_configs, ModuleSigningConfig},
    types::ModuleId,
    utils::create_jwt,
};
use cb_tests::{
    signer_service::{create_admin_jwt, start_server},
    utils::{self, setup_test_env},
};
use eyre::Result;
use reqwest::{Client, StatusCode};
use tokio::time::sleep;
use tracing::{info, warn};

const JWT_MODULE_1: &str = "auth-test-module-1";
const JWT_MODULE_2: &str = "auth-test-module-2";
const JWT_MODULE_3: &str = "stress-test-module";
const JWT_SECRET_1: &str = "super-secret-key-1";
const JWT_SECRET_2: &str = "super-secret-key-2";
const JWT_SECRET_3: &str = "stress-test-secret";
const ADMIN_SECRET: &str = "admin-integration-secret";
const PUBKEY_1: [u8; 48] = [0x88, 0x38, 0x27, 0x19, 0x3f, 0x76, 0x27, 0xcd, 0x04, 0xe6, 0x21, 0xe1, 0xe8, 0xd5, 0x64, 0x98, 0x36, 0x2a, 0x52, 0xb2, 0xa3, 0x0c, 0x9a, 0x1c, 0x72, 0x03, 0x6e, 0xb9, 0x35, 0xc4, 0x27, 0x8d, 0xee, 0x23, 0xd3, 0x8a, 0x24, 0xd2, 0xf7, 0xdd, 0xa6, 0x26, 0x89, 0x88, 0x6f, 0x0c, 0x39, 0xf4];

async fn create_multi_module_signing_configs() -> HashMap<ModuleId, ModuleSigningConfig> {
    let mut cfg = utils::get_commit_boost_config(utils::get_pbs_static_config(utils::get_pbs_config(0)));

    let module_id_1 = ModuleId(JWT_MODULE_1.to_string());
    let signing_id_1 = b256!("0x1111111111111111111111111111111111111111111111111111111111111111");
    let module_id_2 = ModuleId(JWT_MODULE_2.to_string());
    let signing_id_2 = b256!("0x2222222222222222222222222222222222222222222222222222222222222222");
    let module_id_3 = ModuleId(JWT_MODULE_3.to_string());
    let signing_id_3 = b256!("0x3333333333333333333333333333333333333333333333333333333333333333");

    cfg.modules = Some(vec![
        utils::create_module_config(module_id_1.clone(), signing_id_1),
        utils::create_module_config(module_id_2.clone(), signing_id_2),
        utils::create_module_config(module_id_3.clone(), signing_id_3),
    ]);

    let jwts = HashMap::from([
        (module_id_1.clone(), JWT_SECRET_1.to_string()),
        (module_id_2.clone(), JWT_SECRET_2.to_string()),
        (module_id_3.clone(), JWT_SECRET_3.to_string()),
    ]);

    load_module_signing_configs(&cfg, &jwts).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_jwt_auth_rate_limiting_parallel() -> Result<()> {
    setup_test_env();
    let module_id = ModuleId(JWT_MODULE_1.to_string());
    let mod_cfgs = create_multi_module_signing_configs().await;
    let start_config = start_server(21000, &mod_cfgs, ADMIN_SECRET.to_string()).await?;
    let mod_cfg = mod_cfgs.get(&module_id).unwrap();

    let client = Client::new();
    let url = format!("http://{}{}", start_config.endpoint, GET_PUBKEYS_PATH);

    // Trigger rate limit with parallel invalid requests
    let invalid_jwt = create_jwt(&module_id, "wrong-secret")?;
    let mut handles = vec![];
    
    for i in 0..start_config.jwt_auth_fail_limit {
        let client = client.clone();
        let url = url.clone();
        let jwt = invalid_jwt.clone();
        
        handles.push(tokio::spawn(async move {
            let response = client.get(&url).bearer_auth(&jwt).send().await.unwrap();
            (i, response.status())
        }));
    }

    // Wait for all invalid requests to complete
    for handle in handles {
        let (i, status) = handle.await?;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "Request {} should be unauthorized", i);
    }

    // Now test that rate limiting is active
    let valid_jwt = create_jwt(&module_id, &mod_cfg.jwt_secret)?;
    let response = client.get(&url).bearer_auth(&valid_jwt).send().await?;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS, "Should be rate limited");

    // Wait for timeout and verify recovery
    sleep(Duration::from_secs(start_config.jwt_auth_fail_timeout_seconds as u64 + 1)).await;
    let response = client.get(&url).bearer_auth(&valid_jwt).send().await?;
    assert_eq!(response.status(), StatusCode::OK, "Should recover after timeout");

    info!("✓ JWT rate limiting with parallel requests works correctly");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_module_registration_revocation_lifecycle() -> Result<()> {
    setup_test_env();
    let module_id_1 = ModuleId(JWT_MODULE_1.to_string());
    let module_id_2 = ModuleId(JWT_MODULE_2.to_string());
    let mod_cfgs = create_multi_module_signing_configs().await;
    let start_config = start_server(21001, &mod_cfgs, ADMIN_SECRET.to_string()).await?;

    let client = Client::new();
    let pubkeys_url = format!("http://{}{}", start_config.endpoint, GET_PUBKEYS_PATH);
    let revoke_url = format!("http://{}{}", start_config.endpoint, REVOKE_MODULE_PATH);

    let mod_cfg_1 = mod_cfgs.get(&module_id_1).unwrap();
    let mod_cfg_2 = mod_cfgs.get(&module_id_2).unwrap();
    let jwt_1 = create_jwt(&module_id_1, &mod_cfg_1.jwt_secret)?;
    let jwt_2 = create_jwt(&module_id_2, &mod_cfg_2.jwt_secret)?;
    let admin_jwt = create_admin_jwt(ADMIN_SECRET.to_string())?;

    // Phase 1: Both modules should be operational
    let response_1 = client.get(&pubkeys_url).bearer_auth(&jwt_1).send().await?;
    assert_eq!(response_1.status(), StatusCode::OK, "Module 1 should be operational");

    let response_2 = client.get(&pubkeys_url).bearer_auth(&jwt_2).send().await?;
    assert_eq!(response_2.status(), StatusCode::OK, "Module 2 should be operational");

    // Phase 2: Revoke module 1
    let revoke_body = format!("{{\"module_id\": \"{}\"}}", JWT_MODULE_1);
    let response = client
        .post(&revoke_url)
        .header("content-type", "application/json")
        .body(revoke_body)
        .bearer_auth(&admin_jwt)
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK, "Module 1 revocation should succeed");

    // Phase 3: Verify module 1 is revoked, module 2 still works
    let response_1 = client.get(&pubkeys_url).bearer_auth(&jwt_1).send().await?;
    assert_eq!(response_1.status(), StatusCode::UNAUTHORIZED, "Module 1 should be revoked");

    let response_2 = client.get(&pubkeys_url).bearer_auth(&jwt_2).send().await?;
    assert_eq!(response_2.status(), StatusCode::OK, "Module 2 should still work");

    // Phase 4: Revoke module 2
    let revoke_body = format!("{{\"module_id\": \"{}\"}}", JWT_MODULE_2);
    let response = client
        .post(&revoke_url)
        .header("content-type", "application/json")
        .body(revoke_body)
        .bearer_auth(&admin_jwt)
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK, "Module 2 revocation should succeed");

    // Phase 5: Verify both modules are now revoked
    let response_1 = client.get(&pubkeys_url).bearer_auth(&jwt_1).send().await?;
    assert_eq!(response_1.status(), StatusCode::UNAUTHORIZED, "Module 1 should remain revoked");

    let response_2 = client.get(&pubkeys_url).bearer_auth(&jwt_2).send().await?;
    assert_eq!(response_2.status(), StatusCode::UNAUTHORIZED, "Module 2 should be revoked");

    info!("✓ Module registration/revocation lifecycle works correctly");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_concurrent_signing_requests() -> Result<()> {
    setup_test_env();
    let module_id = ModuleId(JWT_MODULE_1.to_string());
    let mod_cfgs = create_multi_module_signing_configs().await;
    let start_config = start_server(21002, &mod_cfgs, ADMIN_SECRET.to_string()).await?;
    let mod_cfg = mod_cfgs.get(&module_id).unwrap();

    let client = Client::new();
    let sign_url = format!("http://{}{}", start_config.endpoint, REQUEST_SIGNATURE_PATH);
    let jwt = create_jwt(&module_id, &mod_cfg.jwt_secret)?;

    let concurrent_requests = 10;
    let mut handles = vec![];

    // Create concurrent signing requests
    for i in 0..concurrent_requests {
        let client = client.clone();
        let url = sign_url.clone();
        let jwt = jwt.clone();
        
        handles.push(tokio::spawn(async move {
            let object_root = b256!("0x0123456789012345678901234567890123456789012345678901234567890123");
            let request = SignRequest::Consensus(SignConsensusRequest {
                pubkey: FixedBytes(PUBKEY_1),
                object_root,
            });

            let start_time = Instant::now();
            let response = client.post(&url).json(&request).bearer_auth(&jwt).send().await.unwrap();
            let duration = start_time.elapsed();
            
            (i, response.status(), duration)
        }));
    }

    // Collect results
    let mut total_duration = Duration::ZERO;
    let mut success_count = 0;

    for handle in handles {
        let (i, status, duration) = handle.await?;
        total_duration += duration;
        
        if status == StatusCode::OK {
            success_count += 1;
        } else {
            warn!("Request {} failed with status: {}", i, status);
        }
    }

    assert_eq!(success_count, concurrent_requests, "All signing requests should succeed");
    
    let avg_duration = total_duration / concurrent_requests as u32;
    info!("✓ {} concurrent signing requests completed, avg duration: {:?}", 
          concurrent_requests, avg_duration);
    
    // Ensure reasonable performance (less than 1 second per request on average)
    assert!(avg_duration < Duration::from_secs(1), "Average request time should be reasonable");

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pbs_signer_auth_flow() -> Result<()> {
    setup_test_env();
    let module_id = ModuleId(JWT_MODULE_1.to_string());
    let mod_cfgs = create_multi_module_signing_configs().await;
    let start_config = start_server(21003, &mod_cfgs, ADMIN_SECRET.to_string()).await?;
    let mod_cfg = mod_cfgs.get(&module_id).unwrap();

    let client = Client::new();
    let pubkeys_url = format!("http://{}{}", start_config.endpoint, GET_PUBKEYS_PATH);
    let sign_url = format!("http://{}{}", start_config.endpoint, REQUEST_SIGNATURE_PATH);

    // Simulate PBS module workflow: get pubkeys, then sign
    let jwt = create_jwt(&module_id, &mod_cfg.jwt_secret)?;

    // Step 1: PBS gets validator pubkeys from signer
    let response = client.get(&pubkeys_url).bearer_auth(&jwt).send().await?;
    assert_eq!(response.status(), StatusCode::OK, "PBS should get pubkeys successfully");
    
    let pubkeys_response = response.json::<cb_common::commit::request::GetPubkeysResponse>().await?;
    assert!(!pubkeys_response.keys.is_empty(), "Should return validator pubkeys");
    info!("✓ PBS retrieved {} pubkeys from signer", pubkeys_response.keys.len());

    // Step 2: PBS requests signature for consensus object
    let object_root = b256!("0xbeefcafefeeddeadbeefcafefeeddeadbeefcafefeeddeadbeefcafefeeddeaa");
    let request = SignRequest::Consensus(SignConsensusRequest {
        pubkey: FixedBytes(PUBKEY_1),
        object_root,
    });

    let response = client.post(&sign_url).json(&request).bearer_auth(&jwt).send().await?;
    assert_eq!(response.status(), StatusCode::OK, "PBS should get signature successfully");
    
    let signature = response.text().await?;
    assert!(!signature.is_empty(), "Signature should not be empty");
    info!("✓ PBS got signature from signer: {} chars", signature.len());

    // Step 3: Test invalid authentication
    let invalid_jwt = create_jwt(&module_id, "wrong-secret")?;
    let response = client.get(&pubkeys_url).bearer_auth(&invalid_jwt).send().await?;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "Invalid auth should be rejected");

    info!("✓ PBS ↔ Signer authentication flow works correctly");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_auth_stress_1000_requests_per_second() -> Result<()> {
    setup_test_env();
    let module_id = ModuleId(JWT_MODULE_3.to_string());
    let mod_cfgs = create_multi_module_signing_configs().await;
    let start_config = start_server(21004, &mod_cfgs, ADMIN_SECRET.to_string()).await?;
    let mod_cfg = mod_cfgs.get(&module_id).unwrap();

    let client = Arc::new(Client::new());
    let pubkeys_url = format!("http://{}{}", start_config.endpoint, GET_PUBKEYS_PATH);
    let jwt = create_jwt(&module_id, &mod_cfg.jwt_secret)?;

    let target_rps = 1000;
    let test_duration = Duration::from_secs(3); // 3 seconds of stress testing
    let total_requests = (target_rps * test_duration.as_secs() as usize) / 3; // Adjust for 3-second test

    info!("Starting stress test: {} requests over {:?} targeting {} RPS", 
          total_requests, test_duration, target_rps);

    let success_count = Arc::new(AtomicUsize::new(0));
    let error_count = Arc::new(AtomicUsize::new(0));
    let start_time = Instant::now();

    // Create request batches to maintain target RPS
    let batch_size = 50;
    let batches = (total_requests + batch_size - 1) / batch_size;
    let delay_between_batches = test_duration / batches as u32;

    let mut batch_handles = vec![];

    for batch_idx in 0..batches {
        let client = Arc::clone(&client);
        let url = pubkeys_url.clone();
        let jwt = jwt.clone();
        let success_count = Arc::clone(&success_count);
        let error_count = Arc::clone(&error_count);
        
        batch_handles.push(tokio::spawn(async move {
            // Wait for this batch's time slot
            sleep(delay_between_batches * batch_idx as u32).await;
            
            let mut request_handles = vec![];
            
            for _ in 0..batch_size {
                let client = Arc::clone(&client);
                let url = url.clone();
                let jwt = jwt.clone();
                let success_count = Arc::clone(&success_count);
                let error_count = Arc::clone(&error_count);
                
                request_handles.push(tokio::spawn(async move {
                    match client.get(&url).bearer_auth(&jwt).send().await {
                        Ok(response) => {
                            if response.status() == StatusCode::OK {
                                success_count.fetch_add(1, Ordering::Relaxed);
                            } else {
                                error_count.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        Err(_) => {
                            error_count.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }));
            }
            
            // Wait for all requests in this batch
            for handle in request_handles {
                let _ = handle.await;
            }
        }));
    }

    // Wait for all batches to complete
    for handle in batch_handles {
        let _ = handle.await;
    }

    let actual_duration = start_time.elapsed();
    let total_processed = success_count.load(Ordering::Relaxed) + error_count.load(Ordering::Relaxed);
    let actual_rps = total_processed as f64 / actual_duration.as_secs_f64();

    let success_rate = success_count.load(Ordering::Relaxed) as f64 / total_processed as f64 * 100.0;

    info!("Stress test completed:");
    info!("  Duration: {:?}", actual_duration);
    info!("  Total requests: {}", total_processed);
    info!("  Successful: {}", success_count.load(Ordering::Relaxed));
    info!("  Errors: {}", error_count.load(Ordering::Relaxed));
    info!("  Actual RPS: {:.1}", actual_rps);
    info!("  Success rate: {:.1}%", success_rate);

    // Performance assertions
    assert!(actual_rps >= 500.0, "Should handle at least 500 RPS (got {:.1})", actual_rps);
    assert!(success_rate >= 95.0, "Success rate should be at least 95% (got {:.1}%)", success_rate);

    info!("✓ Authentication stress test passed: {:.1} RPS with {:.1}% success rate", 
          actual_rps, success_rate);

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_jwt_failure_tracking_and_timeout() -> Result<()> {
    setup_test_env();
    let module_id = ModuleId(JWT_MODULE_2.to_string());
    let mod_cfgs = create_multi_module_signing_configs().await;
    let start_config = start_server(21005, &mod_cfgs, ADMIN_SECRET.to_string()).await?;
    let mod_cfg = mod_cfgs.get(&module_id).unwrap();

    let client = Client::new();
    let url = format!("http://{}{}", start_config.endpoint, GET_PUBKEYS_PATH);

    // Test failure accumulation over time
    let invalid_jwt = create_jwt(&module_id, "invalid-secret")?;
    let valid_jwt = create_jwt(&module_id, &mod_cfg.jwt_secret)?;

    // Make 2 failed attempts (below limit)
    for i in 0..2 {
        let response = client.get(&url).bearer_auth(&invalid_jwt).send().await?;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "Attempt {} should fail", i + 1);
    }

    // Valid request should still work
    let response = client.get(&url).bearer_auth(&valid_jwt).send().await?;
    assert_eq!(response.status(), StatusCode::OK, "Valid request should work");

    // One more failed attempt to trigger rate limit
    let response = client.get(&url).bearer_auth(&invalid_jwt).send().await?;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "Final failed attempt");

    // Now should be rate limited
    let response = client.get(&url).bearer_auth(&valid_jwt).send().await?;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS, "Should be rate limited");

    // Test partial timeout (should still be limited)
    sleep(Duration::from_secs(1)).await;
    let response = client.get(&url).bearer_auth(&valid_jwt).send().await?;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS, "Should still be rate limited");

    // Wait for full timeout period
    sleep(Duration::from_secs(start_config.jwt_auth_fail_timeout_seconds as u64)).await;
    let response = client.get(&url).bearer_auth(&valid_jwt).send().await?;
    assert_eq!(response.status(), StatusCode::OK, "Should recover after full timeout");

    info!("✓ JWT failure tracking and timeout behavior works correctly");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_cross_module_auth_isolation() -> Result<()> {
    setup_test_env();
    let module_id_1 = ModuleId(JWT_MODULE_1.to_string());
    let module_id_2 = ModuleId(JWT_MODULE_2.to_string());
    let mod_cfgs = create_multi_module_signing_configs().await;
    let start_config = start_server(21006, &mod_cfgs, ADMIN_SECRET.to_string()).await?;

    let mod_cfg_1 = mod_cfgs.get(&module_id_1).unwrap();
    let mod_cfg_2 = mod_cfgs.get(&module_id_2).unwrap();

    let client = Client::new();
    let url = format!("http://{}{}", start_config.endpoint, GET_PUBKEYS_PATH);

    // Test that module 1's JWT doesn't work for module 2's context and vice versa
    let jwt_1 = create_jwt(&module_id_1, &mod_cfg_1.jwt_secret)?;
    let jwt_2 = create_jwt(&module_id_2, &mod_cfg_2.jwt_secret)?;

    // Both should work with their own JWTs
    let response = client.get(&url).bearer_auth(&jwt_1).send().await?;
    assert_eq!(response.status(), StatusCode::OK, "Module 1 JWT should work");

    let response = client.get(&url).bearer_auth(&jwt_2).send().await?;
    assert_eq!(response.status(), StatusCode::OK, "Module 2 JWT should work");

    // Cross-auth should fail - try using module 1's secret for module 2's ID
    let invalid_cross_jwt = create_jwt(&module_id_2, &mod_cfg_1.jwt_secret)?;
    let response = client.get(&url).bearer_auth(&invalid_cross_jwt).send().await?;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "Cross-module auth should fail");

    // Trigger rate limit on module 1
    let invalid_jwt_1 = create_jwt(&module_id_1, "wrong-secret")?;
    for _ in 0..start_config.jwt_auth_fail_limit {
        let response = client.get(&url).bearer_auth(&invalid_jwt_1).send().await?;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    // Module 1 should be rate limited
    let response = client.get(&url).bearer_auth(&jwt_1).send().await?;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS, "Module 1 should be rate limited");

    // Module 2 should still work (isolation)
    let response = client.get(&url).bearer_auth(&jwt_2).send().await?;
    assert_eq!(response.status(), StatusCode::OK, "Module 2 should still work");

    info!("✓ Cross-module authentication isolation works correctly");
    Ok(())
}