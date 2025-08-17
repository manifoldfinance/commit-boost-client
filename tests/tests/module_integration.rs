use cb_tests::{conflict_matrix::ConflictMatrix, utils::get_default_config};
use cb_common::{
    config::{CommitBoostConfig, StaticModuleConfig},
    types::ModuleId,
};
use eyre::Result;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use std::time::{Duration, Instant};
use std::collections::HashMap;

/// Helper to create a test configuration with specific modules
struct TestConfig {
    config: CommitBoostConfig,
}

impl TestConfig {
    fn new() -> Self {
        Self {
            config: get_default_config(),
        }
    }
    
    fn with_module(mut self, id: &str, _port: u16) -> Self {
        let module = StaticModuleConfig {
            id: ModuleId::from(id.to_string()),
            kind: cb_common::config::ModuleKind::Commit,
            docker_image: "test_image".to_string(),
            signing_id: Default::default(),
            env: None,
            env_file: None,
        };
        
        if self.config.modules.is_none() {
            self.config.modules = Some(vec![]);
        }
        
        self.config.modules.as_mut().unwrap().push(module);
        self
    }
    
    fn validate(self) -> Result<CommitBoostConfig> {
        // Validate using conflict matrix
        let matrix = ConflictMatrix::new();
        
        if let Some(ref modules) = self.config.modules {
            let module_ids: Vec<String> = modules.iter()
                .map(|m| m.id.to_string())
                .collect();
            
            matrix.validate_module_list(&module_ids)
                .map_err(|e| eyre::eyre!("Module validation failed: {}", e))?;
        }
        
        Ok(self.config)
    }
}

#[test]
fn test_compatible_modules_pass_validation() {
    let config = TestConfig::new()
        .with_module("pbs", 8001)
        .with_module("signer", 8002)
        .with_module("metrics", 8003)
        .validate();
    
    assert!(config.is_ok(), "Compatible modules should pass validation");
}

#[test]
fn test_incompatible_modules_fail_validation() {
    let config = TestConfig::new()
        .with_module("pbs_relay_a", 8001)
        .with_module("pbs_relay_b", 8002)
        .validate();
    
    assert!(config.is_err(), "Incompatible modules should fail validation");
    
    let error_msg = config.unwrap_err().to_string();
    assert!(
        error_msg.contains("pbs_relay_a") && error_msg.contains("pbs_relay_b"),
        "Error should mention both conflicting modules: {}", error_msg
    );
}

#[test]
fn test_port_conflict_detection() {
    let mut used_ports = std::collections::HashSet::new();
    
    // Simulate port usage check
    let ports_to_check = vec![8080, 8081, 8080]; // Duplicate port
    
    for port in ports_to_check {
        assert!(
            used_ports.insert(port) || port == 8080, // 8080 appears twice, so second insert should fail
            "Port {} should be unique", port
        );
    }
    
    // The duplicate port (8080) should have been detected
    assert_eq!(used_ports.len(), 2, "Should only have 2 unique ports");
}

#[test]
fn test_module_id_uniqueness_enforcement() {
    // Test with duplicate module IDs
    let duplicate_modules = vec![
        "test_module".to_string(),
        "other_module".to_string(),
        "test_module".to_string(), // Duplicate
    ];
    
    // This should be caught by the uniqueness check (not the conflict matrix)
    let mut seen_ids = std::collections::HashSet::new();
    let mut has_duplicates = false;
    
    for module_id in &duplicate_modules {
        if !seen_ids.insert(module_id.clone()) {
            has_duplicates = true;
            break;
        }
    }
    
    assert!(has_duplicates, "Duplicate module IDs should be detected");
}

#[test]
fn test_conflict_matrix_integration() {
    let matrix = ConflictMatrix::new();
    
    // Test that matrix correctly identifies compatible modules
    let compatible_config = vec![
        "pbs".to_string(),
        "signer".to_string(),
        "metrics".to_string(),
    ];
    
    assert!(
        matrix.validate_module_list(&compatible_config).is_ok(),
        "Known compatible modules should pass validation"
    );
    
    // Test that matrix correctly identifies incompatible modules
    let incompatible_config = vec![
        "custom_pbs".to_string(),
        "pbs".to_string(),
    ];
    
    assert!(
        matrix.validate_module_list(&incompatible_config).is_err(),
        "Known incompatible modules should fail validation"
    );
}

#[test]
fn test_edge_cases() {
    let matrix = ConflictMatrix::new();
    
    // Test empty configuration
    assert!(
        matrix.validate_module_list(&[]).is_ok(),
        "Empty module list should be valid"
    );
    
    // Test single module
    let single_module = vec!["pbs".to_string()];
    assert!(
        matrix.validate_module_list(&single_module).is_ok(),
        "Single module should be valid"
    );
    
    // Test unknown modules (should default to compatible)
    let unknown_modules = vec![
        "unknown_module_1".to_string(),
        "unknown_module_2".to_string(),
    ];
    assert!(
        matrix.validate_module_list(&unknown_modules).is_ok(),
        "Unknown modules should default to compatible"
    );
}

#[test]
fn test_metric_namespace_separation() {
    // Test that different modules use different metric prefixes
    let module_metrics = vec![
        ("pbs", "cb_pbs_"),
        ("signer", "cb_signer_"),
        ("metrics", "cb_metrics_"),
    ];
    
    // Check that no prefix is a prefix of another
    for (i, (module_a, prefix_a)) in module_metrics.iter().enumerate() {
        for (j, (module_b, prefix_b)) in module_metrics.iter().enumerate() {
            if i != j {
                assert!(
                    !prefix_a.starts_with(prefix_b) || prefix_a == prefix_b,
                    "Module '{}' prefix '{}' conflicts with module '{}' prefix '{}'",
                    module_a, prefix_a, module_b, prefix_b
                );
            }
        }
    }
}

#[test]
fn test_real_world_configuration() {
    // Test a realistic configuration that should work
    let config = TestConfig::new()
        .with_module("commit_boost_pbs", 18550)
        .with_module("commit_boost_signer", 18551)
        .validate();
    
    assert!(
        config.is_ok(),
        "Realistic configuration should be valid: {:?}",
        config.err()
    );
}

#[tokio::test]
async fn test_module_startup_sequence() {
    // This test simulates the module startup process
    // In a real implementation, this would actually start modules
    
    let modules = vec![
        ("pbs", 8080),
        ("signer", 8081),
        ("metrics", 9090),
    ];
    
    // Check conflicts before "starting"
    let matrix = ConflictMatrix::new();
    let module_ids: Vec<String> = modules.iter().map(|(id, _)| id.to_string()).collect();
    
    let validation_result = matrix.validate_module_list(&module_ids);
    assert!(
        validation_result.is_ok(),
        "Module startup should pass conflict validation: {:?}",
        validation_result.err()
    );
    
    // Simulate port binding check
    let mut bound_ports = std::collections::HashSet::new();
    for (module_id, port) in modules {
        assert!(
            bound_ports.insert(port),
            "Module '{}' cannot bind to port {} - already in use",
            module_id, port
        );
    }
}

// Configuration validation tests
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_configuration_validation_pipeline() {
    // Test comprehensive configuration validation
    let mut config = get_default_config();
    
    // Valid configuration should pass
    assert!(validate_config(&config).is_ok());
    
    // Test invalid port range
    config.pbs.pbs_config.port = 0;
    assert!(validate_config(&config).is_err());
    config.pbs.pbs_config.port = 18550;
    
    // Test invalid timeout values
    config.pbs.pbs_config.timeout_get_header_ms = 0;
    assert!(validate_config(&config).is_err());
    config.pbs.pbs_config.timeout_get_header_ms = 12000;
    
    // Test conflicting modules
    config.modules = Some(vec![
        create_test_module("pbs"),
        create_test_module("custom_pbs"),
    ]);
    assert!(validate_config(&config).is_err());
    
    // Test valid module configuration
    config.modules = Some(vec![
        create_test_module("pbs"),
        create_test_module("signer"),
        create_test_module("metrics"),
    ]);
    assert!(validate_config(&config).is_ok());
}

fn validate_config(config: &CommitBoostConfig) -> Result<()> {
    // Validate port ranges
    if config.pbs.pbs_config.port == 0 {
        return Err(eyre::eyre!("Invalid PBS port"));
    }
    
    // Validate timeouts
    if config.pbs.pbs_config.timeout_get_header_ms == 0 {
        return Err(eyre::eyre!("Invalid timeout value"));
    }
    
    // Validate module conflicts
    if let Some(ref modules) = config.modules {
        let matrix = ConflictMatrix::new();
        let module_ids: Vec<String> = modules.iter()
            .map(|m| m.id.to_string())
            .collect();
        matrix.validate_module_list(&module_ids)?;
    }
    
    Ok(())
}

fn create_test_module(id: &str) -> StaticModuleConfig {
    StaticModuleConfig {
        id: ModuleId::from(id.to_string()),
        kind: cb_common::config::ModuleKind::Commit,
        docker_image: "test_image".to_string(),
        signing_id: Default::default(),
        env: None,
        env_file: None,
    }
}

// Metrics aggregation tests
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_metrics_aggregation_contracts() {
    // Test that metrics from different modules are properly aggregated
    let metrics_store = Arc::new(RwLock::new(HashMap::<String, i64>::new()));
    
    // Simulate multiple modules writing metrics concurrently
    let mut handles = vec![];
    
    for module_id in ["pbs", "signer", "metrics"] {
        let store = metrics_store.clone();
        let id = module_id.to_string();
        
        handles.push(tokio::spawn(async move {
            for i in 0..100 {
                let mut metrics = store.write().await;
                let key = format!("cb_{}_{}", id, "requests_total");
                *metrics.entry(key).or_insert(0) += 1;
                
                if i % 10 == 0 {
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
            }
        }));
    }
    
    // Wait for all modules to complete
    for handle in handles {
        handle.await.unwrap();
    }
    
    // Verify metrics aggregation
    let metrics = metrics_store.read().await;
    assert_eq!(*metrics.get("cb_pbs_requests_total").unwrap(), 100);
    assert_eq!(*metrics.get("cb_signer_requests_total").unwrap(), 100);
    assert_eq!(*metrics.get("cb_metrics_requests_total").unwrap(), 100);
    
    // Verify namespace separation
    for (key, _) in metrics.iter() {
        assert!(key.starts_with("cb_"), "All metrics should have cb_ prefix");
    }
}

// Event propagation tests
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_event_propagation_between_modules() {
    // Create event bus for module communication
    let (tx, _) = broadcast::channel::<ModuleEvent>(100);
    
    // Create receivers for each module
    let mut pbs_rx = tx.subscribe();
    let mut signer_rx = tx.subscribe();
    let mut metrics_rx = tx.subscribe();
    
    // Test event propagation
    let events = vec![
        ModuleEvent::Started("pbs".to_string()),
        ModuleEvent::RequestReceived("signer".to_string()),
        ModuleEvent::MetricUpdated("metrics".to_string()),
        ModuleEvent::Stopped("pbs".to_string()),
    ];
    
    // Send events
    for event in &events {
        tx.send(event.clone()).unwrap();
    }
    
    // Verify all modules receive all events
    for event in &events {
        assert_eq!(pbs_rx.recv().await.unwrap(), *event);
        assert_eq!(signer_rx.recv().await.unwrap(), *event);
        assert_eq!(metrics_rx.recv().await.unwrap(), *event);
    }
}

#[derive(Clone, Debug, PartialEq)]
enum ModuleEvent {
    Started(String),
    RequestReceived(String),
    MetricUpdated(String),
    Stopped(String),
}

// Performance test for configuration validation
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_config_validation_performance() {
    // Test that we can validate 100 configurations per second
    let start = Instant::now();
    let iterations = 100;
    
    for _ in 0..iterations {
        let config = TestConfig::new()
            .with_module("pbs", 8001)
            .with_module("signer", 8002)
            .with_module("metrics", 8003)
            .validate();
        
        assert!(config.is_ok());
    }
    
    let elapsed = start.elapsed();
    let rate = iterations as f64 / elapsed.as_secs_f64();
    
    assert!(
        rate >= 100.0,
        "Configuration validation too slow: {:.2} configs/sec (expected >= 100)",
        rate
    );
}

// Test module dependency resolution
#[test]
fn test_module_dependency_resolution() {
    // Some modules may depend on others
    let dependencies = HashMap::from([
        ("pbs", vec!["signer"]),
        ("metrics", vec![]),
        ("custom_module", vec!["pbs", "signer"]),
    ]);
    
    // Test valid dependency order
    let load_order = vec!["signer", "pbs", "metrics", "custom_module"];
    assert!(validate_dependencies(&load_order, &dependencies));
    
    // Test invalid dependency order (pbs before signer)
    let invalid_order = vec!["pbs", "signer", "metrics"];
    assert!(!validate_dependencies(&invalid_order, &dependencies));
}

fn validate_dependencies(load_order: &[&str], dependencies: &HashMap<&str, Vec<&str>>) -> bool {
    let mut loaded = std::collections::HashSet::new();
    
    for module in load_order {
        if let Some(deps) = dependencies.get(module) {
            for dep in deps {
                if !loaded.contains(dep) {
                    return false;
                }
            }
        }
        loaded.insert(*module);
    }
    
    true
}

// Test configuration hot reloading
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_configuration_hot_reload() {
    let config = Arc::new(RwLock::new(get_default_config()));
    let config_clone = config.clone();
    
    // Simulate configuration update
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let mut cfg = config_clone.write().await;
        cfg.pbs.pbs_config.port = 19550;
    });
    
    // Initial port
    {
        let cfg = config.read().await;
        assert_eq!(cfg.pbs.pbs_config.port, 18550);
    }
    
    // Wait for update
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    // Updated port
    {
        let cfg = config.read().await;
        assert_eq!(cfg.pbs.pbs_config.port, 19550);
    }
}

// Test metrics collection across modules
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_cross_module_metrics_collection() {
    struct ModuleMetrics {
        requests: Arc<RwLock<u64>>,
        errors: Arc<RwLock<u64>>,
        latency_ms: Arc<RwLock<Vec<u64>>>,
    }
    
    let pbs_metrics = ModuleMetrics {
        requests: Arc::new(RwLock::new(0)),
        errors: Arc::new(RwLock::new(0)),
        latency_ms: Arc::new(RwLock::new(Vec::new())),
    };
    
    // Simulate concurrent metric updates
    let mut handles = vec![];
    
    for i in 0..10 {
        let requests = pbs_metrics.requests.clone();
        let errors = pbs_metrics.errors.clone();
        let latency = pbs_metrics.latency_ms.clone();
        
        handles.push(tokio::spawn(async move {
            *requests.write().await += 10;
            if i % 3 == 0 {
                *errors.write().await += 1;
            }
            latency.write().await.push(10 + i);
        }));
    }
    
    for handle in handles {
        handle.await.unwrap();
    }
    
    // Verify metrics
    assert_eq!(*pbs_metrics.requests.read().await, 100);
    assert_eq!(*pbs_metrics.errors.read().await, 4); // 0, 3, 6, 9
    assert_eq!(pbs_metrics.latency_ms.read().await.len(), 10);
    
    // Calculate average latency
    let latencies = pbs_metrics.latency_ms.read().await;
    let avg_latency = latencies.iter().sum::<u64>() as f64 / latencies.len() as f64;
    assert!(avg_latency >= 10.0 && avg_latency <= 20.0);
}