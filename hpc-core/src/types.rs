//! Domain types shared across every crate.
//!
//! These are the transport-agnostic, `serde`-friendly representations of the
//! concepts the framework manipulates. The daemon and agent convert between
//! these and the generated protobuf types at the gRPC boundary; the REST API,
//! CLI and persisted state all speak these types directly.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Stable, human-meaningful identifier for a node (usually its hostname).
pub type NodeId = String;

/// Current epoch seconds. Small helper so call sites never reach for
/// `unwrap()` on a `SystemTime` conversion.
pub fn now_unix() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The logical function a node performs in the cluster.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum NodeRole {
    #[default]
    Unspecified,
    Storage,
    Compute,
    Metadata,
    Gateway,
}

/// Coarse lifecycle/health state the daemon tracks for each node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum NodeStatus {
    /// Registered but no metrics/heartbeat observed yet.
    #[default]
    Registered,
    /// Reporting normally within the liveness window.
    Healthy,
    /// Reachable but breaching one or more degradation thresholds.
    Degraded,
    /// No heartbeat within the liveness window.
    Unreachable,
    /// Administratively drained.
    Draining,
}

/// Static description of a node, captured at registration time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeInfo {
    pub node_id: NodeId,
    pub hostname: String,
    pub ip_address: String,
    pub role: NodeRole,
    pub cpu_cores: u32,
    pub total_memory_bytes: u64,
    pub total_disk_bytes: u64,
    pub agent_version: String,
    pub kernel_version: String,
    pub os: String,
    pub started_at_unix: u64,
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
}

/// Aggregate + per-core CPU utilisation, as percentages in `0..=100`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct CpuMetrics {
    pub usage_percent: f64,
    #[serde(default)]
    pub per_core_percent: Vec<f64>,
}

/// Physical + swap memory usage in bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MemoryMetrics {
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub available_bytes: u64,
    pub swap_total_bytes: u64,
    pub swap_used_bytes: u64,
}

impl MemoryMetrics {
    /// Fraction of physical memory in use, in `0.0..=1.0`.
    pub fn used_fraction(&self) -> f64 {
        if self.total_bytes == 0 {
            0.0
        } else {
            self.used_bytes as f64 / self.total_bytes as f64
        }
    }
}

/// Classic 1/5/15-minute load averages.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct LoadMetrics {
    pub one: f64,
    pub five: f64,
    pub fifteen: f64,
}

/// Per-device disk capacity and throughput.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct DiskMetrics {
    pub device: String,
    pub mount_point: String,
    pub fs_type: String,
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub read_bytes_per_sec: u64,
    pub write_bytes_per_sec: u64,
    pub io_utilization_percent: f64,
}

impl DiskMetrics {
    /// Fraction of capacity used, in `0.0..=1.0`.
    pub fn used_fraction(&self) -> f64 {
        if self.total_bytes == 0 {
            0.0
        } else {
            let used = self.total_bytes.saturating_sub(self.available_bytes);
            used as f64 / self.total_bytes as f64
        }
    }
}

/// Aggregate network throughput/error counters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct NetworkMetrics {
    pub rx_bytes_per_sec: u64,
    pub tx_bytes_per_sec: u64,
    pub rx_errors: u64,
    pub tx_errors: u64,
}

/// Health of a single managed filesystem as seen from a node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct FilesystemMetrics {
    pub name: String,
    pub mount_point: String,
    pub mounted: bool,
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub inodes_total: u64,
    pub inodes_used: u64,
}

/// One complete resource sample from a node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetricsReport {
    pub node_id: NodeId,
    pub timestamp_unix: u64,
    pub cpu: CpuMetrics,
    pub memory: MemoryMetrics,
    pub load: LoadMetrics,
    #[serde(default)]
    pub disks: Vec<DiskMetrics>,
    pub network: NetworkMetrics,
    #[serde(default)]
    pub filesystems: Vec<FilesystemMetrics>,
}

/// Action requested by a deploy command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DeployAction {
    #[default]
    Install,
    Upgrade,
    Rollback,
    Remove,
}

/// A request to install/upgrade a filesystem software component on a node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeploySpec {
    pub deployment_id: String,
    pub action: DeployAction,
    pub component: String,
    pub version: String,
    pub target_path: String,
    #[serde(default)]
    pub options: BTreeMap<String, String>,
}

/// Action requested by a filesystem command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FsAction {
    #[default]
    Mount,
    Unmount,
    Remount,
    Check,
    Format,
}

/// A request to perform a mount/unmount/check operation on a node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsSpec {
    pub command_id: String,
    pub action: FsAction,
    pub device: String,
    pub mount_point: String,
    pub fs_type: String,
    #[serde(default)]
    pub mount_options: Vec<String>,
    #[serde(default)]
    pub force: bool,
}

/// Terminal outcome of a command executed by an agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandOutcome {
    pub command_id: String,
    pub node_id: NodeId,
    pub success: bool,
    pub exit_code: i32,
    pub message: String,
    pub stdout: String,
    pub stderr: String,
    pub completed_at_unix: u64,
}

/// The daemon's complete record for one node: static info plus the most recent
/// sample and derived status. This is what the REST API and CLI render.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeRecord {
    pub info: NodeInfo,
    pub status: NodeStatus,
    pub last_seen_unix: u64,
    #[serde(default)]
    pub latest_metrics: Option<MetricsReport>,
}

impl NodeRecord {
    /// Create a freshly-registered record with no metrics yet.
    pub fn new(info: NodeInfo) -> Self {
        NodeRecord {
            last_seen_unix: now_unix(),
            status: NodeStatus::Registered,
            latest_metrics: None,
            info,
        }
    }
}

// ---------------------------------------------------------------------------
// Benchmarking result types (produced by hpc-bench, rendered by hpc-cli)
// ---------------------------------------------------------------------------

/// The category of I/O being benchmarked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IoPattern {
    SequentialRead,
    SequentialWrite,
    RandomRead,
    RandomWrite,
}

impl std::fmt::Display for IoPattern {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            IoPattern::SequentialRead => "sequential_read",
            IoPattern::SequentialWrite => "sequential_write",
            IoPattern::RandomRead => "random_read",
            IoPattern::RandomWrite => "random_write",
        };
        f.write_str(s)
    }
}

/// Latency distribution (microseconds) captured during a benchmark run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct LatencyStats {
    pub min_us: u64,
    pub p50_us: u64,
    pub p90_us: u64,
    pub p99_us: u64,
    pub p999_us: u64,
    pub max_us: u64,
    pub mean_us: u64,
    pub samples: u64,
}

/// Result of a single benchmark scenario.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchResult {
    pub pattern: IoPattern,
    pub block_size_bytes: u64,
    pub total_bytes: u64,
    pub duration_secs: f64,
    pub throughput_mib_s: f64,
    pub iops: f64,
    pub latency: LatencyStats,
}

/// A collection of scenario results plus the environment they ran in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchReport {
    pub target_path: String,
    pub started_at_unix: u64,
    pub results: Vec<BenchResult>,
}
