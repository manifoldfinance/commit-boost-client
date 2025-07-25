use thiserror::Error;

#[derive(Error, Debug)]
pub enum ReservedGasError {
    #[error("Invalid gas configuration from relay {relay_id}: {reason}")]
    InvalidRelayConfig { relay_id: String, reason: String },

    #[error("Gas limit {actual} would be below minimum {minimum} after reserving {reserved}")]
    GasLimitTooLow { actual: u64, minimum: u64, reserved: u64 },

    #[error("Reserved gas amount {amount} exceeds reasonable limit of {max}")]
    ReservedGasTooHigh { amount: u64, max: u64 },

    #[error("Relay communication failed: {0}")]
    RelayError(#[from] reqwest::Error),

    #[error("Relay connection error for {relay_id}: {message}")]
    RelayConnectionError { relay_id: String, message: String },

    #[error("No healthy relays available")]
    NoHealthyRelays,

    #[error("Failed to register validators with any relay")]
    ValidatorRegistrationFailed,

    #[error("Failed to submit block to any relay")]
    BlockSubmissionFailed,

    #[error("Configuration error: {0}")]
    ConfigError(String),

    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, ReservedGasError>;
