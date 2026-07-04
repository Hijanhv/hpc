//! REST/JSON control API served with [`axum`].
//!
//! This is the human- and tooling-facing surface: the CLI and monitor talk to
//! it, and it is trivially `curl`-able. Handlers are thin — they validate input,
//! call into [`SharedState`], and render domain types straight to JSON. All
//! errors flow through [`ApiError`] which maps [`HpcError`] onto sensible HTTP
//! status codes.

use std::collections::BTreeMap;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use hpc_core::error::HpcError;
use hpc_core::types::{
    now_unix, CommandOutcome, DeployAction, DeploySpec, FsAction, FsSpec, MetricsReport,
    NodeRecord, NodeStatus,
};
use serde::{Deserialize, Serialize};

use crate::state::{NodeCommand, SharedState};

/// Build the REST router with shared state injected.
pub fn router(state: SharedState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/api/v1/cluster/status", get(cluster_status))
        .route("/api/v1/nodes", get(list_nodes))
        .route("/api/v1/nodes/{id}", get(get_node).delete(deregister_node))
        .route("/api/v1/nodes/{id}/metrics", get(get_node_metrics))
        .route("/api/v1/nodes/{id}/deploy", post(deploy))
        .route("/api/v1/nodes/{id}/fs", post(fs_command))
        .route("/api/v1/outcomes", get(list_outcomes))
        .with_state(state)
}

// ---------------------------------------------------------------------------
// Error handling
// ---------------------------------------------------------------------------

/// An HTTP-shaped error with a JSON body.
#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        ApiError {
            status,
            message: message.into(),
        }
    }
}

impl From<HpcError> for ApiError {
    fn from(e: HpcError) -> Self {
        let status = match &e {
            HpcError::NotFound(_) => StatusCode::NOT_FOUND,
            HpcError::InvalidState(_) => StatusCode::CONFLICT,
            HpcError::ConfigInvalid(_) | HpcError::Conversion(_) => StatusCode::BAD_REQUEST,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        ApiError::new(status, e.to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        #[derive(Serialize)]
        struct Body {
            error: String,
        }
        (
            self.status,
            Json(Body {
                error: self.message,
            }),
        )
            .into_response()
    }
}

type ApiResult<T> = std::result::Result<T, ApiError>;

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn health() -> &'static str {
    "ok"
}

/// Aggregate cluster health, cheap enough to poll frequently.
#[derive(Debug, Serialize)]
struct ClusterStatus {
    epoch: u64,
    total_nodes: usize,
    healthy: usize,
    degraded: usize,
    unreachable: usize,
    connected_streams: usize,
    server_time_unix: u64,
}

async fn cluster_status(State(state): State<SharedState>) -> Json<ClusterStatus> {
    let nodes = state.list_nodes().await;
    let mut healthy = 0;
    let mut degraded = 0;
    let mut unreachable = 0;
    for n in &nodes {
        match n.status {
            NodeStatus::Healthy | NodeStatus::Registered => healthy += 1,
            NodeStatus::Degraded => degraded += 1,
            NodeStatus::Unreachable => unreachable += 1,
            NodeStatus::Draining => {}
        }
    }
    Json(ClusterStatus {
        epoch: state.epoch(),
        total_nodes: nodes.len(),
        healthy,
        degraded,
        unreachable,
        connected_streams: state.connected_count().await,
        server_time_unix: now_unix(),
    })
}

async fn list_nodes(State(state): State<SharedState>) -> Json<Vec<NodeRecord>> {
    Json(state.list_nodes().await)
}

async fn get_node(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> ApiResult<Json<NodeRecord>> {
    state
        .get_node(&id)
        .await
        .map(Json)
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, format!("unknown node {id}")))
}

async fn deregister_node(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    if state.deregister_node(&id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::new(
            StatusCode::NOT_FOUND,
            format!("unknown node {id}"),
        ))
    }
}

async fn get_node_metrics(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> ApiResult<Json<MetricsReport>> {
    let record = state
        .get_node(&id)
        .await
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, format!("unknown node {id}")))?;
    record
        .latest_metrics
        .map(Json)
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "no metrics reported yet"))
}

async fn list_outcomes(State(state): State<SharedState>) -> ApiResult<Json<Vec<CommandOutcome>>> {
    Ok(Json(state.list_outcomes()?))
}

/// Body for `POST /nodes/{id}/deploy`. `deployment_id` is server-generated when
/// omitted.
#[derive(Debug, Deserialize)]
struct DeployRequest {
    #[serde(default)]
    deployment_id: Option<String>,
    #[serde(default)]
    action: DeployAction,
    component: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    target_path: String,
    #[serde(default)]
    options: BTreeMap<String, String>,
}

/// Response acknowledging a command was accepted and dispatched.
#[derive(Debug, Serialize)]
struct DispatchAck {
    accepted: bool,
    command_id: String,
    node_id: String,
}

async fn deploy(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(req): Json<DeployRequest>,
) -> ApiResult<Json<DispatchAck>> {
    if req.component.trim().is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "component must not be empty",
        ));
    }
    if !state.is_connected(&id).await {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            format!("node {id} is not connected"),
        ));
    }
    let deployment_id = req
        .deployment_id
        .unwrap_or_else(|| format!("dep-{}", uuid::Uuid::new_v4()));
    let spec = DeploySpec {
        deployment_id: deployment_id.clone(),
        action: req.action,
        component: req.component,
        version: req.version,
        target_path: req.target_path,
        options: req.options,
    };
    state.dispatch(&id, NodeCommand::Deploy(spec)).await?;
    Ok(Json(DispatchAck {
        accepted: true,
        command_id: deployment_id,
        node_id: id,
    }))
}

/// Body for `POST /nodes/{id}/fs`.
#[derive(Debug, Deserialize)]
struct FsRequest {
    #[serde(default)]
    command_id: Option<String>,
    action: FsAction,
    #[serde(default)]
    device: String,
    mount_point: String,
    #[serde(default)]
    fs_type: String,
    #[serde(default)]
    mount_options: Vec<String>,
    #[serde(default)]
    force: bool,
}

async fn fs_command(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(req): Json<FsRequest>,
) -> ApiResult<Json<DispatchAck>> {
    if req.mount_point.trim().is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "mount_point must not be empty",
        ));
    }
    if matches!(req.action, FsAction::Format) && !req.force {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "format is destructive and requires force=true",
        ));
    }
    if !state.is_connected(&id).await {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            format!("node {id} is not connected"),
        ));
    }
    let command_id = req
        .command_id
        .unwrap_or_else(|| format!("fs-{}", uuid::Uuid::new_v4()));
    let spec = FsSpec {
        command_id: command_id.clone(),
        action: req.action,
        device: req.device,
        mount_point: req.mount_point,
        fs_type: req.fs_type,
        mount_options: req.mount_options,
        force: req.force,
    };
    state.dispatch(&id, NodeCommand::Fs(spec)).await?;
    Ok(Json(DispatchAck {
        accepted: true,
        command_id,
        node_id: id,
    }))
}
