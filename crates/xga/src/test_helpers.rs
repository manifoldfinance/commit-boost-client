use alloy::network::Ethereum;
use alloy::primitives::Address;
use commit_boost::prelude::*;
use std::sync::Arc;

use crate::config::XGAConfig;
use crate::eigenlayer::EigenLayerConfig;

/// Create a test configuration for EigenLayer
pub fn test_eigenlayer_config() -> EigenLayerConfig {
    EigenLayerConfig {
        enabled: true,
        registry_address: "0x1234567890123456789012345678901234567890".to_string(),
        rpc_url: "https://eth-mainnet.example.com".to_string(),
        operator_address: Some("0xabcdefabcdefabcdefabcdefabcdefabcdefabcd".to_string()),
    }
}

/// Create a test CB config
pub fn test_cb_config() -> Arc<StartCommitModuleConfig<XGAConfig>> {
    let chain = Chain {
        name: "ethereum".to_string(),
        chain: "ethereum".to_string(),
        rpc: vec!["https://eth-mainnet.example.com".to_string()],
        currency: Currency {
            name: "Ether".to_string(),
            symbol: "ETH".to_string(),
            decimals: 18,
        },
        chain_id: 1,
        network_id: Some(1),
        info_url: Some("https://ethereum.org".to_string()),
        native_currency: Some(Currency {
            name: "Ether".to_string(),
            symbol: "ETH".to_string(),
            decimals: 18,
        }),
        short_name: Some("eth".to_string()),
        explorer_url: None,
        parent: None,
        ens: None,
    };
    
    let xga_config = XGAConfig {
        webhook_port: 8080,
        xga_relays: vec!["https://relay.example.com".to_string()],
        commitment_delay_ms: 100,
        retry_attempts: 3,
        retry_delay_ms: 1000,
        max_registration_age_secs: 60,
        probe_relay_capabilities: false,
        eigenlayer: test_eigenlayer_config(),
    };
    
    let signer_client = SignerClient::new("http://localhost:19551".to_string());
    
    Arc::new(StartCommitModuleConfig {
        chain,
        signer_client,
        load_pbs_config: false,
        pbs_config: None,
        extra: xga_config,
    })
}

/// Test operator address
pub fn test_operator_address() -> Address {
    Address::from([0xab; 20])
}