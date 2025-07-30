// Re-export public types and functions
pub mod error;
pub use error::Error;

pub mod circuit_breaker;
pub use circuit_breaker::CircuitBreaker;

pub mod rate_limiter;
pub use rate_limiter::RateLimiter;

pub mod http_client;
pub use http_client::HttpClientFactory;

pub mod utils;
pub use utils::{get_current_timestamp, parse_and_validate_url, MAX_REQUEST_SIZE};
