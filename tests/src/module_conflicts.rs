#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use crate::utils::get_default_config;
    use crate::conflict_matrix::ConflictMatrix;

    #[test]
    fn test_module_id_uniqueness() {
        let config = get_default_config();
        let mut seen_ids = HashSet::new();
        
        if let Some(modules) = config.modules {
            for module in modules {
                assert!(
                    seen_ids.insert(module.id.clone()),
                    "Duplicate module ID: {}", module.id
                );
            }
        }
    }

    #[test]
    fn test_port_conflicts() {
        let config = get_default_config();
        let mut used_ports = HashSet::new();
        
        // Check PBS port
        assert!(
            used_ports.insert(config.pbs.pbs_config.port),
            "PBS port {} conflicts with an existing service", config.pbs.pbs_config.port
        );
        
        // Check signer port if exists
        if let Some(signer) = config.signer {
            assert!(
                used_ports.insert(signer.port),
                "Signer port {} already in use", signer.port
            );
        }
        
        // Check metrics ports if exists
        if let Some(metrics) = config.metrics {
            assert!(
                used_ports.insert(metrics.start_port),
                "Metrics port {} already in use", metrics.start_port
            );
        }
    }

    #[test]
    fn test_metric_namespace_conflicts() {
        // Each module should prefix metrics with cb_{module_name}_
        let known_modules = vec![
            ("pbs", "cb_pbs_"),
            ("signer", "cb_signer_"),
            ("metrics", "cb_metrics_"),
        ];
        
        for (module_a, prefix_a) in &known_modules {
            for (module_b, prefix_b) in &known_modules {
                if module_a != module_b {
                    assert!(
                        !prefix_a.starts_with(prefix_b) || prefix_a == prefix_b,
                        "Metric prefix conflict between {} and {}: '{}' vs '{}'", 
                        module_a, module_b, prefix_a, prefix_b
                    );
                }
            }
        }
    }

    #[test]
    fn test_integration_matrix_validation() {
        let config = get_default_config();
        let matrix = ConflictMatrix::new();
        
        if let Some(modules) = config.modules {
            let module_ids: Vec<String> = modules.iter()
                .map(|m| m.id.to_string())
                .collect();
            
            // Use the matrix to validate all module combinations
            if let Err(error) = matrix.validate_module_list(&module_ids) {
                panic!("Module conflict detected in configuration: {}", error);
            }
        }
    }

    #[test]
    fn test_known_incompatible_combinations() {
        let matrix = ConflictMatrix::new();
        
        // Test specific incompatible combinations
        let incompatible_pairs = vec![
            ("pbs_relay_a", "pbs_relay_b"),
            ("custom_pbs", "pbs"),
            ("duplicate_signer", "signer"),
        ];
        
        for (module_a, module_b) in incompatible_pairs {
            assert!(
                !matrix.can_coexist(module_a, module_b),
                "Modules '{}' and '{}' should be incompatible", module_a, module_b
            );
        }
    }

    #[test]
    fn test_known_compatible_combinations() {
        let matrix = ConflictMatrix::new();
        
        // Test specific compatible combinations
        let compatible_pairs = vec![
            ("pbs", "signer"),
            ("pbs", "metrics"),
            ("signer", "metrics"),
        ];
        
        for (module_a, module_b) in compatible_pairs {
            assert!(
                matrix.can_coexist(module_a, module_b),
                "Modules '{}' and '{}' should be compatible", module_a, module_b
            );
        }
    }

    #[test]
    fn test_conflict_matrix_symmetry() {
        let matrix = ConflictMatrix::new();
        
        // Test that compatibility is symmetric (A compatible with B implies B compatible with A)
        let test_pairs = vec![
            ("pbs", "signer"),
            ("pbs_relay_a", "pbs_relay_b"),
            ("custom_pbs", "pbs"),
        ];
        
        for (module_a, module_b) in test_pairs {
            let a_to_b = matrix.can_coexist(module_a, module_b);
            let b_to_a = matrix.can_coexist(module_b, module_a);
            
            assert_eq!(
                a_to_b, b_to_a,
                "Compatibility should be symmetric: {} <-> {}", module_a, module_b
            );
        }
    }

    #[test]
    fn test_empty_module_configuration() {
        let matrix = ConflictMatrix::new();
        
        // Empty module list should always be valid
        let result = matrix.validate_module_list(&[]);
        assert!(result.is_ok(), "Empty module list should be valid");
    }

    #[test]
    fn test_single_module_configuration() {
        let matrix = ConflictMatrix::new();
        
        // Single module should always be valid
        let single_modules = vec![
            vec!["pbs".to_string()],
            vec!["signer".to_string()],
            vec!["metrics".to_string()],
        ];
        
        for module_list in single_modules {
            let result = matrix.validate_module_list(&module_list);
            assert!(
                result.is_ok(), 
                "Single module '{}' should be valid", module_list[0]
            );
        }
    }
}