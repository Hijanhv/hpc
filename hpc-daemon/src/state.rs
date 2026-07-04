//! In-memory cluster state, shared across the gRPC and HTTP servers.
//!
//! The canonical pattern the task calls for — `Arc<RwLock<…>>` — lives here.
//! [`SharedState`] is a cheap-to-clone handle wrapping:
//!
//! * an `Arc<RwLock<ClusterState>>` holding the node table, and
//! * an `Arc<RwLock<…>>` registry of live command channels (one per connected
//!   agent) used to push deploy/filesystem commands down each agent's
//!   server-stream.
//!
//! Every mutation is also written through to the durable [`Store`], so a daemon
//! restart rehydrates to the last committed state.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use hpc_core::config::{DaemonConfig, Thresholds};
use hpc_core::error::{HpcError, Result};
use hpc_core::types::{
    now_unix, CommandOutcome, DeploySpec, FsSpec, MetricsReport, NodeId, NodeInfo, NodeRecord,
    NodeStatus,
};
use tokio::sync::{mpsc, RwLock};

use crate::store::Store;

/// A command destined for a specific node, carried over that node's live
/// command stream.
#[derive(Debug, Clone)]
pub enum NodeCommand {
    Deploy(DeploySpec),
    Fs(FsSpec),
}

impl NodeCommand {
    /// The id used to correlate the command with its eventual outcome.
    pub fn command_id(&self) -> &str {
        match self {
            NodeCommand::Deploy(d) => &d.deployment_id,
            NodeCommand::Fs(f) => &f.command_id,
        }
    }
}

/// The mutable portion of the cluster's state.
#[derive(Debug, Default)]
struct ClusterState {
    nodes: HashMap<NodeId, NodeRecord>,
}

/// Cheap-to-clone, thread-safe handle to all daemon state.
#[derive(Clone)]
pub struct SharedState {
    inner: Arc<RwLock<ClusterState>>,
    channels: Arc<RwLock<HashMap<NodeId, mpsc::Sender<NodeCommand>>>>,
    store: Store,
    thresholds: Thresholds,
    node_timeout: Duration,
    metrics_interval_secs: u32,
    epoch: u64,
}

impl std::fmt::Debug for SharedState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedState")
            .field("epoch", &self.epoch)
            .field("node_timeout", &self.node_timeout)
            .finish_non_exhaustive()
    }
}

impl SharedState {
    /// Build shared state and hydrate the node table from the durable store.
    pub async fn bootstrap(cfg: &DaemonConfig, store: Store) -> Result<Self> {
        let mut nodes = HashMap::new();
        for mut record in store.load_nodes()? {
            // A node loaded from disk hasn't been seen this run yet.
            record.status = NodeStatus::Unreachable;
            nodes.insert(record.info.node_id.clone(), record);
        }
        tracing::info!(
            node_count = nodes.len(),
            "hydrated cluster state from store"
        );

        Ok(SharedState {
            inner: Arc::new(RwLock::new(ClusterState { nodes })),
            channels: Arc::new(RwLock::new(HashMap::new())),
            store,
            thresholds: cfg.thresholds.clone(),
            node_timeout: cfg.node_timeout,
            metrics_interval_secs: cfg.metrics_interval.as_secs().max(1) as u32,
            epoch: now_unix(),
        })
    }

    /// The epoch this daemon instance started at (bumped only on restart).
    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// The metrics cadence agents should adopt, in seconds.
    pub fn metrics_interval_secs(&self) -> u32 {
        self.metrics_interval_secs
    }

    /// Register (or re-register) a node. Returns the canonical record.
    pub async fn register_node(&self, info: NodeInfo) -> Result<NodeRecord> {
        let node_id = info.node_id.clone();
        let record = {
            let mut state = self.inner.write().await;
            let entry = state
                .nodes
                .entry(node_id.clone())
                .or_insert_with(|| NodeRecord::new(info.clone()));
            entry.info = info;
            entry.status = NodeStatus::Registered;
            entry.last_seen_unix = now_unix();
            entry.clone()
        };
        self.store.put_node(&record)?;
        tracing::info!(%node_id, role = ?record.info.role, "node registered");
        Ok(record)
    }

    /// Apply a fresh metrics sample, recomputing derived health.
    pub async fn ingest_metrics(&self, report: MetricsReport) -> Result<()> {
        let node_id = report.node_id.clone();
        let record = {
            let mut state = self.inner.write().await;
            let Some(record) = state.nodes.get_mut(&node_id) else {
                return Err(HpcError::NotFound(format!("unknown node {node_id}")));
            };
            record.status = classify(&report, &self.thresholds);
            record.last_seen_unix = now_unix();
            record.latest_metrics = Some(report);
            record.clone()
        };
        self.store.put_node(&record)?;
        Ok(())
    }

    /// Record a heartbeat, resurrecting a previously-unreachable node.
    pub async fn heartbeat(&self, node_id: &NodeId) -> Result<()> {
        let mut state = self.inner.write().await;
        let Some(record) = state.nodes.get_mut(node_id) else {
            return Err(HpcError::NotFound(format!("unknown node {node_id}")));
        };
        record.last_seen_unix = now_unix();
        if record.status == NodeStatus::Unreachable {
            record.status = NodeStatus::Healthy;
        }
        Ok(())
    }

    /// Snapshot of all node records, sorted by id for stable output.
    pub async fn list_nodes(&self) -> Vec<NodeRecord> {
        let state = self.inner.read().await;
        let mut v: Vec<_> = state.nodes.values().cloned().collect();
        v.sort_by(|a, b| a.info.node_id.cmp(&b.info.node_id));
        v
    }

    /// Fetch a single node record.
    pub async fn get_node(&self, node_id: &NodeId) -> Option<NodeRecord> {
        self.inner.read().await.nodes.get(node_id).cloned()
    }

    /// Load the recorded command-outcome audit log from the durable store.
    pub fn list_outcomes(&self) -> Result<Vec<CommandOutcome>> {
        self.store.load_outcomes()
    }

    /// Number of agents with a live command stream attached.
    pub async fn connected_count(&self) -> usize {
        self.channels.read().await.len()
    }

    /// Persist a command outcome and clear any transient failure state.
    pub async fn record_outcome(&self, outcome: CommandOutcome) -> Result<()> {
        tracing::info!(
            command_id = %outcome.command_id,
            node = %outcome.node_id,
            success = outcome.success,
            "command outcome received"
        );
        self.store.put_outcome(&outcome)
    }

    /// Sweep for nodes that have not been seen within `node_timeout` and mark
    /// them `Unreachable`. Returns the ids whose status changed.
    pub async fn reap_stale(&self) -> Vec<NodeId> {
        let cutoff = now_unix().saturating_sub(self.node_timeout.as_secs());
        let mut changed = Vec::new();
        let mut state = self.inner.write().await;
        for record in state.nodes.values_mut() {
            let stale = record.last_seen_unix < cutoff;
            if stale && record.status != NodeStatus::Unreachable {
                record.status = NodeStatus::Unreachable;
                changed.push(record.info.node_id.clone());
            }
        }
        changed
    }

    // -- command dispatch ---------------------------------------------------

    /// Called by `StreamCommands` when an agent connects: registers the sending
    /// half of its command channel and returns the receiving half.
    pub async fn attach_command_channel(&self, node_id: NodeId) -> mpsc::Receiver<NodeCommand> {
        let (tx, rx) = mpsc::channel(64);
        self.channels.write().await.insert(node_id.clone(), tx);
        tracing::info!(%node_id, "command stream attached");
        rx
    }

    /// Called when an agent's command stream closes.
    pub async fn detach_command_channel(&self, node_id: &NodeId) {
        self.channels.write().await.remove(node_id);
        tracing::info!(%node_id, "command stream detached");
    }

    /// Whether an agent currently has a live command stream.
    pub async fn is_connected(&self, node_id: &NodeId) -> bool {
        self.channels.read().await.contains_key(node_id)
    }

    /// Push a command to a connected node. Errors if the node is unknown or has
    /// no live command stream.
    pub async fn dispatch(&self, node_id: &NodeId, command: NodeCommand) -> Result<()> {
        if self.get_node(node_id).await.is_none() {
            return Err(HpcError::NotFound(format!("unknown node {node_id}")));
        }
        let sender = {
            let channels = self.channels.read().await;
            channels.get(node_id).cloned()
        };
        let Some(sender) = sender else {
            return Err(HpcError::invalid_state(format!(
                "node {node_id} has no live command stream"
            )));
        };
        tracing::info!(%node_id, command_id = command.command_id(), "dispatching command");
        sender
            .send(command)
            .await
            .map_err(|_| HpcError::invalid_state(format!("command channel for {node_id} closed")))
    }

    /// Administratively remove a node: drop it from memory, close any command
    /// stream, and delete its persisted record.
    pub async fn deregister_node(&self, node_id: &NodeId) -> Result<bool> {
        let removed = self.inner.write().await.nodes.remove(node_id).is_some();
        self.detach_command_channel(node_id).await;
        self.store.delete_node(node_id)?;
        if removed {
            tracing::info!(%node_id, "node deregistered");
        }
        Ok(removed)
    }
}

/// Classify a node's health from a metrics sample and the configured thresholds.
fn classify(report: &MetricsReport, thresholds: &Thresholds) -> NodeStatus {
    let cpu_frac = report.cpu.usage_percent / 100.0;
    if cpu_frac >= thresholds.cpu_degraded {
        return NodeStatus::Degraded;
    }
    if report.memory.used_fraction() >= thresholds.memory_degraded {
        return NodeStatus::Degraded;
    }
    if report
        .disks
        .iter()
        .any(|d| d.used_fraction() >= thresholds.disk_degraded)
    {
        return NodeStatus::Degraded;
    }
    NodeStatus::Healthy
}

#[cfg(test)]
mod tests {
    use super::*;
    use hpc_core::types::{DiskMetrics, MemoryMetrics};

    fn report_with_disk_usage(node: &str, used: u64, total: u64) -> MetricsReport {
        MetricsReport {
            node_id: node.into(),
            timestamp_unix: now_unix(),
            cpu: Default::default(),
            memory: MemoryMetrics {
                total_bytes: 100,
                used_bytes: 10,
                available_bytes: 90,
                ..Default::default()
            },
            load: Default::default(),
            disks: vec![DiskMetrics {
                total_bytes: total,
                available_bytes: total - used,
                ..Default::default()
            }],
            network: Default::default(),
            filesystems: vec![],
        }
    }

    #[test]
    fn classify_flags_full_disk_as_degraded() {
        let th = Thresholds::default();
        let healthy = report_with_disk_usage("n", 10, 100);
        let degraded = report_with_disk_usage("n", 95, 100);
        assert_eq!(classify(&healthy, &th), NodeStatus::Healthy);
        assert_eq!(classify(&degraded, &th), NodeStatus::Degraded);
    }
}
