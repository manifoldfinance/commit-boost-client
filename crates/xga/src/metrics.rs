use lazy_static::lazy_static;
use prometheus::{register_histogram_vec, register_int_gauge_vec, HistogramVec, IntGaugeVec};

lazy_static! {
    /// Gas reservation amounts by relay
    pub static ref GAS_RESERVATION_AMOUNT: IntGaugeVec = register_int_gauge_vec!(
        "cb_reserved_gas_amount",
        "Current gas reservation amount for each relay",
        &["relay_id"]
    )
    .expect("Failed to register gas reservation amount metric");

    /// Gas reservation applied count
    pub static ref GAS_RESERVATION_APPLIED: HistogramVec = register_histogram_vec!(
        "cb_reserved_gas_applied",
        "Gas reservations applied to headers",
        &["relay_id"],
        vec![100_000.0, 500_000.0, 1_000_000.0, 2_000_000.0, 5_000_000.0]
    )
    .expect("Failed to register gas reservation applied metric");

    /// Gas reservation configuration updates
    pub static ref GAS_CONFIG_UPDATES: IntGaugeVec = register_int_gauge_vec!(
        "cb_reserved_gas_config_updates_total",
        "Total number of gas configuration updates from relays",
        &["relay_id"]
    )
    .expect("Failed to register gas config updates metric");
}

/// Initialize all metrics and return them for registration
pub fn init_metrics() -> Vec<Box<dyn prometheus::core::Collector>> {
    // Force lazy_static initialization and collect metrics
    vec![
        Box::new(GAS_RESERVATION_AMOUNT.clone()),
        Box::new(GAS_RESERVATION_APPLIED.clone()),
        Box::new(GAS_CONFIG_UPDATES.clone()),
    ]
}
