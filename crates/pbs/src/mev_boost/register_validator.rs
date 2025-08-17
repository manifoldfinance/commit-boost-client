use std::time::{Duration, Instant};

use alloy::rpc::types::beacon::relay::ValidatorRegistration;
use axum::http::{HeaderMap, HeaderValue};
use cb_common::{
    pbs::{error::PbsError, RelayClient, HEADER_START_TIME_UNIX_MS},
    utils::{get_user_agent_with_version, read_chunked_body_with_max, utcnow_ms},
};
use eyre::bail;
use futures::future::{join_all, select_ok};
use reqwest::header::USER_AGENT;
use tracing::{debug, error, Instrument};
use url::Url;

use crate::{
    constants::{MAX_SIZE_DEFAULT, REGISTER_VALIDATOR_ENDPOINT_TAG, TIMEOUT_ERROR_CODE_STR},
    metrics::{
        REGISTRATIONS_PROCESSED, REGISTRATIONS_SKIPPED, REGISTRATION_CACHE_HITS,
        REGISTRATION_CACHE_MISSES, REGISTRATION_CACHE_SIZE, RELAY_LATENCY, RELAY_STATUS_CODE,
    },
    registration_cache::REGISTRATION_CACHE,
    state::{BuilderApiState, PbsState},
};

/// Implements https://ethereum.github.io/builder-specs/#/Builder/registerValidator
/// Returns 200 if at least one relay returns 200, else 503
pub async fn register_validator<S: BuilderApiState>(
    registrations: Vec<ValidatorRegistration>,
    req_headers: HeaderMap,
    state: PbsState<S>,
) -> eyre::Result<()> {
    let original_count = registrations.len();

    // Scope the cache operations to ensure lock is released
    let (filtered_registrations, cache_stats) = {
        let mut cache = REGISTRATION_CACHE.write();

        // Filter out validators that don't need re-registration
        let filtered_registrations: Vec<ValidatorRegistration> = registrations
            .into_iter()
            .filter(|reg| {
                cache.needs_registration(
                    &reg.message.pubkey,
                    &reg.message.fee_recipient,
                    reg.message.gas_limit,
                )
            })
            .collect();

        // Mark validators as registered before sending to relays
        // This prevents race conditions with concurrent requests
        for reg in &filtered_registrations {
            cache.mark_registered(
                reg.message.pubkey,
                reg.message.fee_recipient,
                reg.message.gas_limit,
            );
        }

        // Get cache statistics before releasing lock
        let stats = (cache.stats(), cache.hit_rate());

        (filtered_registrations, stats)
    }; // Cache lock is automatically dropped here

    let ((hits, misses, size), hit_rate) = cache_stats;

    // Update metrics after releasing the lock
    REGISTRATION_CACHE_HITS.inc_by(hits as u64);
    REGISTRATION_CACHE_MISSES.inc_by(misses as u64);
    REGISTRATION_CACHE_SIZE.set(size as i64);

    // Log cache statistics periodically
    if (hits + misses) % 100 == 0 && (hits + misses) > 0 {
        debug!(
            cache_hits = hits,
            cache_misses = misses,
            cache_size = size,
            hit_rate = hit_rate,
            "Registration cache stats"
        );
    }

    // If all validators are already registered, skip relay calls
    if filtered_registrations.is_empty() {
        REGISTRATIONS_SKIPPED.with_label_values(&["all_cached"]).inc_by(original_count as u64);
        debug!(
            skipped_count = original_count,
            "All validators already registered within TTL, skipping relay calls"
        );
        return Ok(());
    }

    // Record the number of registrations being processed
    REGISTRATIONS_PROCESSED
        .with_label_values(&["filtered"])
        .inc_by(filtered_registrations.len() as u64);
    if original_count > filtered_registrations.len() {
        REGISTRATIONS_SKIPPED
            .with_label_values(&["partial_cached"])
            .inc_by((original_count - filtered_registrations.len()) as u64);
    }

    debug!(
        original_count = original_count,
        filtered_count = filtered_registrations.len(),
        cache_hit_rate = hit_rate,
        "Filtered duplicate registrations"
    );

    // prepare headers
    let mut send_headers = HeaderMap::new();
    send_headers
        .insert(HEADER_START_TIME_UNIX_MS, HeaderValue::from_str(&utcnow_ms().to_string())?);
    send_headers.insert(USER_AGENT, get_user_agent_with_version(&req_headers)?);

    let relays = state.all_relays().to_vec();

    // Build all registration tasks upfront for parallel execution
    let mut handles = Vec::new();
    for relay in relays {
        let batches = if let Some(batch_size) = relay.config.validator_registration_batch_size {
            filtered_registrations.chunks(batch_size).map(|c| c.to_vec()).collect::<Vec<_>>()
        } else {
            vec![filtered_registrations.clone()]
        };

        // Create all tasks for this relay
        for batch in batches {
            handles.push(tokio::spawn(
                send_register_validator_with_timeout(
                    batch,
                    relay.clone(),
                    send_headers.clone(),
                    state.pbs_config().timeout_register_validator_ms,
                    state.pbs_config().register_validator_retry_limit,
                )
                .in_current_span(),
            ));
        }
    }

    if state.pbs_config().wait_all_registrations {
        // wait for all relays registrations to complete
        let results = join_all(handles).await;
        if results.into_iter().any(|res| res.is_ok_and(|res| res.is_ok())) {
            Ok(())
        } else {
            bail!("No relay passed register_validator successfully")
        }
    } else {
        // return once first completes, others proceed in background
        let result = select_ok(handles).await?;
        match result.0 {
            Ok(_) => Ok(()),
            Err(_) => bail!("No relay passed register_validator successfully"),
        }
    }
}

/// Register validator to relay, retry connection errors until the
/// given timeout has passed
async fn send_register_validator_with_timeout(
    registrations: Vec<ValidatorRegistration>,
    relay: RelayClient,
    headers: HeaderMap,
    timeout_ms: u64,
    retry_limit: u32,
) -> Result<(), PbsError> {
    let url = relay.register_validator_url()?;
    let mut remaining_timeout_ms = timeout_ms;
    let mut retry = 0;
    let mut backoff = Duration::from_millis(250);

    loop {
        let start_request = Instant::now();
        match send_register_validator(
            url.clone(),
            &registrations,
            &relay,
            headers.clone(),
            remaining_timeout_ms,
            retry,
        )
        .await
        {
            Ok(_) => return Ok(()),

            Err(err) if err.should_retry() => {
                retry += 1;
                if retry >= retry_limit {
                    error!(
                        relay_id = relay.id.as_str(),
                        retry, "reached retry limit for validator registration"
                    );
                    return Err(err);
                }
                tokio::time::sleep(backoff).await;
                backoff += Duration::from_millis(250);

                remaining_timeout_ms =
                    timeout_ms.saturating_sub(start_request.elapsed().as_millis() as u64);

                if remaining_timeout_ms == 0 {
                    return Err(err);
                }
            }

            Err(err) => return Err(err),
        };
    }
}

async fn send_register_validator(
    url: Url,
    registrations: &[ValidatorRegistration],
    relay: &RelayClient,
    headers: HeaderMap,
    timeout_ms: u64,
    retry: u32,
) -> Result<(), PbsError> {
    let start_request = Instant::now();
    let res = match relay
        .client
        .post(url)
        .timeout(Duration::from_millis(timeout_ms))
        .headers(headers)
        .json(&registrations)
        .send()
        .await
    {
        Ok(res) => res,
        Err(err) => {
            RELAY_STATUS_CODE
                .with_label_values(&[
                    TIMEOUT_ERROR_CODE_STR,
                    REGISTER_VALIDATOR_ENDPOINT_TAG,
                    &relay.id,
                ])
                .inc();
            return Err(err.into());
        }
    };
    let request_latency = start_request.elapsed();
    RELAY_LATENCY
        .with_label_values(&[REGISTER_VALIDATOR_ENDPOINT_TAG, &relay.id])
        .observe(request_latency.as_secs_f64());

    let code = res.status();
    RELAY_STATUS_CODE
        .with_label_values(&[code.as_str(), REGISTER_VALIDATOR_ENDPOINT_TAG, &relay.id])
        .inc();

    if !code.is_success() {
        let response_bytes = read_chunked_body_with_max(res, MAX_SIZE_DEFAULT).await?;
        let err = PbsError::RelayResponse {
            error_msg: String::from_utf8_lossy(&response_bytes).into_owned(),
            code: code.as_u16(),
        };

        // error here since we check if any success above
        error!(relay_id = relay.id.as_ref(), retry, %err, "failed registration");
        return Err(err);
    };

    debug!(
        relay_id = relay.id.as_ref(),
        retry,
        ?code,
        latency = ?request_latency,
        num_registrations = registrations.len(),
        "registration successful"
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::rpc::types::beacon::relay::ValidatorRegistration;
    use std::time::Instant;

    fn create_test_registrations(count: usize) -> Vec<ValidatorRegistration> {
        use alloy::rpc::types::beacon::{BlsPublicKey, BlsSignature};

        (0..count)
            .map(|i| ValidatorRegistration {
                message: alloy::rpc::types::beacon::relay::ValidatorRegistrationMessage {
                    fee_recipient: Default::default(),
                    gas_limit: 30_000_000,
                    timestamp: 1000 + i as u64,
                    pubkey: BlsPublicKey::from([i as u8; 48]),
                },
                signature: BlsSignature::from([0u8; 96]),
            })
            .collect()
    }

    fn calculate_batches(
        registrations: &[ValidatorRegistration],
        batch_size: Option<usize>,
    ) -> Vec<Vec<ValidatorRegistration>> {
        if let Some(size) = batch_size {
            registrations.chunks(size).map(|c| c.to_vec()).collect()
        } else {
            vec![registrations.to_vec()]
        }
    }

    #[test]
    fn test_batch_calculation() {
        let registrations = create_test_registrations(100);

        // Test with batch size
        let batches = calculate_batches(&registrations, Some(20));
        assert_eq!(batches.len(), 5);
        assert_eq!(batches[0].len(), 20);
        assert_eq!(batches[4].len(), 20);

        // Test without batch size
        let batches = calculate_batches(&registrations, None);
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].len(), 100);

        // Test with uneven batch size
        let registrations = create_test_registrations(95);
        let batches = calculate_batches(&registrations, Some(20));
        assert_eq!(batches.len(), 5);
        assert_eq!(batches[4].len(), 15); // Last batch has remainder
    }

    #[tokio::test]
    async fn test_parallel_execution_timing() {
        let start = Instant::now();

        // Create mock tasks that sleep
        let tasks: Vec<_> = (0..5)
            .map(|_| {
                tokio::spawn(async {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    Ok::<_, eyre::Error>(())
                })
            })
            .collect();

        let results = join_all(tasks).await;

        let elapsed = start.elapsed();

        // All tasks should complete in parallel
        // Should complete in ~100ms, not 500ms
        assert!(
            elapsed < Duration::from_millis(200),
            "Parallel execution took {:?}, expected < 200ms",
            elapsed
        );

        // Verify all completed successfully
        assert_eq!(results.len(), 5);
        for result in results {
            assert!(result.is_ok());
        }
    }

    #[tokio::test]
    async fn test_parallel_vs_sequential_timing() {
        // Sequential execution simulation
        let start_seq = Instant::now();
        for _ in 0..3 {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let seq_time = start_seq.elapsed();

        // Parallel execution
        let start_par = Instant::now();
        let tasks: Vec<_> = (0..3)
            .map(|_| {
                tokio::spawn(async {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                })
            })
            .collect();
        join_all(tasks).await;
        let par_time = start_par.elapsed();

        // Parallel should be significantly faster
        assert!(
            par_time < seq_time / 2,
            "Parallel {:?} should be much faster than sequential {:?}",
            par_time,
            seq_time
        );
    }

    #[test]
    fn test_registration_deduplication() {
        use crate::registration_cache::RegistrationCache;

        let mut cache = RegistrationCache::new();
        let registrations = create_test_registrations(5);

        // First time all should need registration
        for reg in &registrations {
            assert!(cache.needs_registration(
                &reg.message.pubkey,
                &reg.message.fee_recipient,
                reg.message.gas_limit
            ));
            cache.mark_registered(
                reg.message.pubkey,
                reg.message.fee_recipient,
                reg.message.gas_limit,
            );
        }

        // Second time none should need registration
        for reg in &registrations {
            assert!(!cache.needs_registration(
                &reg.message.pubkey,
                &reg.message.fee_recipient,
                reg.message.gas_limit
            ));
        }

        // Check cache stats
        let (hits, misses, size) = cache.stats();
        assert_eq!(hits, 5);
        assert_eq!(misses, 5);
        assert_eq!(size, 5);
        assert_eq!(cache.hit_rate(), 50.0);
    }

    #[test]
    fn test_registration_param_changes() {
        use crate::registration_cache::RegistrationCache;
        use alloy::primitives::Address;

        let mut cache = RegistrationCache::new();
        let reg = create_test_registrations(1).into_iter().next().unwrap();

        // Initial registration
        assert!(cache.needs_registration(
            &reg.message.pubkey,
            &reg.message.fee_recipient,
            reg.message.gas_limit
        ));
        cache.mark_registered(reg.message.pubkey, reg.message.fee_recipient, reg.message.gas_limit);

        // Same params should not need re-registration
        assert!(!cache.needs_registration(
            &reg.message.pubkey,
            &reg.message.fee_recipient,
            reg.message.gas_limit
        ));

        // Change fee recipient - should need re-registration
        let new_fee_recipient = Address::from([1u8; 20]);
        assert!(cache.needs_registration(
            &reg.message.pubkey,
            &new_fee_recipient,
            reg.message.gas_limit
        ));

        // Change gas limit - should need re-registration
        assert!(cache.needs_registration(
            &reg.message.pubkey,
            &reg.message.fee_recipient,
            25_000_000
        ));
    }

    #[test]
    fn test_filtered_registrations_logic() {
        use crate::registration_cache::RegistrationCache;

        let mut cache = RegistrationCache::new();
        let registrations = create_test_registrations(10);

        // Mark first 5 as already registered
        for reg in &registrations[0..5] {
            cache.mark_registered(
                reg.message.pubkey,
                reg.message.fee_recipient,
                reg.message.gas_limit,
            );
        }

        // Filter registrations like the main function does
        let filtered: Vec<_> = registrations
            .iter()
            .filter(|reg| {
                cache.needs_registration(
                    &reg.message.pubkey,
                    &reg.message.fee_recipient,
                    reg.message.gas_limit,
                )
            })
            .cloned()
            .collect();

        // Should only have 5 registrations (the ones not in cache)
        assert_eq!(filtered.len(), 5);

        // Verify the filtered ones are the correct ones (6-10)
        for (i, reg) in filtered.iter().enumerate() {
            assert_eq!(reg.message.pubkey, registrations[i + 5].message.pubkey);
        }
    }

    #[test]
    fn test_cache_concurrent_access() {
        use crate::registration_cache::REGISTRATION_CACHE;
        use std::thread;

        let registrations = create_test_registrations(20);
        let mut handles = vec![];

        // Spawn multiple threads accessing the global cache
        for chunk in registrations.chunks(5) {
            let chunk_regs = chunk.to_vec();
            let handle = thread::spawn(move || {
                let mut cache = REGISTRATION_CACHE.write();
                for reg in chunk_regs {
                    let needs = cache.needs_registration(
                        &reg.message.pubkey,
                        &reg.message.fee_recipient,
                        reg.message.gas_limit,
                    );
                    if needs {
                        cache.mark_registered(
                            reg.message.pubkey,
                            reg.message.fee_recipient,
                            reg.message.gas_limit,
                        );
                    }
                }
            });
            handles.push(handle);
        }

        // Wait for all threads to complete
        for handle in handles {
            handle.join().unwrap();
        }

        // Verify all registrations are in cache
        let cache = REGISTRATION_CACHE.read();
        let (_, _, size) = cache.stats();
        assert_eq!(size, 20);
    }
}
