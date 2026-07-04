//! Prometheus metric definitions and the exposition endpoint.
//!
//! All metrics live in a private [`Registry`] owned by [`Metrics`]. Per-node
//! gauges are cleared and repopulated on every scrape so that nodes which leave
//! the cluster stop being exported (avoiding stale series). The struct is cheap
//! to clone — every prometheus metric is internally reference-counted.

use hpc_core::types::{NodeRecord, NodeStatus};
use prometheus::{
    Encoder, GaugeVec, IntCounter, IntGauge, IntGaugeVec, Opts, Registry, TextEncoder,
};

use crate::health::Detector;

/// Owns the registry and every exported metric.
#[derive(Clone)]
pub struct Metrics {
    registry: Registry,
    nodes_total: IntGauge,
    nodes_healthy: IntGauge,
    nodes_degraded: IntGauge,
    nodes_unreachable: IntGauge,
    node_up: IntGaugeVec,
    node_degraded: IntGaugeVec,
    cpu_percent: GaugeVec,
    memory_ratio: GaugeVec,
    load1: GaugeVec,
    disk_ratio: GaugeVec,
    scrape_errors: IntCounter,
}

impl std::fmt::Debug for Metrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Metrics").finish_non_exhaustive()
    }
}

impl Metrics {
    /// Build and register every metric. Returns an error if two metrics collide
    /// (a programming error, surfaced rather than panicked).
    pub fn new() -> hpc_core::Result<Self> {
        let registry = Registry::new();

        let gauge = |name: &str, help: &str| -> hpc_core::Result<IntGauge> {
            let g = IntGauge::with_opts(Opts::new(name, help)).map_err(reg_err)?;
            registry.register(Box::new(g.clone())).map_err(reg_err)?;
            Ok(g)
        };
        let int_vec = |name: &str, help: &str, labels: &[&str]| -> hpc_core::Result<IntGaugeVec> {
            let g = IntGaugeVec::new(Opts::new(name, help), labels).map_err(reg_err)?;
            registry.register(Box::new(g.clone())).map_err(reg_err)?;
            Ok(g)
        };
        let float_vec = |name: &str, help: &str, labels: &[&str]| -> hpc_core::Result<GaugeVec> {
            let g = GaugeVec::new(Opts::new(name, help), labels).map_err(reg_err)?;
            registry.register(Box::new(g.clone())).map_err(reg_err)?;
            Ok(g)
        };

        let scrape_errors = IntCounter::with_opts(Opts::new(
            "hpc_scrape_errors_total",
            "daemon scrape failures",
        ))
        .map_err(reg_err)?;
        registry
            .register(Box::new(scrape_errors.clone()))
            .map_err(reg_err)?;

        Ok(Metrics {
            nodes_total: gauge("hpc_nodes_total", "total registered nodes")?,
            nodes_healthy: gauge("hpc_nodes_healthy", "nodes reporting healthy")?,
            nodes_degraded: gauge("hpc_nodes_degraded", "nodes in a degraded state")?,
            nodes_unreachable: gauge("hpc_nodes_unreachable", "nodes with no heartbeat")?,
            node_up: int_vec("hpc_node_up", "1 if the node is reachable", &["node"])?,
            node_degraded: int_vec(
                "hpc_node_degraded",
                "1 if the node breaches a threshold",
                &["node"],
            )?,
            cpu_percent: float_vec("hpc_node_cpu_percent", "node CPU usage percent", &["node"])?,
            memory_ratio: float_vec(
                "hpc_node_memory_used_ratio",
                "node memory used fraction",
                &["node"],
            )?,
            load1: float_vec("hpc_node_load1", "node 1-minute load average", &["node"])?,
            disk_ratio: float_vec(
                "hpc_node_disk_used_ratio",
                "filesystem used fraction",
                &["node", "mount"],
            )?,
            scrape_errors,
            registry,
        })
    }

    /// Note a failed scrape of the daemon.
    pub fn record_scrape_error(&self) {
        self.scrape_errors.inc();
    }

    /// Replace all per-node series with the current snapshot and refresh the
    /// aggregate counters. Returns the set of nodes judged degraded, with the
    /// human-readable reasons, so the caller can log transitions.
    pub fn update(&self, nodes: &[NodeRecord], detector: &Detector) -> Vec<(String, Vec<String>)> {
        // Clear per-node vectors so departed nodes are not left as stale series.
        self.node_up.reset();
        self.node_degraded.reset();
        self.cpu_percent.reset();
        self.memory_ratio.reset();
        self.load1.reset();
        self.disk_ratio.reset();

        let (mut healthy, mut degraded, mut unreachable) = (0i64, 0i64, 0i64);
        let mut degraded_nodes = Vec::new();

        for node in nodes {
            let id = node.info.node_id.as_str();
            let up = node.status != NodeStatus::Unreachable;
            self.node_up.with_label_values(&[id]).set(up as i64);

            match node.status {
                NodeStatus::Unreachable => unreachable += 1,
                NodeStatus::Degraded => degraded += 1,
                _ => healthy += 1,
            }

            if let Some(m) = &node.latest_metrics {
                self.cpu_percent
                    .with_label_values(&[id])
                    .set(m.cpu.usage_percent);
                self.memory_ratio
                    .with_label_values(&[id])
                    .set(m.memory.used_fraction());
                self.load1.with_label_values(&[id]).set(m.load.one);
                for d in &m.disks {
                    self.disk_ratio
                        .with_label_values(&[id, &d.mount_point])
                        .set(d.used_fraction());
                }
            }

            let reasons = detector.evaluate(node);
            let is_degraded = !reasons.is_empty();
            self.node_degraded
                .with_label_values(&[id])
                .set(is_degraded as i64);
            if is_degraded {
                degraded_nodes.push((node.info.node_id.clone(), reasons));
            }
        }

        self.nodes_total.set(nodes.len() as i64);
        self.nodes_healthy.set(healthy);
        self.nodes_degraded.set(degraded);
        self.nodes_unreachable.set(unreachable);

        degraded_nodes
    }

    /// Render the registry in Prometheus text exposition format.
    pub fn render(&self) -> hpc_core::Result<String> {
        let mut buffer = Vec::new();
        let encoder = TextEncoder::new();
        encoder
            .encode(&self.registry.gather(), &mut buffer)
            .map_err(reg_err)?;
        String::from_utf8(buffer)
            .map_err(|e| hpc_core::HpcError::Metrics(format!("invalid utf8 in metrics: {e}")))
    }
}

fn reg_err(e: impl std::fmt::Display) -> hpc_core::HpcError {
    hpc_core::HpcError::Metrics(e.to_string())
}
