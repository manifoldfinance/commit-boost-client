use eyre::{Result, WrapErr};

use super::error::Error;

/// Request body size limit (1MB)
pub const MAX_REQUEST_SIZE: usize = 1024 * 1024;

/// Safe URL parsing and validation
///
/// # Errors
///
/// Returns an error if:
/// - The URL cannot be parsed
/// - The URL scheme is not HTTPS
/// - The URL points to a local address
pub fn parse_and_validate_url(url: &str) -> Result<url::Url> {
    let parsed = url::Url::parse(url)
        .map_err(|e| Error::InvalidUrl(format!("Failed to parse URL '{url}': {e}")))?;

    // Validate scheme
    if parsed.scheme() != "https" {
        return Err(Error::InvalidUrl(format!("URL '{url}' must use HTTPS scheme")).into());
    }

    // Validate host exists
    let host =
        parsed.host_str().ok_or_else(|| Error::InvalidUrl(format!("URL '{url}' has no host")))?;

    // Reject localhost and local IPs
    if is_local_address(host) {
        return Err(
            Error::InvalidUrl(format!("URL '{url}' must not point to local addresses")).into()
        );
    }

    Ok(parsed)
}

/// Check if a host address is local
#[must_use]
fn is_local_address(host: &str) -> bool {
    matches!(
        host,
        "localhost" | "127.0.0.1" | "0.0.0.0" | "::1" | "[::1]"
    ) || host.starts_with("192.168.")
      || host.starts_with("10.")
      || is_private_172_range(host)
      || host.starts_with("169.254.")  // Link-local
      || host.starts_with("fc00::")    // IPv6 unique local
      || host.starts_with("fe80::") // IPv6 link-local
}

/// Check if host is in the 172.16.0.0/12 private range
#[must_use]
fn is_private_172_range(host: &str) -> bool {
    if let Some(rest) = host.strip_prefix("172.") {
        if let Some(second_octet) = rest.split('.').next() {
            if let Ok(num) = second_octet.parse::<u8>() {
                return (16..=31).contains(&num);
            }
        }
    }
    false
}

/// Get current timestamp with proper error handling
///
/// # Errors
///
/// Returns an error if the system time is before UNIX epoch
pub fn get_current_timestamp() -> Result<u64> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .wrap_err("Failed to get current timestamp")
}

/// Constant-time comparison for security-sensitive operations
#[must_use]
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    constant_time_eq::constant_time_eq(a, b)
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_url_validation() {
        // Valid URLs
        assert!(parse_and_validate_url("https://relay.example.com").is_ok());
        assert!(parse_and_validate_url("https://relay.example.com:8080").is_ok());

        // Invalid URLs
        assert!(parse_and_validate_url("http://relay.example.com").is_err());
        assert!(parse_and_validate_url("https://localhost").is_err());
        assert!(parse_and_validate_url("https://127.0.0.1").is_err());
        assert!(parse_and_validate_url("https://192.168.1.1").is_err());
        assert!(parse_and_validate_url("not-a-url").is_err());
    }

    #[test]
    fn test_private_172_range_detection() {
        assert!(is_private_172_range("172.16.0.1"));
        assert!(is_private_172_range("172.20.10.5"));
        assert!(is_private_172_range("172.31.255.255"));
        assert!(!is_private_172_range("172.15.0.1"));
        assert!(!is_private_172_range("172.32.0.1"));
        assert!(!is_private_172_range("173.16.0.1"));
    }

    #[test]
    fn test_constant_time_comparison() {
        let a = b"hello";
        let b = b"hello";
        let c = b"world";
        let d = b"hell";

        assert!(constant_time_eq(a, b));
        assert!(!constant_time_eq(a, c));
        assert!(!constant_time_eq(a, d));
    }

    #[test]
    fn test_get_current_timestamp() {
        let timestamp = get_current_timestamp().unwrap();
        assert!(timestamp > 0);

        // Verify it's a reasonable timestamp (after year 2020)
        assert!(timestamp > 1_577_836_800); // Jan 1, 2020
    }
}
