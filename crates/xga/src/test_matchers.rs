//! Domain-specific matchers for reserved gas testing with googletest

#[cfg(test)]
pub mod matchers {
    use googletest::prelude::*;

    /// Matcher for checking if gas amount represents a healthy relay
    pub fn is_healthy_relay() -> impl Matcher<ActualT = u64> {
        le(10_000_000u64)
    }

    /// Matcher for checking if gas amount represents a degraded relay
    pub fn is_degraded_relay() -> impl Matcher<ActualT = u64> {
        all!(gt(10_000_000u64), le(20_000_000u64))
    }

    /// Matcher for checking if gas amount represents an unhealthy relay
    pub fn is_unhealthy_relay() -> impl Matcher<ActualT = u64> {
        gt(20_000_000u64)
    }
}

#[cfg(test)]
mod tests {
    use super::matchers::*;
    use googletest::prelude::*;

    #[test]
    fn test_relay_health_matchers() -> Result<()> {
        // Healthy relay
        assert_that!(5_000_000u64, is_healthy_relay());

        // Degraded relay
        assert_that!(15_000_000u64, is_degraded_relay());

        // Unhealthy relay
        assert_that!(25_000_000u64, is_unhealthy_relay());

        Ok(())
    }
}
