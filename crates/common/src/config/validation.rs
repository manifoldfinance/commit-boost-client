use eyre::{bail, Result};
use std::collections::HashSet;

use super::CommitBoostConfig;

/// Validate configuration for module conflicts
pub fn validate_no_conflicts(config: &CommitBoostConfig) -> Result<()> {
    let mut module_ids = HashSet::new();
    let mut ports = HashSet::new();

    // Check module IDs for uniqueness
    if let Some(modules) = &config.modules {
        for module in modules {
            if !module_ids.insert(&module.id) {
                bail!("Duplicate module ID: {}", module.id);
            }
        }

        // Check module compatibility
        for i in 0..modules.len() {
            for j in i + 1..modules.len() {
                let module_a = modules[i].id.as_str();
                let module_b = modules[j].id.as_str();

                // Basic incompatibility checks
                if is_incompatible(module_a, module_b) {
                    bail!("Incompatible modules: '{}' and '{}'", module_a, module_b);
                }
            }
        }
    }

    // Check port conflicts
    if !ports.insert(config.pbs.pbs_config.port) {
        bail!("Port conflict: PBS port {} already in use", config.pbs.pbs_config.port);
    }

    if let Some(signer) = &config.signer {
        if !ports.insert(signer.port) {
            bail!("Port conflict: Signer port {} already in use", signer.port);
        }
    }

    if let Some(metrics) = &config.metrics {
        if !ports.insert(metrics.start_port) {
            bail!("Port conflict: Metrics port {} already in use", metrics.start_port);
        }
    }

    Ok(())
}

/// Check if two modules are known to be incompatible
fn is_incompatible(module_a: &str, module_b: &str) -> bool {
    let incompatible_pairs =
        [("pbs_relay_a", "pbs_relay_b"), ("custom_pbs", "pbs"), ("duplicate_signer", "signer")];

    for (a, b) in &incompatible_pairs {
        if (module_a == *a && module_b == *b) || (module_a == *b && module_b == *a) {
            return true;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_incompatible() {
        assert!(is_incompatible("pbs_relay_a", "pbs_relay_b"));
        assert!(is_incompatible("pbs_relay_b", "pbs_relay_a")); // Symmetric
        assert!(is_incompatible("custom_pbs", "pbs"));
        assert!(is_incompatible("pbs", "custom_pbs")); // Symmetric

        // Compatible pairs should return false
        assert!(!is_incompatible("pbs", "signer"));
        assert!(!is_incompatible("signer", "metrics"));
    }
}
