//! The diagnostic data model: structured error reports, the platform
//! description, detected anomalies, and the top-level bundle that ties them
//! together into a single machine-readable JSON document.

use std::collections::BTreeMap;

use hpc_core::types::now_unix;
use serde::{Deserialize, Serialize};

use crate::proc::{DiskStat, KernelInfo, MemInfo, MountEntry, NetDev};

/// Severity of a report or detected anomaly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    Error,
    Fatal,
}

/// A structured crash/error report contributed by any crate in the workspace.
///
/// The daemon, agent, monitor, etc. build these when something goes wrong and
/// hand them to [`crate::collect_with_reports`], which embeds them in the
/// bundle alongside the live system snapshot — so a report is always paired
/// with the machine state at the time it was gathered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagReport {
    /// Originating crate or subsystem, e.g. `"hpc-daemon"`.
    pub source: String,
    pub severity: Severity,
    /// A short, stable machine key for the event, e.g. `"grpc_disconnect"`.
    pub kind: String,
    /// Human-readable description.
    pub message: String,
    /// Arbitrary structured context (node id, path, error chain, …).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub context: BTreeMap<String, String>,
    pub captured_at_unix: u64,
}

impl DiagReport {
    /// Build a report with the given severity.
    pub fn new(
        source: impl Into<String>,
        severity: Severity,
        kind: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        DiagReport {
            source: source.into(),
            severity,
            kind: kind.into(),
            message: message.into(),
            context: BTreeMap::new(),
            captured_at_unix: now_unix(),
        }
    }

    /// Shorthand for a [`Severity::Warning`] report.
    pub fn warning(
        source: impl Into<String>,
        kind: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self::new(source, Severity::Warning, kind, message)
    }

    /// Shorthand for a [`Severity::Error`] report.
    pub fn error(
        source: impl Into<String>,
        kind: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self::new(source, Severity::Error, kind, message)
    }

    /// Attach a key/value pair to the report's context, builder-style.
    #[must_use]
    pub fn with_context(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.context.insert(key.into(), value.into());
        self
    }
}

/// Where the bundle was captured.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostInfo {
    pub hostname: String,
    /// Compile-time target OS (`std::env::consts::OS`).
    pub os: String,
    /// Compile-time target architecture (`std::env::consts::ARCH`).
    pub arch: String,
}

impl HostInfo {
    /// Gather host identity without failing: unknown fields fall back to
    /// sensible placeholders.
    pub fn collect() -> Self {
        let hostname = std::fs::read_to_string("/proc/sys/kernel/hostname")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .or_else(|| std::env::var("HOSTNAME").ok())
            .unwrap_or_else(|| "unknown".to_string());
        HostInfo {
            hostname,
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
        }
    }
}

/// The platform description used for cross-platform bug analysis: kernel
/// identity, the distinct filesystem types in use, and the raw mount table.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformInfo {
    pub kernel: KernelInfo,
    /// Distinct filesystem types currently mounted, sorted.
    pub filesystems: Vec<String>,
    pub mounts: Vec<MountEntry>,
}

/// A detected condition that could plausibly explain a bug or behavioural
/// difference between platforms.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Anomaly {
    pub severity: Severity,
    /// Machine-readable category, e.g. `"memory_pressure"`.
    pub category: String,
    pub detail: String,
}

/// The complete diagnostic bundle — the single JSON artifact `hpc-diag collect`
/// emits.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiagnosticBundle {
    /// Schema version, so downstream tooling can evolve safely.
    pub schema_version: u32,
    pub generated_at_unix: u64,
    pub host: HostInfo,
    pub platform: PlatformInfo,
    pub memory: Option<MemInfo>,
    pub disks: Vec<DiskStat>,
    pub network: Vec<NetDev>,
    #[serde(default)]
    pub reports: Vec<DiagReport>,
    pub anomalies: Vec<Anomaly>,
}

impl DiagnosticBundle {
    /// The current bundle schema version.
    pub const SCHEMA_VERSION: u32 = 1;

    /// Serialize to pretty-printed JSON.
    pub fn to_json_pretty(&self) -> hpc_core::Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    /// Serialize to compact single-line JSON.
    pub fn to_json(&self) -> hpc_core::Result<String> {
        Ok(serde_json::to_string(self)?)
    }
}

/// Filesystem types generally regarded as ephemeral: state written to them does
/// not survive a reboot, a classic source of "works until restart" bugs.
const EPHEMERAL_FS: &[&str] = &["tmpfs", "ramfs", "overlay", "devtmpfs"];

/// Inspect the collected snapshot and flag platform differences / conditions
/// that could explain bugs. This is intentionally heuristic — every anomaly is
/// a *hypothesis* an engineer can confirm, not a verdict.
pub fn detect_anomalies(memory: Option<&MemInfo>, platform: &PlatformInfo) -> Vec<Anomaly> {
    let mut out = Vec::new();

    if let Some(mem) = memory {
        let avail = mem.available_fraction();
        if mem.mem_total_kb > 0 && avail < 0.10 {
            out.push(Anomaly {
                severity: Severity::Error,
                category: "memory_pressure".into(),
                detail: format!(
                    "only {:.1}% of memory available; latency spikes, allocator failures and OOM kills are plausible here",
                    avail * 100.0
                ),
            });
        }
        if mem.swap_total_kb == 0 {
            out.push(Anomaly {
                severity: Severity::Info,
                category: "no_swap".into(),
                detail:
                    "no swap configured; memory pressure surfaces as hard OOM rather than slowdown"
                        .into(),
            });
        }
    }

    // Old kernels lack newer I/O syscalls (e.g. io_uring landed in 5.1),
    // a common source of "works on my box" divergence.
    if let Some((major, minor)) = platform.kernel.version_pair() {
        if major < 5 || (major == 5 && minor < 1) {
            out.push(Anomaly {
                severity: Severity::Warning,
                category: "old_kernel".into(),
                detail: format!(
                    "kernel {} predates io_uring (5.1); async I/O and syscall behaviour may differ from newer nodes",
                    platform.kernel.release
                ),
            });
        }
    }

    // Heterogeneous filesystems are a prime multi-platform bug surface:
    // fsync semantics, atime, and rename atomicity all vary by fs.
    if platform.filesystems.len() > 1 {
        out.push(Anomaly {
            severity: Severity::Info,
            category: "heterogeneous_filesystems".into(),
            detail: format!(
                "multiple filesystem types in use ({}); durability and atomicity semantics can differ per mount",
                platform.filesystems.join(", ")
            ),
        });
    }

    for m in &platform.mounts {
        if EPHEMERAL_FS.contains(&m.fs_type.as_str()) && m.mount_point.starts_with("/var") {
            out.push(Anomaly {
                severity: Severity::Warning,
                category: "ephemeral_state_dir".into(),
                detail: format!(
                    "{} is {} (ephemeral); state written there is lost on reboot",
                    m.mount_point, m.fs_type
                ),
            });
        }
        if m.mount_point == "/" && m.options.iter().any(|o| o == "ro") {
            out.push(Anomaly {
                severity: Severity::Error,
                category: "root_read_only".into(),
                detail: "root filesystem is mounted read-only; writes will fail".into(),
            });
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plat(kernel_release: &str, fs: &[&str]) -> PlatformInfo {
        PlatformInfo {
            kernel: KernelInfo {
                raw: format!("Linux version {kernel_release}"),
                release: kernel_release.to_string(),
            },
            filesystems: fs.iter().map(|s| s.to_string()).collect(),
            mounts: Vec::new(),
        }
    }

    #[test]
    fn flags_memory_pressure() {
        let mem = MemInfo {
            mem_total_kb: 1000,
            mem_available_kb: 50,
            ..Default::default()
        };
        let a = detect_anomalies(Some(&mem), &plat("6.5.0", &["ext4"]));
        assert!(a.iter().any(|x| x.category == "memory_pressure"));
    }

    #[test]
    fn flags_old_kernel() {
        let a = detect_anomalies(None, &plat("4.19.0", &["ext4"]));
        assert!(a
            .iter()
            .any(|x| x.category == "old_kernel" && x.severity == Severity::Warning));
    }

    #[test]
    fn modern_kernel_is_not_flagged_old() {
        let a = detect_anomalies(None, &plat("6.5.0", &["ext4"]));
        assert!(!a.iter().any(|x| x.category == "old_kernel"));
    }

    #[test]
    fn flags_heterogeneous_filesystems() {
        let a = detect_anomalies(None, &plat("6.5.0", &["ext4", "xfs", "tmpfs"]));
        assert!(a.iter().any(|x| x.category == "heterogeneous_filesystems"));
    }

    #[test]
    fn report_builder_attaches_context() {
        let r = DiagReport::error("hpc-daemon", "grpc_disconnect", "peer went away")
            .with_context("node", "storage-01");
        assert_eq!(r.severity, Severity::Error);
        assert_eq!(
            r.context.get("node").map(String::as_str),
            Some("storage-01")
        );
    }
}
