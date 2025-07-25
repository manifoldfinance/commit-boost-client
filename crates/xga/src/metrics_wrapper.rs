use prometheus::{HistogramVec, IntGaugeVec};
use std::sync::Arc;

/// Wrapper for metrics to allow for easier testing and dependency injection
pub struct MetricsWrapper {
    pub gas_reservation_amount: IntGaugeVec,
    pub gas_reservation_applied: HistogramVec,
    pub gas_config_updates: IntGaugeVec,
}

impl MetricsWrapper {
    /// Create a new metrics wrapper with production metrics
    pub fn new() -> Self {
        Self {
            gas_reservation_amount: crate::metrics::GAS_RESERVATION_AMOUNT.clone(),
            gas_reservation_applied: crate::metrics::GAS_RESERVATION_APPLIED.clone(),
            gas_config_updates: crate::metrics::GAS_CONFIG_UPDATES.clone(),
        }
    }

    /// Record a gas reservation amount update
    pub fn record_gas_reservation_amount(&self, relay_id: &str, amount: u64) {
        self.gas_reservation_amount.with_label_values(&[relay_id]).set(amount as i64);
    }

    /// Record a gas reservation application
    pub fn record_gas_reservation_applied(&self, relay_id: &str, amount: u64) {
        self.gas_reservation_applied.with_label_values(&[relay_id]).observe(amount as f64);
    }

    /// Record a gas config update
    pub fn record_gas_config_update(&self, relay_id: &str) {
        self.gas_config_updates.with_label_values(&[relay_id]).inc();
    }
}

/// Trait for abstracting metrics operations
pub trait MetricsRecorder: Send + Sync {
    fn record_gas_reservation_amount(&self, relay_id: &str, amount: u64);
    fn record_gas_reservation_applied(&self, relay_id: &str, amount: u64);
    fn record_gas_config_update(&self, relay_id: &str);
}

impl MetricsRecorder for MetricsWrapper {
    fn record_gas_reservation_amount(&self, relay_id: &str, amount: u64) {
        self.record_gas_reservation_amount(relay_id, amount);
    }

    fn record_gas_reservation_applied(&self, relay_id: &str, amount: u64) {
        self.record_gas_reservation_applied(relay_id, amount);
    }

    fn record_gas_config_update(&self, relay_id: &str) {
        self.record_gas_config_update(relay_id);
    }
}

/// No-op implementation for testing
pub struct NoOpMetricsRecorder;

impl MetricsRecorder for NoOpMetricsRecorder {
    fn record_gas_reservation_amount(&self, _relay_id: &str, _amount: u64) {}
    fn record_gas_reservation_applied(&self, _relay_id: &str, _amount: u64) {}
    fn record_gas_config_update(&self, _relay_id: &str) {}
}

/// Create a shared metrics recorder
pub fn create_metrics_recorder(use_noop: bool) -> Arc<dyn MetricsRecorder> {
    if use_noop {
        Arc::new(NoOpMetricsRecorder)
    } else {
        Arc::new(MetricsWrapper::new())
    }
}
