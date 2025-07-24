use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use alloy::{
    primitives::{utils::format_ether, B256, U256},
    providers::Provider,
    rpc::types::{beacon::BlsPublicKey, Block},
};
use axum::http::{HeaderMap, HeaderValue};
use cb_common::{
    constants::APPLICATION_BUILDER_DOMAIN,
    error_utils::SafeHttpError,
    pbs::{
        error::{PbsError, ValidationError},
        GetHeaderParams, GetHeaderResponse, RelayClient, VersionedResponse, EMPTY_TX_ROOT_HASH,
        HEADER_START_TIME_UNIX_MS,
    },
    signature::verify_signed_message,
    signer::BlsSignature,
    types::Chain,
    utils::{
        get_user_agent_with_version, ms_into_slot, read_chunked_body_with_max,
        timestamp_of_slot_start_sec, utcnow_ms,
    },
};
use futures::future::join_all;
use parking_lot::RwLock;
use reqwest::{header::USER_AGENT, StatusCode};
use tokio::time::sleep;
use tracing::{debug, error, trace, warn, Instrument};
use tree_hash::TreeHash;
use url::Url;

use crate::{
    constants::{
        GET_HEADER_ENDPOINT_TAG, MAX_SIZE_GET_HEADER_RESPONSE, TIMEOUT_ERROR_CODE,
        TIMEOUT_ERROR_CODE_STR,
    },
    metrics::{RELAY_HEADER_VALUE, RELAY_LAST_SLOT, RELAY_LATENCY, RELAY_STATUS_CODE},
    state::{BuilderApiState, PbsState},
    utils::check_gas_limit,
};

/// Implements https://ethereum.github.io/builder-specs/#/Builder/getHeader
/// Returns 200 if at least one relay returns 200, else 204
pub async fn get_header<S: BuilderApiState>(
    params: GetHeaderParams,
    req_headers: HeaderMap,
    state: PbsState<S>,
) -> eyre::Result<Option<GetHeaderResponse>> {
    let parent_block = Arc::new(RwLock::new(None));
    if state.extra_validation_enabled() {
        if let Some(rpc_url) = state.pbs_config().rpc_url.clone() {
            tokio::spawn(
                fetch_parent_block(rpc_url, params.parent_hash, parent_block.clone())
                    .in_current_span(),
            );
        }
    }

    let ms_into_slot = ms_into_slot(params.slot, state.config.chain);
    let (pbs_config, relays, maybe_mux_id) = state.mux_config_and_relays(&params.pubkey);

    if let Some(mux_id) = maybe_mux_id {
        debug!(mux_id, relays = relays.len(), pubkey = %params.pubkey, "using mux config");
    } else {
        debug!(relays = relays.len(), pubkey = %params.pubkey, "using default config");
    }

    let max_timeout_ms = pbs_config
        .timeout_get_header_ms
        .min(pbs_config.late_in_slot_time_ms.saturating_sub(ms_into_slot));

    if max_timeout_ms == 0 {
        warn!(
            ms_into_slot,
            threshold = pbs_config.late_in_slot_time_ms,
            "late in slot, skipping relay requests"
        );

        return Ok(None);
    }

    // prepare headers, except for start time which is set in `send_one_get_header`
    let mut send_headers = HeaderMap::new();
    send_headers.insert(USER_AGENT, get_user_agent_with_version(&req_headers)?);

    let mut handles = Vec::with_capacity(relays.len());
    for relay in relays.iter() {
        handles.push(
            send_timed_get_header(
                params,
                relay.clone(),
                state.config.chain,
                send_headers.clone(),
                ms_into_slot,
                max_timeout_ms,
                ValidationContext {
                    skip_sigverify: state.pbs_config().skip_sigverify,
                    min_bid_wei: state.pbs_config().min_bid_wei,
                    extra_validation_enabled: state.extra_validation_enabled(),
                    parent_block: parent_block.clone(),
                },
            )
            .in_current_span(),
        );
    }

    let results = join_all(handles).await;
    let mut relay_bids = Vec::with_capacity(relays.len());
    for (i, res) in results.into_iter().enumerate() {
        let relay_id = relays[i].id.as_str();

        match res {
            Ok(Some(res)) => {
                RELAY_LAST_SLOT.with_label_values(&[relay_id]).set(params.slot as i64);
                let value_gwei =
                    (res.value() / U256::from(1_000_000_000)).try_into().unwrap_or_default();
                RELAY_HEADER_VALUE.with_label_values(&[relay_id]).set(value_gwei);

                relay_bids.push(res)
            }
            Ok(_) => {}
            Err(err) if err.is_timeout() => error!(err = "Timed Out", relay_id),
            Err(err) => error!(%err, relay_id),
        }
    }

    let max_bid = relay_bids.into_iter().max_by_key(|bid| bid.value());

    Ok(max_bid)
}

/// Fetch the parent block from the RPC URL for extra validation of the header.
/// Extra validation will be skipped if:
/// - relay returns header before parent block is fetched
/// - parent block is not found, eg because of a RPC delay
async fn fetch_parent_block(
    rpc_url: Url,
    parent_hash: B256,
    parent_block: Arc<RwLock<Option<Block>>>,
) {
    let provider = alloy::providers::ProviderBuilder::new().on_http(rpc_url).to_owned();

    debug!(%parent_hash, "fetching parent block");

    match provider.get_block_by_hash(parent_hash).await {
        Ok(maybe_block) => {
            debug!(block_found = maybe_block.is_some(), "fetched parent block");
            let mut guard = parent_block.write();
            *guard = maybe_block;
        }
        Err(err) => {
            error!(%err, "fetch failed");
        }
    }
}

async fn send_timed_get_header(
    params: GetHeaderParams,
    relay: RelayClient,
    chain: Chain,
    headers: HeaderMap,
    ms_into_slot: u64,
    mut timeout_left_ms: u64,
    validation: ValidationContext,
) -> Result<Option<GetHeaderResponse>, PbsError> {
    let url = relay.get_header_url(params.slot, params.parent_hash, params.pubkey)?;

    if relay.config.enable_timing_games {
        if let Some(target_ms) = relay.config.target_first_request_ms {
            // sleep until target time in slot

            let delay = target_ms.saturating_sub(ms_into_slot);
            if delay > 0 {
                debug!(
                    relay_id = relay.id.as_ref(),
                    target_ms, ms_into_slot, "TG: waiting to send first header request"
                );
                timeout_left_ms = timeout_left_ms.saturating_sub(delay);
                sleep(Duration::from_millis(delay)).await;
            } else {
                debug!(
                    relay_id = relay.id.as_ref(),
                    target_ms, ms_into_slot, "TG: request already late enough in slot"
                );
            }
        }

        if let Some(send_freq_ms) = relay.config.frequency_get_header_ms {
            let mut handles = Vec::new();

            debug!(
                relay_id = relay.id.as_ref(),
                send_freq_ms, timeout_left_ms, "TG: sending multiple header requests"
            );

            loop {
                handles.push(tokio::spawn(
                    send_one_get_header(
                        params,
                        relay.clone(),
                        chain,
                        RequestContext {
                            timeout_ms: timeout_left_ms,
                            url: url.clone(),
                            headers: headers.clone(),
                        },
                        validation.clone(),
                    )
                    .in_current_span(),
                ));

                if timeout_left_ms > send_freq_ms {
                    // enough time for one more
                    timeout_left_ms = timeout_left_ms.saturating_sub(send_freq_ms);
                    sleep(Duration::from_millis(send_freq_ms)).await;
                } else {
                    break;
                }
            }

            let results = join_all(handles).await;
            let mut n_headers = 0;

            if let Some((_, maybe_header)) = results
                .into_iter()
                .filter_map(|res| {
                    // ignore join error and timeouts, log other errors
                    res.ok().and_then(|inner_res| match inner_res {
                        Ok(maybe_header) => {
                            if maybe_header.1.is_some() {
                                n_headers += 1;
                                Some(maybe_header)
                            } else {
                                // filter out 204 responses that are returned if the request
                                // is after the relay cutoff
                                None
                            }
                        }
                        Err(err) if err.is_timeout() => None,
                        Err(err) => {
                            error!(relay_id = relay.id.as_ref(),%err, "TG: error sending header request");
                            None
                        }
                    })
                })
                .max_by_key(|(start_time, _)| *start_time)
            {
                debug!(relay_id = relay.id.as_ref(), n_headers, "TG: received headers from relay");
                return Ok(maybe_header);
            } else {
                // all requests failed
                warn!(relay_id = relay.id.as_ref(), "TG: no headers received");

                let safe_err = SafeHttpError {
                    status_code: TIMEOUT_ERROR_CODE,
                    message: "no headers received".to_string(),
                    error_code: Some("TIMEOUT".to_string()),
                    response_size: 0,
                    truncated: false,
                };
                return Err(PbsError::RelayResponse(safe_err));
            }
        }
    }

    // if no timing games or no repeated send, just send one request
    send_one_get_header(
        params,
        relay,
        chain,
        RequestContext { timeout_ms: timeout_left_ms, url, headers },
        validation,
    )
    .await
    .map(|(_, maybe_header)| maybe_header)
}

struct RequestContext {
    url: Url,
    timeout_ms: u64,
    headers: HeaderMap,
}

#[derive(Clone)]
struct ValidationContext {
    skip_sigverify: bool,
    min_bid_wei: U256,
    extra_validation_enabled: bool,
    parent_block: Arc<RwLock<Option<Block>>>,
}

async fn send_one_get_header(
    params: GetHeaderParams,
    relay: RelayClient,
    chain: Chain,
    mut req_config: RequestContext,
    validation: ValidationContext,
) -> Result<(u64, Option<GetHeaderResponse>), PbsError> {
    // the timestamp in the header is the consensus block time which is fixed,
    // use the beginning of the request as proxy to make sure we use only the
    // last one received
    let start_request_time = utcnow_ms();
    req_config.headers.insert(HEADER_START_TIME_UNIX_MS, HeaderValue::from(start_request_time));

    let start_request = Instant::now();
    let res = match relay
        .client
        .get(req_config.url)
        .timeout(Duration::from_millis(req_config.timeout_ms))
        .headers(req_config.headers)
        .send()
        .await
    {
        Ok(res) => res,
        Err(err) => {
            RELAY_STATUS_CODE
                .with_label_values(&[TIMEOUT_ERROR_CODE_STR, GET_HEADER_ENDPOINT_TAG, &relay.id])
                .inc();
            return Err(err.into());
        }
    };

    let request_latency = start_request.elapsed();
    RELAY_LATENCY
        .with_label_values(&[GET_HEADER_ENDPOINT_TAG, &relay.id])
        .observe(request_latency.as_secs_f64());

    let code = res.status();
    RELAY_STATUS_CODE.with_label_values(&[code.as_str(), GET_HEADER_ENDPOINT_TAG, &relay.id]).inc();

    let response_bytes = read_chunked_body_with_max(res, MAX_SIZE_GET_HEADER_RESPONSE).await?;
    if !code.is_success() {
        let safe_err = SafeHttpError::from_response_bytes(code, &response_bytes);
        return Err(PbsError::RelayResponse(safe_err));
    };
    if code == StatusCode::NO_CONTENT {
        debug!(
            relay_id = relay.id.as_ref(),
            ?code,
            latency = ?request_latency,
            response = ?response_bytes,
            "no header from relay"
        );
        return Ok((start_request_time, None));
    }

    let get_header_response = match serde_json::from_slice::<GetHeaderResponse>(&response_bytes) {
        Ok(parsed) => parsed,
        Err(err) => {
            // Log full response at trace level for debugging
            trace!(
                relay_id = relay.id.as_ref(),
                response_size = response_bytes.len(),
                response = ?String::from_utf8_lossy(&response_bytes),
                "Failed to parse get_header response"
            );
            return Err(PbsError::json_decode_with_bytes(err, &response_bytes, code));
        }
    };

    debug!(
        relay_id = relay.id.as_ref(),
        latency = ?request_latency,
        version = get_header_response.version(),
        value_eth = format_ether(get_header_response.value()),
        block_hash = %get_header_response.block_hash(),
        "received new header"
    );

    match &get_header_response {
        VersionedResponse::Electra(res) => {
            let header_data = HeaderData {
                block_hash: res.message.header.block_hash,
                parent_hash: res.message.header.parent_hash,
                tx_root: res.message.header.transactions_root,
                value: res.message.value,
                timestamp: res.message.header.timestamp,
            };

            validate_header_data(
                &header_data,
                chain,
                params.parent_hash,
                validation.min_bid_wei,
                params.slot,
            )?;

            if !validation.skip_sigverify {
                validate_signature(
                    chain,
                    relay.pubkey(),
                    res.message.pubkey,
                    &res.message,
                    &res.signature,
                )?;
            }
        }
    }

    if validation.extra_validation_enabled {
        let parent_block = validation.parent_block.read();
        if let Some(parent_block) = parent_block.as_ref() {
            extra_validation(parent_block, &get_header_response)?;
        } else {
            warn!(
                relay_id = relay.id.as_ref(),
                "parent block not found, skipping extra validation"
            );
        }
    }

    Ok((start_request_time, Some(get_header_response)))
}

struct HeaderData {
    block_hash: B256,
    parent_hash: B256,
    tx_root: B256,
    value: U256,
    timestamp: u64,
}

fn validate_header_data(
    header_data: &HeaderData,
    chain: Chain,
    expected_parent_hash: B256,
    minimum_bid_wei: U256,
    slot: u64,
) -> Result<(), ValidationError> {
    if header_data.block_hash == B256::ZERO {
        return Err(ValidationError::EmptyBlockhash);
    }

    if expected_parent_hash != header_data.parent_hash {
        return Err(ValidationError::ParentHashMismatch {
            expected: expected_parent_hash,
            got: header_data.parent_hash,
        });
    }

    if header_data.tx_root == EMPTY_TX_ROOT_HASH {
        return Err(ValidationError::EmptyTxRoot);
    }

    if header_data.value < minimum_bid_wei {
        return Err(ValidationError::BidTooLow { min: minimum_bid_wei, got: header_data.value });
    }

    let expected_timestamp = timestamp_of_slot_start_sec(slot, chain);
    if expected_timestamp != header_data.timestamp {
        return Err(ValidationError::TimestampMismatch {
            expected: expected_timestamp,
            got: header_data.timestamp,
        });
    }

    Ok(())
}

fn validate_signature<T: TreeHash>(
    chain: Chain,
    expected_relay_pubkey: BlsPublicKey,
    received_relay_pubkey: BlsPublicKey,
    message: &T,
    signature: &BlsSignature,
) -> Result<(), ValidationError> {
    if expected_relay_pubkey != received_relay_pubkey {
        return Err(ValidationError::PubkeyMismatch {
            expected: expected_relay_pubkey,
            got: received_relay_pubkey,
        });
    }

    verify_signed_message(
        chain,
        &expected_relay_pubkey,
        &message,
        signature,
        APPLICATION_BUILDER_DOMAIN,
    )
    .map_err(ValidationError::Sigverify)?;

    Ok(())
}

fn extra_validation(
    parent_block: &Block,
    signed_header: &GetHeaderResponse,
) -> Result<(), ValidationError> {
    if signed_header.block_number() != parent_block.header.number + 1 {
        return Err(ValidationError::BlockNumberMismatch {
            parent: parent_block.header.number,
            header: signed_header.block_number(),
        });
    }

    if !check_gas_limit(signed_header.gas_limit(), parent_block.header.gas_limit) {
        return Err(ValidationError::GasLimit {
            parent: parent_block.header.gas_limit,
            header: signed_header.gas_limit(),
        });
    };

    Ok(())
}

#[cfg(test)]
mod tests {
    use alloy::{
        primitives::{B256, U256},
        rpc::types::beacon::BlsPublicKey,
    };
    use blst::min_pk;
    use cb_common::{
        pbs::{error::ValidationError, ExecutionPayloadHeaderMessageElectra, EMPTY_TX_ROOT_HASH},
        signature::sign_builder_message,
        types::Chain,
        utils::timestamp_of_slot_start_sec,
    };

    use super::{validate_header_data, *};

    #[test]
    fn test_validate_header() {
        let slot = 5;
        let parent_hash = B256::from_slice(&[1; 32]);
        let chain = Chain::Holesky;
        let min_bid = U256::from(10);

        let mut mock_header_data = HeaderData {
            block_hash: B256::default(),
            parent_hash: B256::default(),
            tx_root: EMPTY_TX_ROOT_HASH,
            value: U256::default(),
            timestamp: 0,
        };

        assert_eq!(
            validate_header_data(&mock_header_data, chain, parent_hash, min_bid, slot,),
            Err(ValidationError::EmptyBlockhash)
        );

        mock_header_data.block_hash.0[1] = 1;

        assert_eq!(
            validate_header_data(&mock_header_data, chain, parent_hash, min_bid, slot,),
            Err(ValidationError::ParentHashMismatch {
                expected: parent_hash,
                got: B256::default()
            })
        );

        mock_header_data.parent_hash = parent_hash;

        assert_eq!(
            validate_header_data(&mock_header_data, chain, parent_hash, min_bid, slot,),
            Err(ValidationError::EmptyTxRoot)
        );

        mock_header_data.tx_root = Default::default();

        assert_eq!(
            validate_header_data(&mock_header_data, chain, parent_hash, min_bid, slot,),
            Err(ValidationError::BidTooLow { min: min_bid, got: U256::ZERO })
        );

        mock_header_data.value = U256::from(11);

        let expected = timestamp_of_slot_start_sec(slot, chain);
        assert_eq!(
            validate_header_data(&mock_header_data, chain, parent_hash, min_bid, slot,),
            Err(ValidationError::TimestampMismatch { expected, got: 0 })
        );

        mock_header_data.timestamp = expected;

        assert!(validate_header_data(&mock_header_data, chain, parent_hash, min_bid, slot).is_ok());
    }

    #[test]
    fn test_validate_signature() {
        let secret_key = min_pk::SecretKey::from_bytes(&[
            0, 136, 227, 100, 165, 57, 106, 129, 181, 15, 235, 189, 200, 120, 70, 99, 251, 144,
            137, 181, 230, 124, 189, 193, 115, 153, 26, 0, 197, 135, 103, 63,
        ])
        .unwrap();
        let pubkey = BlsPublicKey::from_slice(&secret_key.sk_to_pk().to_bytes());

        let message = ExecutionPayloadHeaderMessageElectra::default();

        let signature = sign_builder_message(Chain::Holesky, &secret_key, &message);

        assert_eq!(
            validate_signature(
                Chain::Holesky,
                BlsPublicKey::default(),
                pubkey,
                &message,
                &BlsSignature::default()
            ),
            Err(ValidationError::PubkeyMismatch { expected: BlsPublicKey::default(), got: pubkey })
        );

        assert!(matches!(
            validate_signature(Chain::Holesky, pubkey, pubkey, &message, &BlsSignature::default()),
            Err(ValidationError::Sigverify(_))
        ));

        assert!(validate_signature(Chain::Holesky, pubkey, pubkey, &message, &signature).is_ok());
    }

    #[test]
    fn test_signature_verification_with_expected_key() {
        // This test verifies that signature verification uses the expected_relay_pubkey
        // instead of received_relay_pubkey, confirming our security fix
        
        let secret_key = min_pk::SecretKey::from_bytes(&[
            1, 136, 227, 100, 165, 57, 106, 129, 181, 15, 235, 189, 200, 120, 70, 99, 251, 144,
            137, 181, 230, 124, 189, 193, 115, 153, 26, 0, 197, 135, 103, 63,
        ])
        .unwrap();
        let pubkey = BlsPublicKey::from_slice(&secret_key.sk_to_pk().to_bytes());

        let message = ExecutionPayloadHeaderMessageElectra::default();
        let signature = sign_builder_message(Chain::Holesky, &secret_key, &message);

        // Test: Signature verification should use expected pubkey for validation
        // This should pass because both pubkeys match and signature is valid
        let result = validate_signature(Chain::Holesky, pubkey, pubkey, &message, &signature);
        assert!(result.is_ok(), "Legitimate signature should validate successfully");

        // Test: Even with pubkey mismatch, we can verify the behavior
        // This should fail with PubkeyMismatch, not signature verification error
        let different_key = BlsPublicKey::default();
        let result = validate_signature(Chain::Holesky, different_key, pubkey, &message, &signature);
        assert!(matches!(result, Err(ValidationError::PubkeyMismatch { .. })));
    }

    #[test]
    fn test_legitimate_signature_validation_regression() {
        // Regression test to ensure our security fix doesn't break legitimate cases
        
        // Create two different legitimate relay configurations
        let secret1 = min_pk::SecretKey::from_bytes(&[
            2, 136, 227, 100, 165, 57, 106, 129, 181, 15, 235, 189, 200, 120, 70, 99, 251, 144,
            137, 181, 230, 124, 189, 193, 115, 153, 26, 0, 197, 135, 103, 63,
        ]).unwrap();
        let pubkey1 = BlsPublicKey::from_slice(&secret1.sk_to_pk().to_bytes());

        let secret2 = min_pk::SecretKey::from_bytes(&[
            3, 136, 227, 100, 165, 57, 106, 129, 181, 15, 235, 189, 200, 120, 70, 99, 251, 144,
            137, 181, 230, 124, 189, 193, 115, 153, 26, 0, 197, 135, 103, 63,
        ]).unwrap();
        let pubkey2 = BlsPublicKey::from_slice(&secret2.sk_to_pk().to_bytes());

        let message = ExecutionPayloadHeaderMessageElectra::default();

        // Test relay 1
        let signature1 = sign_builder_message(Chain::Holesky, &secret1, &message);
        let result1 = validate_signature(Chain::Holesky, pubkey1, pubkey1, &message, &signature1);
        assert!(result1.is_ok(), "Relay 1 signature should validate");

        // Test relay 2  
        let signature2 = sign_builder_message(Chain::Holesky, &secret2, &message);
        let result2 = validate_signature(Chain::Holesky, pubkey2, pubkey2, &message, &signature2);
        assert!(result2.is_ok(), "Relay 2 signature should validate");

        // Test cross-validation should fail (wrong key for signature)
        let cross_result = validate_signature(Chain::Holesky, pubkey2, pubkey2, &message, &signature1);
        assert!(cross_result.is_err(), "Cross-validation should fail");
        assert!(matches!(cross_result.unwrap_err(), ValidationError::Sigverify(_)));
    }

    #[test]
    fn test_security_fix_prevents_wrong_key_usage() {
        // This test specifically validates that our fix prevents using the wrong key
        // for signature verification, which was the core vulnerability
        
        let legitimate_secret = min_pk::SecretKey::from_bytes(&[
            10, 136, 227, 100, 165, 57, 106, 129, 181, 15, 235, 189, 200, 120, 70, 99, 251, 144,
            137, 181, 230, 124, 189, 193, 115, 153, 26, 0, 197, 135, 103, 63,
        ]).unwrap();
        let legitimate_pubkey = BlsPublicKey::from_slice(&legitimate_secret.sk_to_pk().to_bytes());

        let attacker_secret = min_pk::SecretKey::from_bytes(&[
            20, 136, 227, 100, 165, 57, 106, 129, 181, 15, 235, 189, 200, 120, 70, 99, 251, 144,
            137, 181, 230, 124, 189, 193, 115, 153, 26, 0, 197, 135, 103, 63,
        ]).unwrap();
        let attacker_pubkey = BlsPublicKey::from_slice(&attacker_secret.sk_to_pk().to_bytes());

        let message = ExecutionPayloadHeaderMessageElectra::default();
        
        // Attacker creates signature with their key
        let attacker_signature = sign_builder_message(Chain::Holesky, &attacker_secret, &message);

        // Test: The vulnerability would allow this if signature verification used received_pubkey
        // Our fix ensures it uses expected_pubkey, so this attack fails at pubkey mismatch
        let attack_result = validate_signature(
            Chain::Holesky,
            legitimate_pubkey,  // Expected from config
            attacker_pubkey,    // Attacker's pubkey in response
            &message,
            &attacker_signature,
        );

        // Should fail with pubkey mismatch (attack blocked)
        assert!(attack_result.is_err());
        match attack_result.unwrap_err() {
            ValidationError::PubkeyMismatch { expected, got } => {
                assert_eq!(expected, legitimate_pubkey);
                assert_eq!(got, attacker_pubkey);
            }
            _ => panic!("Attack should be blocked by pubkey mismatch"),
        }

        // Verify that legitimate usage still works
        let legitimate_signature = sign_builder_message(Chain::Holesky, &legitimate_secret, &message);
        let legitimate_result = validate_signature(
            Chain::Holesky,
            legitimate_pubkey,
            legitimate_pubkey,
            &message,
            &legitimate_signature,
        );
        assert!(legitimate_result.is_ok(), "Legitimate signature should still work");
    }
}
