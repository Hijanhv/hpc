//! Degradation detection and the daemon scrape loop.
//!
//! [`Detector`] encodes the policy: given a node's latest sample and the
//! configured thresholds, it returns a list of human-readable reasons the node
//! is unhealthy (empty ⇒ healthy). [`run`] polls the daemon on an interval,
//! feeds the results to the [`Metrics`] exporter, and logs the *transitions*
//! (newly degraded / recovered) rather than spamming on every scrape.

use std::collections::HashSet;
use std::time::Duration;

use hpc_core::config::{MonitorConfig, Thresholds};
use hpc_core::types::{NodeRecord, NodeStatus};

use crate::metrics::Metrics;

/// Applies threshold policy to a node record.
#[derive(Debug, Clone)]
pub struct Detector {
    thresholds: Thresholds,
}

impl Detector {
    /// Create a detector from configured thresholds.
    pub fn new(thresholds: Thresholds) -> Self {
        Detector { thresholds }
    }

    /// Return every reason `node` is considered degraded (empty ⇒ healthy).
    pub fn evaluate(&self, node: &NodeRecord) -> Vec<String> {
        let mut reasons = Vec::new();

        if node.status == NodeStatus::Unreachable {
            reasons.push("unreachable (no heartbeat)".to_string());
            // No point inspecting stale metrics further.
            return reasons;
        }

        let Some(m) = &node.latest_metrics else {
            return reasons;
        };

        let cpu_frac = m.cpu.usage_percent / 100.0;
        if cpu_frac >= self.thresholds.cpu_degraded {
            reasons.push(format!(
                "cpu {:.0}% >= {:.0}%",
                m.cpu.usage_percent,
                self.thresholds.cpu_degraded * 100.0
            ));
        }

        let mem_frac = m.memory.used_fraction();
        if mem_frac >= self.thresholds.memory_degraded {
            reasons.push(format!(
                "memory {:.0}% >= {:.0}%",
                mem_frac * 100.0,
                self.thresholds.memory_degraded * 100.0
            ));
        }

        for d in &m.disks {
            let frac = d.used_fraction();
            if frac >= self.thresholds.disk_degraded {
                reasons.push(format!(
                    "disk {} {:.0}% >= {:.0}%",
                    d.mount_point,
                    frac * 100.0,
                    self.thresholds.disk_degraded * 100.0
                ));
            }
        }

        let cores = node.info.cpu_cores.max(1) as f64;
        let load_per_core = m.load.one / cores;
        if load_per_core >= self.thresholds.load_per_core_degraded {
            reasons.push(format!(
                "load/core {:.2} >= {:.2}",
                load_per_core, self.thresholds.load_per_core_degraded
            ));
        }

        reasons
    }
}

/// Poll the daemon forever, updating metrics and logging health transitions.
pub async fn run(cfg: MonitorConfig, metrics: Metrics) {
    let detector = Detector::new(cfg.thresholds.clone());
    let http = match reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "failed to build HTTP client; scrape loop disabled");
            return;
        }
    };
    let url = format!("{}/api/v1/nodes", cfg.daemon_api.trim_end_matches('/'));

    let mut ticker = tokio::time::interval(cfg.scrape_interval);
    let mut previously_degraded: HashSet<String> = HashSet::new();

    loop {
        ticker.tick().await;
        match scrape(&http, &url).await {
            Ok(nodes) => {
                let degraded = metrics.update(&nodes, &detector);
                let current: HashSet<String> = degraded.iter().map(|(id, _)| id.clone()).collect();

                for (id, reasons) in &degraded {
                    if !previously_degraded.contains(id) {
                        tracing::warn!(node = %id, reasons = %reasons.join("; "), "node degraded");
                    }
                }
                for id in previously_degraded.difference(&current) {
                    tracing::info!(node = %id, "node recovered");
                }
                previously_degraded = current;

                tracing::debug!(
                    nodes = nodes.len(),
                    degraded = degraded.len(),
                    "scrape complete"
                );
            }
            Err(e) => {
                metrics.record_scrape_error();
                tracing::warn!(error = %e, url = %url, "scrape failed");
            }
        }
    }
}

async fn scrape(http: &reqwest::Client, url: &str) -> anyhow::Result<Vec<NodeRecord>> {
    let resp = http.get(url).send().await?.error_for_status()?;
    let nodes = resp.json::<Vec<NodeRecord>>().await?;
    Ok(nodes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hpc_core::types::{now_unix, MemoryMetrics, MetricsReport, NodeInfo, NodeRole};

    fn node_with_mem(used: u64, total: u64) -> NodeRecord {
        let info = NodeInfo {
            node_id: "n1".into(),
            hostname: "n1".into(),
            ip_address: "10.0.0.1".into(),
            role: NodeRole::Storage,
            cpu_cores: 4,
            total_memory_bytes: total,
            total_disk_bytes: 0,
            agent_version: "0.1.0".into(),
            kernel_version: "6".into(),
            os: "linux".into(),
            started_at_unix: 0,
            labels: Default::default(),
        };
        let mut rec = NodeRecord::new(info);
        rec.status = NodeStatus::Healthy;
        rec.latest_metrics = Some(MetricsReport {
            node_id: "n1".into(),
            timestamp_unix: now_unix(),
            cpu: Default::default(),
            memory: MemoryMetrics {
                total_bytes: total,
                used_bytes: used,
                available_bytes: total - used,
                ..Default::default()
            },
            load: Default::default(),
            disks: vec![],
            network: Default::default(),
            filesystems: vec![],
        });
        rec
    }

    #[test]
    fn flags_high_memory() {
        let det = Detector::new(Thresholds::default());
        assert!(det.evaluate(&node_with_mem(50, 100)).is_empty());
        let reasons = det.evaluate(&node_with_mem(95, 100));
        assert_eq!(reasons.len(), 1);
        assert!(reasons[0].contains("memory"));
    }

    #[test]
    fn unreachable_short_circuits() {
        let det = Detector::new(Thresholds::default());
        let mut rec = node_with_mem(10, 100);
        rec.status = NodeStatus::Unreachable;
        let reasons = det.evaluate(&rec);
        assert_eq!(reasons, vec!["unreachable (no heartbeat)".to_string()]);
    }
}
