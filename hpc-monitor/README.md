# hpc-monitor

A Prometheus exporter and degradation detector for the cluster. It scrapes the
daemon's REST API on an interval, translates cluster state into Prometheus
metrics, applies threshold policy to flag unhealthy nodes, and exposes
everything at `/metrics`.

## What it does

- **Scrape loop** (`src/health.rs`) — polls `GET /api/v1/nodes` every
  `scrape_interval`, feeds results to the exporter, and logs health
  *transitions* (newly degraded / recovered) rather than spamming every scrape.
- **Degradation detection** (`Detector`) — a node is degraded if it is
  unreachable, or its latest sample breaches any configured threshold: CPU %,
  memory fraction, per-filesystem disk fraction, or load-average-per-core. Each
  breach yields a human-readable reason that is logged and drives the
  `hpc_node_degraded` gauge.
- **Exposition** (`src/metrics.rs`) — an `axum` server exposes `/metrics` in
  Prometheus text format and `/health`.

## Exported metrics

| Metric | Type | Labels | Meaning |
|--------|------|--------|---------|
| `hpc_nodes_total` | gauge | — | Registered nodes. |
| `hpc_nodes_healthy` / `_degraded` / `_unreachable` | gauge | — | Aggregate status counts. |
| `hpc_node_up` | gauge | `node` | 1 if reachable, else 0. |
| `hpc_node_degraded` | gauge | `node` | 1 if any threshold is breached. |
| `hpc_node_cpu_percent` | gauge | `node` | CPU usage percent. |
| `hpc_node_memory_used_ratio` | gauge | `node` | Memory used fraction. |
| `hpc_node_load1` | gauge | `node` | 1-minute load average. |
| `hpc_node_disk_used_ratio` | gauge | `node`, `mount` | Filesystem used fraction. |
| `hpc_scrape_errors_total` | counter | — | Failed daemon scrapes. |

Per-node series are cleared and repopulated on every scrape, so nodes that leave
the cluster stop being exported (no stale series).

## Run it

```bash
hpc-monitor --config configs/monitor.toml   # serves :9090/metrics
curl -s localhost:9090/metrics | grep hpc_
```

Point Prometheus at `hpc-monitor:9090`. See
[`configs/monitor.toml`](../configs/monitor.toml) for thresholds and the daemon
API URL.
