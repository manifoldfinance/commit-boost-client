use std::time::Duration;

use eyre::{Result, WrapErr};
use reqwest::{Client, ClientBuilder};

/// HTTP client factory with connection pooling and timeouts
pub struct HttpClientFactory {
    default_timeout: Duration,
    max_idle_per_host: usize,
}

impl HttpClientFactory {
    /// Create a new HTTP client factory with default settings
    pub fn new() -> Self {
        Self { default_timeout: Duration::from_secs(10), max_idle_per_host: 10 }
    }

    /// Create a new HTTP client with connection pooling
    pub fn create_client(&self) -> Result<Client> {
        ClientBuilder::new()
            .pool_max_idle_per_host(self.max_idle_per_host)
            .timeout(self.default_timeout)
            .build()
            .wrap_err("Failed to create HTTP client")
    }

    /// Create a client with custom timeout
    pub fn create_client_with_timeout(&self, timeout: Duration) -> Result<Client> {
        ClientBuilder::new()
            .pool_max_idle_per_host(self.max_idle_per_host)
            .timeout(timeout)
            .build()
            .wrap_err("Failed to create HTTP client with custom timeout")
    }

    /// Create a client with custom configuration
    pub fn create_client_with_config<F>(&self, config_fn: F) -> Result<Client>
    where
        F: FnOnce(ClientBuilder) -> ClientBuilder,
    {
        let builder = ClientBuilder::new()
            .pool_max_idle_per_host(self.max_idle_per_host)
            .timeout(self.default_timeout);

        config_fn(builder).build().wrap_err("Failed to create HTTP client with custom config")
    }
}

impl Default for HttpClientFactory {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_factory_creation() {
        let factory = HttpClientFactory::new();
        assert!(factory.create_client().is_ok());
    }

    #[test]
    fn test_client_with_custom_timeout() {
        let factory = HttpClientFactory::new();
        let client = factory.create_client_with_timeout(Duration::from_secs(30));
        assert!(client.is_ok());
    }

    #[test]
    fn test_client_with_custom_config() {
        let factory = HttpClientFactory::new();
        let client = factory.create_client_with_config(|builder| {
            builder.pool_max_idle_per_host(20).connect_timeout(Duration::from_secs(5))
        });
        assert!(client.is_ok());
    }
}
