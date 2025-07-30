use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Instant,
};

use commit_boost::prelude::*;
use tokio::sync::{mpsc, Mutex};

use crate::{
    commitment::RegistrationNotification,
    config::XGAConfig,
    eigenlayer::DefaultEigenLayerIntegration,
    infrastructure::{CircuitBreaker, HttpClientFactory, RateLimiter},
};

/// Nonce entry with expiration time
#[derive(Debug, Clone)]
pub struct NonceEntry {
    pub created_at: Instant,
    pub validator_pubkey: BlsPublicKey,
}

/// Application state shared across webhook handlers
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<StartCommitModuleConfig<XGAConfig>>,
    pub processing_sender: mpsc::Sender<RegistrationNotification>,
    pub nonce_tracker: Arc<Mutex<HashMap<[u8; 32], NonceEntry>>>,
    pub dedup_tracker: Arc<Mutex<HashSet<(BlsPublicKey, u64, String)>>>,
    pub circuit_breaker: Arc<CircuitBreaker>,
    pub http_client_factory: Arc<HttpClientFactory>,
    pub rate_limiter: Arc<RateLimiter>,
    pub eigenlayer: Option<Arc<Mutex<DefaultEigenLayerIntegration>>>,
}

impl AppState {
    /// Create a new AppState instance
    pub fn new(
        config: Arc<StartCommitModuleConfig<XGAConfig>>,
        processing_sender: mpsc::Sender<RegistrationNotification>,
        circuit_breaker: Arc<CircuitBreaker>,
        http_client_factory: Arc<HttpClientFactory>,
        rate_limiter: Arc<RateLimiter>,
        eigenlayer: Option<Arc<Mutex<DefaultEigenLayerIntegration>>>,
    ) -> Self {
        Self {
            config,
            processing_sender,
            nonce_tracker: Arc::new(Mutex::new(HashMap::with_capacity(1000))), /* Pre-allocate for typical validator count */
            dedup_tracker: Arc::new(Mutex::new(HashSet::with_capacity(1000))), /* Pre-allocate for typical validator count */
            circuit_breaker,
            http_client_factory,
            rate_limiter,
            eigenlayer,
        }
    }
}
