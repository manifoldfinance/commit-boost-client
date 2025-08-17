//! Metrics for PBS module
//! We collect two types of metrics within the PBS module:
//! - what PBS receives from relays
//! - what PBS returns to the beacon node

use lazy_static::lazy_static;
use prometheus::{
    register_histogram_vec_with_registry, register_int_counter_vec_with_registry,
    register_int_gauge_vec_with_registry, register_int_counter_with_registry, register_int_gauge_with_registry,
    HistogramVec, IntCounterVec, IntGaugeVec, IntCounter, IntGauge, Registry,
};

lazy_static! {
    pub static ref PBS_METRICS_REGISTRY: Registry =
        Registry::new_custom(Some("cb_pbs".to_string()), None).unwrap();

    // FROM RELAYS
    /// Status code received by relay by endpoint
    pub static ref RELAY_STATUS_CODE: IntCounterVec = register_int_counter_vec_with_registry!(
        "relay_status_code_total",
        "HTTP status code received by relay",
        &["http_status_code", "endpoint", "relay_id"],
        PBS_METRICS_REGISTRY
    )
    .unwrap();

    /// Latency by relay by endpoint
    pub static ref RELAY_LATENCY: HistogramVec = register_histogram_vec_with_registry!(
        "relay_latency",
        "HTTP latency by relay",
        &["endpoint", "relay_id"],
        PBS_METRICS_REGISTRY
    )
    .unwrap();

    /// Latest slot for which relay delivered a header
    pub static ref RELAY_LAST_SLOT: IntGaugeVec = register_int_gauge_vec_with_registry!(
        "relay_last_slot",
        "Latest slot for which relay delivered a header",
        &["relay_id"],
        PBS_METRICS_REGISTRY
    )
    .unwrap();

    /// Latest slot for which relay delivered a header
    // Don't store slot number to avoid creating high cardinality, if needed can just aggregate for 12sec
    pub static ref RELAY_HEADER_VALUE: IntGaugeVec = register_int_gauge_vec_with_registry!(
        "relay_header_value",
        "Header value in gwei delivered by relay",
        &["relay_id"],
        PBS_METRICS_REGISTRY
    )
    .unwrap();


    // TO BEACON NODE
    /// Status code returned to beacon node by endpoint
    pub static ref BEACON_NODE_STATUS: IntCounterVec = register_int_counter_vec_with_registry!(
        "beacon_node_status_code_total",
        "HTTP status code returned to beacon node",
        &["http_status_code", "endpoint"],
        PBS_METRICS_REGISTRY
    ).unwrap();

    // REGISTRATION CACHE METRICS
    /// Number of registration cache hits
    pub static ref REGISTRATION_CACHE_HITS: IntCounter = register_int_counter_with_registry!(
        "registration_cache_hits_total",
        "Number of registration cache hits",
        PBS_METRICS_REGISTRY
    ).unwrap();

    /// Number of registration cache misses
    pub static ref REGISTRATION_CACHE_MISSES: IntCounter = register_int_counter_with_registry!(
        "registration_cache_misses_total", 
        "Number of registration cache misses",
        PBS_METRICS_REGISTRY
    ).unwrap();

    /// Current size of registration cache
    pub static ref REGISTRATION_CACHE_SIZE: IntGauge = register_int_gauge_with_registry!(
        "registration_cache_size",
        "Current size of registration cache",
        PBS_METRICS_REGISTRY
    ).unwrap();

    /// Number of registrations skipped due to cache
    pub static ref REGISTRATIONS_SKIPPED: IntCounterVec = register_int_counter_vec_with_registry!(
        "registrations_skipped_total",
        "Number of registrations skipped due to cache",
        &["reason"],
        PBS_METRICS_REGISTRY
    ).unwrap();

    /// Number of registrations processed
    pub static ref REGISTRATIONS_PROCESSED: IntCounterVec = register_int_counter_vec_with_registry!(
        "registrations_processed_total",
        "Number of registrations processed",
        &["status"],
        PBS_METRICS_REGISTRY
    ).unwrap();
}
