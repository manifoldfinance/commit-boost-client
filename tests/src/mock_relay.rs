use std::{
    net::SocketAddr,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, RwLock,
    },
    time::{SystemTime, UNIX_EPOCH},
};

use alloy::{primitives::U256, rpc::types::beacon::relay::ValidatorRegistration};
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use cb_common::{
    pbs::{
        ExecutionPayloadHeaderMessageElectra, GetHeaderParams, GetHeaderResponse,
        SignedExecutionPayloadHeader, SubmitBlindedBlockResponse, BUILDER_API_PATH,
        GET_HEADER_PATH, GET_STATUS_PATH, REGISTER_VALIDATOR_PATH, SUBMIT_BLOCK_PATH,
    },
    signature::sign_builder_root,
    signer::BlsSecretKey,
    types::Chain,
    utils::{blst_pubkey_to_alloy, timestamp_of_slot_start_sec},
};
use cb_pbs::MAX_SIZE_SUBMIT_BLOCK_RESPONSE;
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tracing::{debug, info};
use tree_hash::TreeHash;

pub async fn start_mock_relay_service(state: Arc<MockRelayState>, port: u16) -> eyre::Result<()> {
    let app = mock_relay_app_router(state);

    let socket = SocketAddr::new("0.0.0.0".parse()?, port);
    let listener = TcpListener::bind(socket).await?;

    axum::serve(listener, app).await?;
    Ok(())
}

/// XGA-specific state for mock relay
#[derive(Debug)]
pub struct XgaState {
    received_commitments: Arc<AtomicU64>,
    stored_commitments: Arc<RwLock<Vec<serde_json::Value>>>,
    mock_registrations: Arc<RwLock<Vec<ValidatorRegistration>>>,
    capability_supported: bool,
}

impl XgaState {
    fn new(registrations: Vec<ValidatorRegistration>) -> Self {
        Self {
            received_commitments: Arc::new(AtomicU64::new(0)),
            stored_commitments: Arc::new(RwLock::new(Vec::new())),
            mock_registrations: Arc::new(RwLock::new(registrations)),
            capability_supported: true,
        }
    }
}

/// XGA relay response format
#[derive(Debug, Serialize, Deserialize)]
struct XgaRelayResponse {
    success: bool,
    message: Option<String>,
    commitment_id: Option<String>,
}

/// Query parameters for registrations endpoint
#[derive(Debug, Deserialize)]
struct RegistrationsQuery {
    since: Option<u64>,
}

/// Response format for registrations endpoint
#[derive(Debug, Serialize)]
struct RelayRegistrationsResponse {
    registrations: Vec<ValidatorRegistration>,
}

pub struct MockRelayState {
    pub chain: Chain,
    pub signer: BlsSecretKey,
    large_body: bool,
    received_get_header: Arc<AtomicU64>,
    received_get_status: Arc<AtomicU64>,
    received_register_validator: Arc<AtomicU64>,
    received_submit_block: Arc<AtomicU64>,
    response_override: RwLock<Option<StatusCode>>,
    xga: Option<XgaState>,
}

impl MockRelayState {
    pub fn received_get_header(&self) -> u64 {
        self.received_get_header.load(Ordering::Relaxed)
    }
    pub fn received_get_status(&self) -> u64 {
        self.received_get_status.load(Ordering::Relaxed)
    }
    pub fn received_register_validator(&self) -> u64 {
        self.received_register_validator.load(Ordering::Relaxed)
    }
    pub fn received_submit_block(&self) -> u64 {
        self.received_submit_block.load(Ordering::Relaxed)
    }
    pub fn large_body(&self) -> bool {
        self.large_body
    }
    pub fn set_response_override(&self, status: StatusCode) {
        *self.response_override.write().unwrap() = Some(status);
    }
    
    // XGA-specific methods
    pub fn received_xga_commitments(&self) -> u64 {
        self.xga.as_ref().map_or(0, |xga| xga.received_commitments.load(Ordering::Relaxed))
    }
    
    pub fn get_stored_xga_commitments(&self) -> Vec<serde_json::Value> {
        self.xga.as_ref().map_or(vec![], |xga| xga.stored_commitments.read().unwrap().clone())
    }
}

impl MockRelayState {
    pub fn new(chain: Chain, signer: BlsSecretKey) -> Self {
        Self {
            chain,
            signer,
            large_body: false,
            received_get_header: Default::default(),
            received_get_status: Default::default(),
            received_register_validator: Default::default(),
            received_submit_block: Default::default(),
            response_override: RwLock::new(None),
            xga: None,
        }
    }

    pub fn with_large_body(self) -> Self {
        Self { large_body: true, ..self }
    }
    
    /// Enable XGA support with optional pre-populated registrations
    pub fn with_xga_support(mut self, registrations: Vec<ValidatorRegistration>) -> Self {
        self.xga = Some(XgaState::new(registrations));
        self
    }
}

pub fn mock_relay_app_router(state: Arc<MockRelayState>) -> Router {
    let builder_routes = Router::new()
        .route(GET_HEADER_PATH, get(handle_get_header))
        .route(GET_STATUS_PATH, get(handle_get_status))
        .route(REGISTER_VALIDATOR_PATH, post(handle_register_validator))
        .route(SUBMIT_BLOCK_PATH, post(handle_submit_block))
        // XGA routes
        .route("/registrations", get(handle_get_registrations))
        .route("/xga/commitment", post(handle_xga_commitment))
        .route("/xga/capabilities", get(handle_xga_capabilities))
        .with_state(state);

    Router::new().nest(BUILDER_API_PATH, builder_routes)
}

async fn handle_get_header(
    State(state): State<Arc<MockRelayState>>,
    Path(GetHeaderParams { parent_hash, .. }): Path<GetHeaderParams>,
) -> Response {
    state.received_get_header.fetch_add(1, Ordering::Relaxed);

    let mut response: SignedExecutionPayloadHeader<ExecutionPayloadHeaderMessageElectra> =
        SignedExecutionPayloadHeader::default();

    response.message.header.parent_hash = parent_hash;
    response.message.header.block_hash.0[0] = 1;
    response.message.value = U256::from(10);
    response.message.pubkey = blst_pubkey_to_alloy(&state.signer.sk_to_pk());
    response.message.header.timestamp = timestamp_of_slot_start_sec(0, state.chain);

    let object_root = response.message.tree_hash_root().0;
    response.signature = sign_builder_root(state.chain, &state.signer, object_root);

    let response = GetHeaderResponse::Electra(response);
    (StatusCode::OK, Json(response)).into_response()
}

async fn handle_get_status(State(state): State<Arc<MockRelayState>>) -> impl IntoResponse {
    state.received_get_status.fetch_add(1, Ordering::Relaxed);
    StatusCode::OK
}

async fn handle_register_validator(
    State(state): State<Arc<MockRelayState>>,
    Json(validators): Json<Vec<ValidatorRegistration>>,
) -> impl IntoResponse {
    state.received_register_validator.fetch_add(1, Ordering::Relaxed);
    debug!("Received {} registrations", validators.len());

    if let Some(status) = state.response_override.read().unwrap().as_ref() {
        return (*status).into_response();
    }

    StatusCode::OK.into_response()
}

async fn handle_submit_block(State(state): State<Arc<MockRelayState>>) -> Response {
    state.received_submit_block.fetch_add(1, Ordering::Relaxed);
    if state.large_body() {
        (StatusCode::OK, Json(vec![1u8; 1 + MAX_SIZE_SUBMIT_BLOCK_RESPONSE])).into_response()
    } else {
        let response = SubmitBlindedBlockResponse::default();
        Json(response).into_response()
    }
}

// XGA handler functions

async fn handle_get_registrations(
    State(state): State<Arc<MockRelayState>>,
    Query(params): Query<RegistrationsQuery>,
) -> Response {
    // Check if XGA is enabled
    let Some(xga) = &state.xga else {
        return (StatusCode::NOT_FOUND, "XGA not enabled").into_response();
    };

    let registrations = xga.mock_registrations.read().unwrap();
    
    // Filter by timestamp if 'since' parameter provided
    let filtered_registrations: Vec<ValidatorRegistration> = if let Some(since) = params.since {
        registrations
            .iter()
            .filter(|reg| reg.message.timestamp >= since)
            .cloned()
            .collect()
    } else {
        registrations.clone()
    };

    let response = RelayRegistrationsResponse {
        registrations: filtered_registrations,
    };

    Json(response).into_response()
}

async fn handle_xga_commitment(
    State(state): State<Arc<MockRelayState>>,
    headers: HeaderMap,
    Json(commitment): Json<serde_json::Value>,
) -> impl IntoResponse {
    // Check if XGA is enabled
    let Some(xga) = &state.xga else {
        return (StatusCode::NOT_FOUND, "XGA not enabled").into_response();
    };

    // Check for response override
    if let Some(status) = state.response_override.read().unwrap().as_ref() {
        return (*status).into_response();
    }

    // Log client information from headers
    if let Some(user_agent) = headers.get("user-agent") {
        debug!("XGA commitment from client: {:?}", user_agent);
    }
    
    // Validate content type
    if let Some(content_type) = headers.get("content-type") {
        if !content_type.to_str().unwrap_or("").contains("application/json") {
            return (StatusCode::BAD_REQUEST, "Invalid content type").into_response();
        }
    }

    // Increment counter
    xga.received_commitments.fetch_add(1, Ordering::Relaxed);
    
    // Store the commitment
    xga.stored_commitments.write().unwrap().push(commitment.clone());
    
    info!("Received XGA commitment");
    debug!("XGA commitment data: {:?}", commitment);

    // Generate a mock commitment ID using timestamp and counter
    let commitment_id = format!("xga_{}_{}", 
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
        xga.received_commitments.load(Ordering::Relaxed));
    
    let response = XgaRelayResponse {
        success: true,
        message: Some("Commitment accepted".to_string()),
        commitment_id: Some(commitment_id.clone()),
    };

    let resp = Response::builder()
        .status(StatusCode::OK)
        .header("x-xga-commitment-id", commitment_id)
        .header("x-xga-rate-limit-remaining", "99")
        .header("content-type", "application/json")
        .body(axum::body::Body::from(serde_json::to_string(&response).unwrap()))
        .unwrap();
    
    resp
}

async fn handle_xga_capabilities(
    State(state): State<Arc<MockRelayState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    // Check if XGA is enabled
    let Some(xga) = &state.xga else {
        return (StatusCode::NOT_FOUND, "XGA not enabled").into_response();
    };

    if !xga.capability_supported {
        return (StatusCode::NOT_FOUND, "XGA capabilities not supported").into_response();
    }
    
    // Check Accept header to determine response format
    let prefers_json = headers
        .get("accept")
        .and_then(|h| h.to_str().ok())
        .map(|s| s.contains("application/json"))
        .unwrap_or(true);
    
    debug!("Capabilities request prefers JSON: {}", prefers_json);

    // Return capabilities response
    let capabilities = serde_json::json!({
        "version": "1.0.0",
        "supported": true,
        "endpoints": {
            "commitment": "/eth/v1/builder/xga/commitment",
            "capabilities": "/eth/v1/builder/xga/capabilities"
        },
        "features": ["rate_limiting", "signature_verification"]
    });

    // Also support OPTIONS request detection by adding custom header
    let mut response = (StatusCode::OK, Json(capabilities)).into_response();
    response.headers_mut().insert("x-xga-supported", "true".parse().unwrap());
    
    // Add CORS headers if Origin is present
    if let Some(origin) = headers.get("origin") {
        response.headers_mut().insert("access-control-allow-origin", origin.clone());
        response.headers_mut().insert("access-control-allow-methods", "GET, POST, OPTIONS".parse().unwrap());
    }
    
    response
}
