use crate::config::RelayGasConfig;
use crate::error::{ReservedGasError, Result};

/// Maximum reasonable gas limit (30M gas)
pub const MAX_REASONABLE_GAS: u64 = 30_000_000;

/// Minimum block gas limit to enforce (1M gas)
pub const MIN_BLOCK_GAS_LIMIT: u64 = 1_000_000;

/// Minimum gas limit after reservation (10M gas)
pub const MIN_GAS_AFTER_RESERVATION: u64 = 10_000_000;

/// Validate a gas limit value
pub fn validate_gas_limit(gas_limit: u64, context: &str) -> Result<()> {
    if gas_limit == 0 {
        return Err(ReservedGasError::ConfigError(format!(
            "{}: gas limit cannot be zero",
            context
        )));
    }

    if gas_limit > MAX_REASONABLE_GAS {
        return Err(ReservedGasError::ReservedGasTooHigh {
            amount: gas_limit,
            max: MAX_REASONABLE_GAS,
        });
    }

    Ok(())
}

/// Validate a relay gas configuration
pub fn validate_relay_gas_config(config: &RelayGasConfig, relay_id: &str) -> Result<()> {
    // Validate reserved gas limit
    validate_gas_limit(config.reserved_gas_limit, "reserved gas limit")?;

    // Validate minimum gas limit if provided
    if let Some(min_gas) = config.min_gas_limit {
        if min_gas < MIN_BLOCK_GAS_LIMIT {
            return Err(ReservedGasError::InvalidRelayConfig {
                relay_id: relay_id.to_string(),
                reason: format!(
                    "minimum gas limit {} is below threshold {}",
                    min_gas, MIN_BLOCK_GAS_LIMIT
                ),
            });
        }
    }

    Ok(())
}

/// Validate that gas limit after reservation meets requirements
pub fn validate_gas_after_reservation(
    original_gas_limit: u64,
    reserved_gas: u64,
    min_required: u64,
) -> Result<u64> {
    let new_gas_limit = original_gas_limit.saturating_sub(reserved_gas);

    if new_gas_limit < min_required {
        return Err(ReservedGasError::GasLimitTooLow {
            actual: new_gas_limit,
            minimum: min_required,
            reserved: reserved_gas,
        });
    }

    Ok(new_gas_limit)
}

/// Check if a relay response is valid for processing
pub fn is_valid_relay_response(config: &RelayGasConfig, _relay_id: &str) -> bool {
    // Quick validation without returning errors
    if config.reserved_gas_limit == 0 || config.reserved_gas_limit > MAX_REASONABLE_GAS {
        return false;
    }

    if let Some(min_gas) = config.min_gas_limit {
        if min_gas < MIN_BLOCK_GAS_LIMIT {
            return false;
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_gas_limit() {
        assert!(validate_gas_limit(1_000_000, "test").is_ok());
        assert!(validate_gas_limit(0, "test").is_err());
        assert!(validate_gas_limit(40_000_000, "test").is_err());
    }

    #[test]
    fn test_validate_gas_after_reservation() {
        assert_eq!(
            validate_gas_after_reservation(15_000_000, 5_000_000, 10_000_000).unwrap(),
            10_000_000
        );

        assert!(validate_gas_after_reservation(10_000_000, 5_000_000, 10_000_000).is_err());
    }
}
