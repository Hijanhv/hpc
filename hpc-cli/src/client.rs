//! Thin async client over the daemon's REST API.
//!
//! Every method maps one-to-one onto an endpoint in `hpc-daemon`'s `api`
//! module and deserialises straight into the shared `hpc_core::types`. Non-2xx
//! responses are turned into a descriptive error by reading the daemon's
//! `{ "error": ... }` body.

use std::time::Duration;

use anyhow::{bail, Context, Result};
use hpc_core::types::{MetricsReport, NodeRecord};
use serde::de::DeserializeOwned;
use serde::Serialize;

/// Handle to a running daemon's REST API.
#[derive(Debug, Clone)]
pub struct ApiClient {
    base: String,
    http: reqwest::Client,
}

impl ApiClient {
    /// Create a client targeting `base` (e.g. `http://127.0.0.1:8080`).
    pub fn new(base: impl Into<String>) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .context("building HTTP client")?;
        Ok(ApiClient {
            base: base.into().trim_end_matches('/').to_string(),
            http,
        })
    }

    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let url = format!("{}{path}", self.base);
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;
        Self::decode(resp, &url).await
    }

    async fn post<B: Serialize, T: DeserializeOwned>(&self, path: &str, body: &B) -> Result<T> {
        let url = format!("{}{path}", self.base);
        let resp = self
            .http
            .post(&url)
            .json(body)
            .send()
            .await
            .with_context(|| format!("POST {url}"))?;
        Self::decode(resp, &url).await
    }

    async fn decode<T: DeserializeOwned>(resp: reqwest::Response, url: &str) -> Result<T> {
        let status = resp.status();
        let bytes = resp
            .bytes()
            .await
            .with_context(|| format!("reading {url}"))?;
        if !status.is_success() {
            // Try to surface the daemon's structured error message.
            let msg = serde_json::from_slice::<ErrorBody>(&bytes)
                .map(|e| e.error)
                .unwrap_or_else(|_| String::from_utf8_lossy(&bytes).into_owned());
            bail!("{url} -> {status}: {msg}");
        }
        serde_json::from_slice(&bytes).with_context(|| format!("decoding response from {url}"))
    }

    /// `GET /api/v1/cluster/status`
    pub async fn cluster_status(&self) -> Result<ClusterStatus> {
        self.get("/api/v1/cluster/status").await
    }

    /// `GET /api/v1/nodes`
    pub async fn list_nodes(&self) -> Result<Vec<NodeRecord>> {
        self.get("/api/v1/nodes").await
    }

    /// `GET /api/v1/nodes/{id}`
    pub async fn get_node(&self, id: &str) -> Result<NodeRecord> {
        self.get(&format!("/api/v1/nodes/{id}")).await
    }

    /// `GET /api/v1/nodes/{id}/metrics`
    pub async fn node_metrics(&self, id: &str) -> Result<MetricsReport> {
        self.get(&format!("/api/v1/nodes/{id}/metrics")).await
    }

    /// `POST /api/v1/nodes/{id}/deploy`
    pub async fn deploy(&self, id: &str, body: &serde_json::Value) -> Result<DispatchAck> {
        self.post(&format!("/api/v1/nodes/{id}/deploy"), body).await
    }

    /// `POST /api/v1/nodes/{id}/fs`
    pub async fn fs_command(&self, id: &str, body: &serde_json::Value) -> Result<DispatchAck> {
        self.post(&format!("/api/v1/nodes/{id}/fs"), body).await
    }
}

#[derive(Debug, serde::Deserialize)]
struct ErrorBody {
    error: String,
}

/// Mirror of the daemon's cluster-status response (only the fields the CLI
/// renders; serde ignores the rest).
#[derive(Debug, serde::Deserialize)]
pub struct ClusterStatus {
    pub total_nodes: usize,
    pub healthy: usize,
    pub degraded: usize,
    pub unreachable: usize,
    pub connected_streams: usize,
}

/// Mirror of the daemon's command-dispatch acknowledgement.
#[derive(Debug, serde::Deserialize)]
pub struct DispatchAck {
    pub command_id: String,
    pub node_id: String,
}
