use alloy::{
    primitives::{Address, Bytes, FixedBytes, B256, U256},
    sol_types::SolValue,
};
use eyre::Result;

// Function selectors for XGARegistry contract
pub const OPERATORS_SELECTOR: [u8; 4] = [0x13, 0x71, 0x25, 0x23]; // operators(address)
pub const GET_PENDING_REWARDS_SELECTOR: [u8; 4] = [0x6a, 0x27, 0xb3, 0x0b]; // getPendingRewards(address)
pub const PENALTY_RATES_SELECTOR: [u8; 4] = [0x0e, 0x39, 0x8a, 0x8b]; // penaltyRates(address)

/// Encode a function call with an address parameter
pub fn encode_address_call(selector: [u8; 4], address: Address) -> Bytes {
    let mut data = Vec::with_capacity(36);
    data.extend_from_slice(&selector);
    data.extend_from_slice(&address.abi_encode());
    Bytes::from(data)
}

/// Decode a uint256 result from contract call
pub fn decode_uint256(data: Bytes) -> Result<U256> {
    if data.len() != 32 {
        return Err(eyre::eyre!("Invalid data length for uint256: {}", data.len()));
    }
    Ok(U256::abi_decode(&data, true)?)
}

/// Decode the operators function return value
pub fn decode_operator_data(data: Bytes) -> Result<(B256, Bytes, U256, U256, bool, U256)> {
    // The operators function returns a tuple of:
    // (bytes32 commitmentHash, bytes signature, uint256 registrationBlock,
    //  uint256 lastRewardBlock, bool isActive, uint256 accumulatedRewards)

    if data.len() < 192 {
        // Minimum size for fixed parts
        return Err(eyre::eyre!("Invalid data length for operator data: {}", data.len()));
    }

    // Decode as a tuple
    type OperatorTuple = (FixedBytes<32>, Bytes, U256, U256, bool, U256);
    let decoded = OperatorTuple::abi_decode(&data, true)?;

    Ok((B256::from(decoded.0), decoded.1, decoded.2, decoded.3, decoded.4, decoded.5))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_address_call() {
        let address = Address::from([0x42; 20]);
        let encoded = encode_address_call(OPERATORS_SELECTOR, address);

        // Check selector
        assert_eq!(&encoded[0..4], &OPERATORS_SELECTOR);

        // Check address is padded to 32 bytes
        assert_eq!(encoded.len(), 36);
        assert_eq!(&encoded[16..36], address.as_slice());
    }

    #[test]
    fn test_decode_uint256() {
        let value = U256::from(12345u64);
        let encoded = value.abi_encode();
        let decoded = decode_uint256(Bytes::from(encoded)).unwrap();
        assert_eq!(decoded, value);
    }
}
