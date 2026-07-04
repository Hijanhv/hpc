//! Strongly-typed configuration for every binary, loaded from TOML.
//!
//! Each daemon/agent/monitor has its own top-level config struct. All fields
//! have sensible defaults via `#[serde(default)]` so a minimal config file
//! (or none at all) still yields a working process. Durations are parsed with
//! `humantime` (e.g. `"15s"`, `"2m"`).

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::{HpcError, Result};
use crate::types::NodeRole;

/// Read and parse a TOML config of type `T` from `path`.
///
/// Returns [`HpcError::Io`] if the file cannot be read and
/// [`HpcError::ConfigParse`] if it is not valid TOML for `T`.
pub fn load<T: for<'de> Deserialize<'de>>(path: impl AsRef<Path>) -> Result<T> {
    let path = path.as_ref();
    let raw = std::fs::read_to_string(path).map_err(|e| HpcError::io_at(path, e))?;
    let value = toml::from_str(&raw)?;
    Ok(value)
}

/// Load config of type `T` from `path` if it exists, otherwise return
/// `T::default()`. Useful for binaries where a config file is optional.
pub fn load_or_default<T>(path: impl AsRef<Path>) -> Result<T>
where
    T: for<'de> Deserialize<'de> + Default,
{
    let path = path.as_ref();
    if path.exists() {
        load(path)
    } else {
        tracing::debug!(path = %path.display(), "config file absent; using defaults");
        Ok(T::default())
    }
}

/// Shared logging/tracing configuration embedded in each binary's config.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LogConfig {
    /// `tracing_subscriber` env-filter directive, e.g. `"info,hpc_daemon=debug"`.
    pub filter: String,
    /// Emit machine-readable JSON logs instead of the human formatter.
    pub json: bool,
    /// Include source file/line in log records.
    pub with_location: bool,
}

impl Default for LogConfig {
    fn default() -> Self {
        LogConfig {
            filter: "info".to_string(),
            json: false,
            with_location: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Daemon
// ---------------------------------------------------------------------------

/// Top-level configuration for `hpc-daemon`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DaemonConfig {
    /// Address the gRPC control server listens on (agents dial this).
    pub grpc_addr: SocketAddr,
    /// Address the REST/JSON API listens on (CLI and monitor call this).
    pub http_addr: SocketAddr,
    /// Directory for persistent state (the redb database lives here).
    pub data_dir: PathBuf,
    /// A node with no heartbeat within this window is marked `Unreachable`.
    #[serde(with = "humantime_serde")]
    pub node_timeout: Duration,
    /// Cadence the daemon directs agents to report metrics at.
    #[serde(with = "humantime_serde")]
    pub metrics_interval: Duration,
    /// Degradation thresholds applied to incoming metrics.
    pub thresholds: Thresholds,
    pub log: LogConfig,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        DaemonConfig {
            grpc_addr: "0.0.0.0:7443"
                .parse()
                .unwrap_or_else(|_| default_addr(7443)),
            http_addr: "0.0.0.0:8080"
                .parse()
                .unwrap_or_else(|_| default_addr(8080)),
            data_dir: PathBuf::from("/var/lib/hpc"),
            node_timeout: Duration::from_secs(30),
            metrics_interval: Duration::from_secs(10),
            thresholds: Thresholds::default(),
            log: LogConfig::default(),
        }
    }
}

impl DaemonConfig {
    /// Path to the embedded state database.
    pub fn store_path(&self) -> PathBuf {
        self.data_dir.join("cluster.redb")
    }

    /// Validate cross-field invariants that `serde` cannot express.
    pub fn validate(&self) -> Result<()> {
        if self.node_timeout < self.metrics_interval {
            return Err(HpcError::ConfigInvalid(format!(
                "node_timeout ({:?}) must be >= metrics_interval ({:?})",
                self.node_timeout, self.metrics_interval
            )));
        }
        self.thresholds.validate()
    }
}

/// Degradation thresholds used by the daemon and monitor to classify health.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Thresholds {
    /// CPU usage fraction (0..1) above which a node is considered degraded.
    pub cpu_degraded: f64,
    /// Memory usage fraction (0..1) above which a node is considered degraded.
    pub memory_degraded: f64,
    /// Disk usage fraction (0..1) above which a filesystem is degraded.
    pub disk_degraded: f64,
    /// Load-average-per-core ratio above which a node is degraded.
    pub load_per_core_degraded: f64,
}

impl Default for Thresholds {
    fn default() -> Self {
        Thresholds {
            cpu_degraded: 0.90,
            memory_degraded: 0.90,
            disk_degraded: 0.85,
            load_per_core_degraded: 2.0,
        }
    }
}

impl Thresholds {
    /// Validate that all fractional thresholds fall within `0.0..=1.0`.
    pub fn validate(&self) -> Result<()> {
        for (name, v) in [
            ("cpu_degraded", self.cpu_degraded),
            ("memory_degraded", self.memory_degraded),
            ("disk_degraded", self.disk_degraded),
        ] {
            if !(0.0..=1.0).contains(&v) {
                return Err(HpcError::ConfigInvalid(format!(
                    "{name} must be within 0.0..=1.0, got {v}"
                )));
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Agent
// ---------------------------------------------------------------------------

/// Top-level configuration for `hpc-agent`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentConfig {
    /// `http://host:port` URL of the daemon's gRPC endpoint.
    pub daemon_endpoint: String,
    /// Stable node id; defaults to the machine hostname when empty.
    pub node_id: String,
    /// Advertised role of this node.
    pub role: NodeRole,
    /// Free-form scheduling labels advertised at registration.
    #[serde(default)]
    pub labels: std::collections::BTreeMap<String, String>,
    /// How often to sample and report metrics (may be overridden by the daemon).
    #[serde(with = "humantime_serde")]
    pub metrics_interval: Duration,
    /// Heartbeat cadence.
    #[serde(with = "humantime_serde")]
    pub heartbeat_interval: Duration,
    /// Base backoff for reconnecting to the daemon after a failure.
    #[serde(with = "humantime_serde")]
    pub reconnect_backoff: Duration,
    /// Whether the agent is permitted to actually run mount/deploy commands.
    /// When false (the default) commands are validated and logged but not
    /// executed — a safety guard for demos and dry-runs.
    pub allow_exec: bool,
    pub log: LogConfig,
}

impl Default for AgentConfig {
    fn default() -> Self {
        AgentConfig {
            daemon_endpoint: "http://127.0.0.1:7443".to_string(),
            node_id: String::new(),
            role: NodeRole::Storage,
            labels: Default::default(),
            metrics_interval: Duration::from_secs(10),
            heartbeat_interval: Duration::from_secs(5),
            reconnect_backoff: Duration::from_secs(2),
            allow_exec: false,
            log: LogConfig::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// Monitor
// ---------------------------------------------------------------------------

/// Top-level configuration for `hpc-monitor`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MonitorConfig {
    /// Base URL of the daemon REST API to scrape, e.g. `http://127.0.0.1:8080`.
    pub daemon_api: String,
    /// Address to expose the Prometheus `/metrics` endpoint on.
    pub metrics_addr: SocketAddr,
    /// How often to poll the daemon for cluster state.
    #[serde(with = "humantime_serde")]
    pub scrape_interval: Duration,
    /// Degradation thresholds (shared shape with the daemon).
    pub thresholds: Thresholds,
    pub log: LogConfig,
}

impl Default for MonitorConfig {
    fn default() -> Self {
        MonitorConfig {
            daemon_api: "http://127.0.0.1:8080".to_string(),
            metrics_addr: "0.0.0.0:9090"
                .parse()
                .unwrap_or_else(|_| default_addr(9090)),
            scrape_interval: Duration::from_secs(15),
            thresholds: Thresholds::default(),
            log: LogConfig::default(),
        }
    }
}

/// Fallback socket address constructor that never panics.
fn default_addr(port: u16) -> SocketAddr {
    use std::net::{IpAddr, Ipv4Addr};
    SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_defaults_are_valid() {
        assert!(DaemonConfig::default().validate().is_ok());
    }

    #[test]
    fn node_timeout_must_exceed_metrics_interval() {
        let cfg = DaemonConfig {
            node_timeout: Duration::from_secs(1),
            metrics_interval: Duration::from_secs(10),
            ..Default::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn parses_partial_toml_with_defaults() {
        let toml_src = r#"
            grpc_addr = "127.0.0.1:9000"
            [log]
            filter = "debug"
        "#;
        let cfg: DaemonConfig = toml::from_str(toml_src).expect("valid toml");
        assert_eq!(cfg.grpc_addr.port(), 9000);
        assert_eq!(cfg.log.filter, "debug");
        // untouched fields keep their defaults
        assert_eq!(cfg.http_addr.port(), 8080);
    }

    #[test]
    fn out_of_range_threshold_is_rejected() {
        let cfg = DaemonConfig {
            thresholds: Thresholds {
                cpu_degraded: 1.5,
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(cfg.validate().is_err());
    }
}
