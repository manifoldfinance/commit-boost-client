//! Test utilities for fuzzing and property-based testing

use alloy_rpc_types::beacon::relay::{ValidatorRegistration, ValidatorRegistrationMessage};
use commit_boost::prelude::BlsPublicKey;
use proptest::prelude::*;

use crate::{
    commitment::{RegistrationNotification, XGACommitment, XGAParameters},
    config::XGAConfig,
    infrastructure::get_current_timestamp,
};

/// Generate arbitrary bytes of a specific length
pub fn arb_bytes<const N: usize>() -> impl Strategy<Value = [u8; N]> {
    prop::collection::vec(any::<u8>(), N).prop_map(|v| {
        let mut arr = [0u8; N];
        arr.copy_from_slice(&v);
        arr
    })
}

/// Generate arbitrary valid URLs
pub fn arb_valid_url() -> impl Strategy<Value = String> {
    (
        prop::bool::ANY,
        "[a-z]{3,10}",
        "[a-z]{2,6}",
        prop::option::of("[a-z0-9/-]{0,20}"),
        prop::option::of(1000u16..9999),
    )
        .prop_map(|(use_subdomain, domain, tld, path, port)| {
            let subdomain = if use_subdomain { "relay." } else { "" };
            let base = format!("https://{}{}.{}", subdomain, domain, tld);
            let with_path = if let Some(p) = path { format!("{}/{}", base, p) } else { base };
            if let Some(p) = port {
                format!("{}:{}", with_path, p)
            } else {
                with_path
            }
        })
}

/// Generate arbitrary invalid URLs
pub fn arb_invalid_url() -> impl Strategy<Value = String> {
    prop_oneof![
        // HTTP instead of HTTPS
        arb_valid_url().prop_map(|url| url.replace("https://", "http://")),
        // Local addresses
        Just("https://localhost".to_string()),
        Just("https://127.0.0.1".to_string()),
        Just("https://192.168.1.1".to_string()),
        Just("https://10.0.0.1".to_string()),
        Just("https://172.16.0.1".to_string()),
        // Malformed URLs
        Just("not-a-url".to_string()),
        Just("".to_string()),
        Just("https://".to_string()),
        // Random strings
        "[a-zA-Z0-9!@#$%^&*()_+-=]{1,50}",
    ]
}

/// Generate arbitrary XGA parameters
pub fn arb_xga_parameters() -> impl Strategy<Value = XGAParameters> {
    (
        any::<u64>(), // version
        any::<u64>(), // min_inclusion_slot
        any::<u64>(), // max_inclusion_slot
        any::<u64>(), // flags
    )
        .prop_map(|(version, min_slot, max_slot, flags)| {
            // Ensure min <= max
            let (min_inclusion_slot, max_inclusion_slot) =
                if min_slot <= max_slot { (min_slot, max_slot) } else { (max_slot, min_slot) };

            XGAParameters { version, min_inclusion_slot, max_inclusion_slot, flags }
        })
}

/// Generate arbitrary XGA commitment
pub fn arb_xga_commitment() -> impl Strategy<Value = XGACommitment> {
    (arb_bytes::<32>(), arb_bytes::<48>(), arb_valid_url(), any::<u64>(), arb_xga_parameters())
        .prop_map(|(reg_hash, pubkey, relay_url, chain_id, params)| {
            XGACommitment::new(reg_hash, BlsPublicKey::from(pubkey), relay_url, chain_id, params)
        })
}

/// Generate arbitrary validator registration
pub fn arb_validator_registration() -> impl Strategy<Value = ValidatorRegistration> {
    (
        arb_bytes::<48>(),           // pubkey
        arb_bytes::<20>(),           // fee_recipient
        1_000_000u64..50_000_000u64, // gas_limit
        any::<u64>(),                // timestamp
        arb_bytes::<96>(),           // signature
    )
        .prop_map(|(pubkey, fee_recipient, gas_limit, timestamp, signature)| {
            ValidatorRegistration {
                message: ValidatorRegistrationMessage {
                    pubkey: alloy_rpc_types::beacon::BlsPublicKey::from(pubkey),
                    fee_recipient: alloy::primitives::Address::from(fee_recipient),
                    gas_limit,
                    timestamp,
                },
                signature: alloy_rpc_types::beacon::BlsSignature::from(signature),
            }
        })
}

/// Generate arbitrary registration notification
pub fn arb_registration_notification() -> impl Strategy<Value = RegistrationNotification> {
    (arb_validator_registration(), arb_valid_url(), any::<u64>()).prop_map(
        |(registration, relay_url, timestamp)| RegistrationNotification {
            registration,
            relay_url,
            timestamp,
        },
    )
}

/// Generate valid registration notification (passes validation)
pub fn arb_valid_registration_notification() -> impl Strategy<Value = RegistrationNotification> {
    (
        arb_bytes::<48>().prop_filter("not all zeros", |b| b != &[0u8; 48]),
        arb_bytes::<20>().prop_filter("not all zeros", |b| b != &[0u8; 20]),
        1_000_000u64..50_000_000u64, // valid gas_limit range
        arb_bytes::<96>().prop_filter("not all zeros", |b| b != &[0u8; 96]),
        arb_valid_url(),
    )
        .prop_map(|(pubkey, fee_recipient, gas_limit, signature, relay_url)| {
            let current_time = get_current_timestamp().unwrap_or(0);
            let timestamp = current_time.saturating_sub(10); // 10 seconds ago

            RegistrationNotification {
                registration: ValidatorRegistration {
                    message: ValidatorRegistrationMessage {
                        pubkey: alloy_rpc_types::beacon::BlsPublicKey::from(pubkey),
                        fee_recipient: alloy::primitives::Address::from(fee_recipient),
                        gas_limit,
                        timestamp,
                    },
                    signature: alloy_rpc_types::beacon::BlsSignature::from(signature),
                },
                relay_url,
                timestamp,
            }
        })
}

/// Generate arbitrary XGA config
pub fn arb_xga_config() -> impl Strategy<Value = XGAConfig> {
    (
        1u64..86400,    // max_registration_age_secs (1 sec to 1 day)
        1u64..3600,     // polling_interval_secs (1 sec to 1 hour)
        prop::collection::vec(arb_valid_url(), 0..10), // xga_relays
        any::<bool>(),  // probe_relay_capabilities
    )
        .prop_map(|(age, polling_interval, relays, probe)| XGAConfig {
            max_registration_age_secs: age,
            polling_interval_secs: polling_interval,
            xga_relays: relays,
            probe_relay_capabilities: probe,
            retry_config: Default::default(),
            eigenlayer: Default::default(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::parse_and_validate_url;

    proptest! {
        #[test]
        fn test_arb_valid_url_generates_valid_urls(url in arb_valid_url()) {
            assert!(parse_and_validate_url(&url).is_ok());
        }

        #[test]
        fn test_arb_invalid_url_generates_invalid_urls(url in arb_invalid_url()) {
            assert!(parse_and_validate_url(&url).is_err());
        }

        #[test]
        fn test_xga_commitment_tree_hash_deterministic(commitment in arb_xga_commitment()) {
            let hash1 = commitment.get_tree_hash_root();
            let hash2 = commitment.get_tree_hash_root();
            assert_eq!(hash1, hash2);
        }

        #[test]
        fn test_registration_hash_deterministic(reg in arb_validator_registration()) {
            let hash1 = XGACommitment::hash_registration(&reg);
            let hash2 = XGACommitment::hash_registration(&reg);
            assert_eq!(hash1, hash2);
        }
    }
}
