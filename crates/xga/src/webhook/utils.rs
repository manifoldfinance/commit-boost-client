/// Extract relay ID from URL
/// This extracts the hostname (and port if present) from a URL to use as a
/// relay identifier
pub fn extract_relay_id(url: &str) -> String {
    // Simple extraction - take the hostname
    // In production, this should be more sophisticated
    url.split("://").nth(1).and_then(|s| s.split('/').next()).unwrap_or(url).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_relay_id() {
        assert_eq!(extract_relay_id("https://relay.example.com/api"), "relay.example.com");
        assert_eq!(extract_relay_id("http://localhost:8080"), "localhost:8080");
        assert_eq!(
            extract_relay_id("https://relay.example.com:9000/v1/relay"),
            "relay.example.com:9000"
        );
        assert_eq!(extract_relay_id("relay.example.com"), "relay.example.com");
    }
}
