use std::{sync::Arc, time::Duration};

use alloy::rpc::types::beacon::relay::ValidatorRegistration;
use cb_common::{
    signer::{random_secret, BlsPublicKey},
    types::Chain,
    utils::blst_pubkey_to_alloy,
};
use cb_tests::mock_relay::{start_mock_relay_service, MockRelayState};
use eyre::Result;
use reqwest::StatusCode;

#[tokio::test]
async fn test_xga_mock_relay_support() -> Result<()> {
    // Create test setup
    let signer = random_secret();
    let pubkey: BlsPublicKey = blst_pubkey_to_alloy(&signer.sk_to_pk()).into();
    let chain = Chain::Holesky;
    let port = 5000;
    
    // Create mock registration
    let test_registration: ValidatorRegistration = serde_json::from_str(
        r#"{
            "message": {
                "fee_recipient": "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "gas_limit": "100000",
                "timestamp": "1000000",
                "pubkey": "0xb060572f535ba5615b874ebfef757fbe6825352ad257e31d724e57fe25a067a13cfddd0f00cb17bf3a3d2e901a380c17"
            },
            "signature": "0x88274f2d78d30ae429cc16f5c64657b491ccf26291c821cf953da34f16d60947d4f245decdce4a492e8d8f949482051b184aaa890d5dd97788387689335a1fee37cbe55c0227f81b073ce6e93b45f96169f497ed322d3d384d79ccaa7846d5ab"
        }"#,
    )?;
    
    // Start mock relay with XGA support
    let mock_state = Arc::new(
        MockRelayState::new(chain, signer)
            .with_xga_support(vec![test_registration.clone()])
    );
    tokio::spawn(start_mock_relay_service(mock_state.clone(), port));
    
    // Give server time to start
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    // Test 1: Check XGA capabilities
    let client = reqwest::Client::new();
    let capabilities_url = format!("http://localhost:{}/eth/v1/builder/xga/capabilities", port);
    let resp = client.get(&capabilities_url).send().await?;
    assert_eq!(resp.status(), StatusCode::OK);
    
    let capabilities: serde_json::Value = resp.json().await?;
    assert_eq!(capabilities["supported"], true);
    assert_eq!(capabilities["version"], "1.0.0");
    
    // Test 2: Get registrations
    let registrations_url = format!("http://localhost:{}/eth/v1/builder/registrations", port);
    let resp = client.get(&registrations_url).send().await?;
    assert_eq!(resp.status(), StatusCode::OK);
    
    let response: serde_json::Value = resp.json().await?;
    let registrations = response["registrations"].as_array().unwrap();
    assert_eq!(registrations.len(), 1);
    
    // Test 3: Submit XGA commitment
    let commitment_url = format!("http://localhost:{}/eth/v1/builder/xga/commitment", port);
    let test_commitment = serde_json::json!({
        "message": {
            "registration_hash": [0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],
            "validator_pubkey": pubkey,
            "relay_id": [0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],
            "xga_version": 1,
            "parameters": {
                "version": 1,
                "min_inclusion_slot": 100,
                "max_inclusion_slot": 200,
                "flags": 0
            },
            "timestamp": 1000000,
            "chain_id": chain.id(),
            "signing_domain": [0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]
        },
        "signature": [0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]
    });
    
    let resp = client.post(&commitment_url)
        .json(&test_commitment)
        .send()
        .await?;
    assert_eq!(resp.status(), StatusCode::OK);
    
    let response: serde_json::Value = resp.json().await?;
    assert_eq!(response["success"], true);
    assert!(response["commitment_id"].is_string());
    
    // Verify counters
    assert_eq!(mock_state.received_xga_commitments(), 1);
    let stored = mock_state.get_stored_xga_commitments();
    assert_eq!(stored.len(), 1);
    
    Ok(())
}

#[tokio::test]
async fn test_xga_disabled() -> Result<()> {
    let signer = random_secret();
    let chain = Chain::Holesky;
    let port = 5001;
    
    // Start mock relay WITHOUT XGA support
    let mock_state = Arc::new(MockRelayState::new(chain, signer));
    tokio::spawn(start_mock_relay_service(mock_state.clone(), port));
    
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    // Try to access XGA endpoints - should return NOT_FOUND
    let client = reqwest::Client::new();
    
    let capabilities_url = format!("http://localhost:{}/eth/v1/builder/xga/capabilities", port);
    let resp = client.get(&capabilities_url).send().await?;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    
    let commitment_url = format!("http://localhost:{}/eth/v1/builder/xga/commitment", port);
    let resp = client.post(&commitment_url).json(&serde_json::json!({})).send().await?;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    
    Ok(())
}