//! # hpc-diag
//!
//! Multi-platform diagnostics and bug-analysis tooling for the HPC framework.
//! It gathers a machine-readable snapshot of a node — memory, disk and network
//! counters from `/proc`, the kernel identity, and the mount table — folds in
//! any structured [`DiagReport`]s contributed by other crates, then runs a set
//! of heuristics that flag *platform differences and conditions that could
//! explain a bug* ([`detect_anomalies`]). The result is a single JSON
//! [`DiagnosticBundle`] you can attach to an incident or diff between two nodes
//! to see what actually differs.
//!
//! ## Resilience
//!
//! Collection never aborts on a missing source. Each collector that fails
//! (for instance, `/proc` does not exist on macOS or in a minimal container)
//! is recorded as a [`Severity::Warning`] report inside the bundle, so
//! `hpc-diag collect` always produces a well-formed document describing exactly
//! what it could and could not observe.
#![forbid(unsafe_code)]
#![warn(missing_debug_implementations)]

pub mod proc;
pub mod report;

use std::collections::BTreeSet;

use hpc_core::types::now_unix;

pub use report::{
    detect_anomalies, Anomaly, DiagReport, DiagnosticBundle, HostInfo, PlatformInfo, Severity,
};

/// Collect a full diagnostic bundle for the current host.
pub fn collect() -> DiagnosticBundle {
    collect_with_reports(Vec::new())
}

/// Collect a full diagnostic bundle, embedding `reports` contributed by other
/// crates alongside the live system snapshot.
pub fn collect_with_reports(mut reports: Vec<DiagReport>) -> DiagnosticBundle {
    let memory = match proc::collect_meminfo() {
        Ok(m) => Some(m),
        Err(e) => {
            reports.push(unavailable("/proc/meminfo", &e));
            None
        }
    };

    let disks = match proc::collect_diskstats() {
        Ok(d) => d,
        Err(e) => {
            reports.push(unavailable("/proc/diskstats", &e));
            Vec::new()
        }
    };

    let network = match proc::collect_net_dev() {
        Ok(n) => n,
        Err(e) => {
            reports.push(unavailable("/proc/net/dev", &e));
            Vec::new()
        }
    };

    let mounts = match proc::collect_mounts() {
        Ok(m) => m,
        Err(e) => {
            reports.push(unavailable("/proc/mounts", &e));
            Vec::new()
        }
    };

    let kernel = match proc::collect_kernel() {
        Ok(k) => k,
        Err(e) => {
            reports.push(unavailable("/proc/version", &e));
            proc::KernelInfo::default()
        }
    };

    // Distinct, sorted filesystem types currently mounted.
    let filesystems: Vec<String> = mounts
        .iter()
        .map(|m| m.fs_type.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();

    let platform = PlatformInfo {
        kernel,
        filesystems,
        mounts,
    };

    let anomalies = detect_anomalies(memory.as_ref(), &platform);

    DiagnosticBundle {
        schema_version: DiagnosticBundle::SCHEMA_VERSION,
        generated_at_unix: now_unix(),
        host: HostInfo::collect(),
        platform,
        memory,
        disks,
        network,
        reports,
        anomalies,
    }
}

/// Build the "collector unavailable" note recorded when a `/proc` source can't
/// be read (e.g. running off-Linux).
fn unavailable(source: &str, err: &hpc_core::HpcError) -> DiagReport {
    DiagReport::warning(
        "hpc-diag",
        "collector_unavailable",
        format!("could not read {source}: {err}"),
    )
    .with_context("source", source)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collect_always_yields_serializable_bundle() {
        // On a non-Linux host this exercises the resilience path: the bundle is
        // still well-formed and every failed collector shows up as a report.
        let bundle = collect();
        assert_eq!(bundle.schema_version, DiagnosticBundle::SCHEMA_VERSION);
        let json = bundle.to_json_pretty().expect("serialize");
        assert!(json.contains("\"schema_version\""));
    }

    #[test]
    fn contributed_reports_are_embedded() {
        let report = DiagReport::error("hpc-agent", "panic", "executor thread panicked")
            .with_context("node", "compute-07");
        let bundle = collect_with_reports(vec![report]);
        assert!(bundle
            .reports
            .iter()
            .any(|r| r.source == "hpc-agent" && r.kind == "panic"));
    }
}
