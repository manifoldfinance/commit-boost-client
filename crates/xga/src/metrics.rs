use once_cell::sync::Lazy;
use prometheus::{
    register_int_counter_vec_with_registry, register_int_counter_with_registry, IntCounter,
    IntCounterVec, Registry,
};

/// Prometheus registry for XGA metrics
pub static XGA_METRICS_REGISTRY: Lazy<Registry> = Lazy::new(Registry::new);

/// Counter for registrations received via webhook
pub static REGISTRATIONS_RECEIVED: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter_with_registry!(
        "xga_registrations_received_total",
        "Total number of validator registrations received via webhook",
        XGA_METRICS_REGISTRY.clone()
    )
    .expect("Failed to register REGISTRATIONS_RECEIVED metric")
});

/// Counter for commitments queued for processing
pub static COMMITMENTS_QUEUED: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter_with_registry!(
        "xga_commitments_queued_total",
        "Total number of XGA commitments queued for processing",
        XGA_METRICS_REGISTRY.clone()
    )
    .expect("Failed to register COMMITMENTS_QUEUED metric")
});

/// Counter for signatures requested from signer
pub static SIGNATURES_REQUESTED: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter_with_registry!(
        "xga_signatures_requested_total",
        "Total number of signature requests sent to signer",
        XGA_METRICS_REGISTRY.clone()
    )
    .expect("Failed to register SIGNATURES_REQUESTED metric")
});

/// Counter for signatures successfully received
pub static SIGNATURES_RECEIVED: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter_with_registry!(
        "xga_signatures_received_total",
        "Total number of signatures successfully received from signer",
        XGA_METRICS_REGISTRY.clone()
    )
    .expect("Failed to register SIGNATURES_RECEIVED metric")
});

/// Counter for signature errors
pub static SIGNATURE_ERRORS: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter_with_registry!(
        "xga_signature_errors_total",
        "Total number of signature request errors",
        XGA_METRICS_REGISTRY.clone()
    )
    .expect("Failed to register SIGNATURE_ERRORS metric")
});

/// Counter for signature verifications performed
pub static SIGNATURE_VERIFICATIONS: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter_with_registry!(
        "xga_signature_verifications_total",
        "Total number of signature verifications performed",
        XGA_METRICS_REGISTRY.clone()
    )
    .expect("Failed to register SIGNATURE_VERIFICATIONS metric")
});

/// Counter for commitments sent to relays
pub static COMMITMENTS_SENT: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter_with_registry!(
        "xga_commitments_sent_total",
        "Total number of XGA commitments sent to relays",
        XGA_METRICS_REGISTRY.clone()
    )
    .expect("Failed to register COMMITMENTS_SENT metric")
});

/// Counter for commitments accepted by relays
pub static COMMITMENTS_ACCEPTED: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter_with_registry!(
        "xga_commitments_accepted_total",
        "Total number of XGA commitments accepted by relays",
        XGA_METRICS_REGISTRY.clone()
    )
    .expect("Failed to register COMMITMENTS_ACCEPTED metric")
});

/// Counter for commitments rejected by relays
pub static COMMITMENTS_REJECTED: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter_with_registry!(
        "xga_commitments_rejected_total",
        "Total number of XGA commitments rejected by relays",
        XGA_METRICS_REGISTRY.clone()
    )
    .expect("Failed to register COMMITMENTS_REJECTED metric")
});

/// Counter for relay communication errors
pub static RELAY_ERRORS: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter_with_registry!(
        "xga_relay_errors_total",
        "Total number of errors communicating with relays",
        XGA_METRICS_REGISTRY.clone()
    )
    .expect("Failed to register RELAY_ERRORS metric")
});

/// Per-relay metrics for tracking individual relay performance
pub static RELAY_METRICS: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec_with_registry!(
        "xga_relay_operations_total",
        "Total operations per relay",
        &["relay_id", "operation", "status"],
        XGA_METRICS_REGISTRY.clone()
    )
    .expect("Failed to register RELAY_METRICS metric")
});

/// Validator-specific metrics
pub static VALIDATOR_METRICS: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec_with_registry!(
        "xga_validator_operations_total",
        "Total operations per validator",
        &["validator_pubkey", "operation"],
        XGA_METRICS_REGISTRY.clone()
    )
    .expect("Failed to register VALIDATOR_METRICS metric")
});
