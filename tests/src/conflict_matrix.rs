use eyre::{bail, Result};
use std::collections::HashSet;

/// Simple conflict matrix for module compatibility checking
pub struct ConflictMatrix {
    incompatible_pairs: HashSet<(String, String)>,
}

impl ConflictMatrix {
    pub fn new() -> Self {
        let mut incompatible_pairs = HashSet::new();

        // Define known incompatible module pairs
        incompatible_pairs.insert(("pbs_relay_a".into(), "pbs_relay_b".into()));
        incompatible_pairs.insert(("custom_pbs".into(), "pbs".into()));
        incompatible_pairs.insert(("duplicate_signer".into(), "signer".into()));

        Self { incompatible_pairs }
    }

    /// Check if two modules can coexist in the same configuration
    pub fn can_coexist(&self, module_a: &str, module_b: &str) -> bool {
        // Check both orderings since the set might only have one direction
        !self.incompatible_pairs.contains(&(module_a.into(), module_b.into()))
            && !self.incompatible_pairs.contains(&(module_b.into(), module_a.into()))
    }

    /// Validate a list of modules for conflicts
    pub fn validate_module_list(&self, module_ids: &[String]) -> Result<()> {
        // Check for duplicates first
        let mut seen = HashSet::new();
        for module_id in module_ids {
            if !seen.insert(module_id) {
                bail!("Duplicate module ID: {}", module_id);
            }
        }

        // Check for incompatible pairs
        for i in 0..module_ids.len() {
            for j in i + 1..module_ids.len() {
                let module_a = &module_ids[i];
                let module_b = &module_ids[j];

                if !self.can_coexist(module_a, module_b) {
                    bail!("Incompatible modules: '{}' and '{}'", module_a, module_b);
                }
            }
        }

        Ok(())
    }
}

impl Default for ConflictMatrix {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_conflict_matrix_basic() {
        let matrix = ConflictMatrix::new();

        // Test known compatible pairs
        assert!(matrix.can_coexist("pbs", "signer"));
        assert!(matrix.can_coexist("signer", "pbs")); // Symmetric

        // Test known incompatible pairs
        assert!(!matrix.can_coexist("pbs_relay_a", "pbs_relay_b"));
        assert!(!matrix.can_coexist("pbs_relay_b", "pbs_relay_a")); // Symmetric
    }

    #[test]
    fn test_validate_module_list() {
        let matrix = ConflictMatrix::new();

        // Valid configuration
        let valid_modules = vec!["pbs".to_string(), "signer".to_string()];
        assert!(matrix.validate_module_list(&valid_modules).is_ok());

        // Invalid configuration - duplicates
        let duplicate_modules = vec!["pbs".to_string(), "pbs".to_string()];
        assert!(matrix.validate_module_list(&duplicate_modules).is_err());

        // Invalid configuration - conflicts
        let conflict_modules = vec!["pbs_relay_a".to_string(), "pbs_relay_b".to_string()];
        assert!(matrix.validate_module_list(&conflict_modules).is_err());
    }
}
