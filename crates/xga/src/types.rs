use std::sync::Arc;

/// Type alias for relay ID
pub type RelayId = Arc<String>;

/// Type alias for gas amount in wei
pub type GasAmount = u64;

/// Represents a bid from a relay with gas reservation applied
#[derive(Debug, Clone)]
pub struct AdjustedBid {
    pub response: cb_common::pbs::GetHeaderResponse,
    pub original_gas_limit: GasAmount,
    pub reserved_gas: GasAmount,
    pub relay_id: RelayId,
}

/// Result of querying a single relay
#[derive(Debug)]
pub enum RelayQueryResult {
    Bid(AdjustedBid),
    NoBid,
    Error(String),
}

/// Represents the outcome of gas reservation application
///
/// This struct contains the results of applying a gas reservation to a block,
/// including the new gas limit after reservation and the amount that was reserved.
///
/// # Example
///
/// ```ignore
/// let outcome = state.apply_reservation("flashbots", 30_000_000)?;
/// assert_eq!(outcome.new_gas_limit, 28_000_000);
/// assert_eq!(outcome.reserved_amount, 2_000_000);
/// ```
#[derive(Debug, Clone)]
pub struct GasReservationOutcome {
    /// The new gas limit after applying the reservation
    pub new_gas_limit: GasAmount,
    /// The amount of gas that was reserved
    pub reserved_amount: GasAmount,
    /// The ID of the relay this reservation is for
    pub relay_id: String,
}

/// Relay configuration update event
///
/// Tracks changes to relay gas reservation configurations, including
/// the previous reservation amount (if any) and the new amount.
///
/// # Example
///
/// ```ignore
/// let event = service.update_relay_configuration(
///     "flashbots".to_string(),
///     RelayGasConfig {
///         reserved_gas_limit: 2_000_000,
///         min_gas_limit: Some(15_000_000),
///         update_interval: None,
///     }
/// );
/// println!("Updated {} from {:?} to {}",
///     event.relay_id, event.old_reservation, event.new_reservation);
/// ```
#[derive(Debug, Clone)]
pub struct ConfigUpdateEvent {
    /// The ID of the relay whose configuration was updated
    pub relay_id: String,
    /// The previous gas reservation amount, if any
    pub old_reservation: Option<GasAmount>,
    /// The new gas reservation amount
    pub new_reservation: GasAmount,
    /// When this update occurred
    pub timestamp: std::time::Instant,
}

/// Information about a relay's reserve requirement
#[derive(Debug, Clone, serde::Serialize)]
pub struct RelayReserveInfo {
    /// The ID of the relay
    pub relay_id: String,
    /// The minimum gas capacity that must be reserved (if query succeeded)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reserve: Option<u64>,
    /// Time taken to query the relay in milliseconds
    pub query_time_ms: u64,
    /// Status of the query
    pub status: ReserveQueryStatus,
    /// Error message if query failed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Status of a reserve query to a relay
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ReserveQueryStatus {
    Success,
    Error,
    Timeout,
}
