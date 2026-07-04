//! Node resource metrics collection.
//!
//! The collector blends two sources:
//!
//! * **[`sysinfo`]** for portable CPU / memory / load / disk-capacity figures
//!   and the static node description, and
//! * **`/proc`** (Linux only) for per-device I/O throughput (`/proc/diskstats`)
//!   and network throughput (`/proc/net/dev`), which sysinfo does not expose as
//!   rates. These counters are cumulative, so the collector keeps the previous
//!   sample and reports deltas divided by wall-clock elapsed.
//!
//! On non-Linux hosts (e.g. a macOS dev box) the `/proc`-derived rates are
//! simply zero; everything else still works, which keeps the agent runnable
//! anywhere for development.

use std::collections::HashMap;
use std::time::Instant;

use hpc_core::types::{
    now_unix, CpuMetrics, DiskMetrics, LoadMetrics, MemoryMetrics, MetricsReport, NetworkMetrics,
    NodeInfo, NodeRole,
};
use sysinfo::{Disks, System};

/// One collector per agent process; holds the previous cumulative counters so
/// throughput can be derived on each [`collect`](Collector::collect).
pub struct Collector {
    node_id: String,
    role: NodeRole,
    sys: System,
    prev: Option<Prev>,
}

impl std::fmt::Debug for Collector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Collector")
            .field("node_id", &self.node_id)
            .finish_non_exhaustive()
    }
}

struct Prev {
    at: Instant,
    /// device -> (read_bytes, write_bytes) cumulative
    disk: HashMap<String, (u64, u64)>,
    /// cumulative (rx_bytes, tx_bytes, rx_errors, tx_errors)
    net: (u64, u64, u64, u64),
}

impl Collector {
    /// Create a collector for `node_id`, taking an initial system snapshot.
    pub fn new(node_id: impl Into<String>, role: NodeRole) -> Self {
        let mut sys = System::new_all();
        sys.refresh_all();
        Collector {
            node_id: node_id.into(),
            role,
            sys,
            prev: None,
        }
    }

    /// Build the static [`NodeInfo`] advertised at registration time.
    pub fn node_info(&self, agent_version: &str) -> NodeInfo {
        let hostname = System::host_name().unwrap_or_else(|| self.node_id.clone());
        let total_disk_bytes = Disks::new_with_refreshed_list()
            .iter()
            .map(|d| d.total_space())
            .max()
            .unwrap_or(0);
        NodeInfo {
            node_id: self.node_id.clone(),
            hostname,
            ip_address: local_ip(),
            role: self.role,
            cpu_cores: self.sys.cpus().len() as u32,
            total_memory_bytes: self.sys.total_memory(),
            total_disk_bytes,
            agent_version: agent_version.to_string(),
            kernel_version: System::kernel_version().unwrap_or_default(),
            os: System::long_os_version().unwrap_or_else(|| System::name().unwrap_or_default()),
            started_at_unix: now_unix(),
            labels: Default::default(),
        }
    }

    /// Sample the machine and produce a [`MetricsReport`].
    pub fn collect(&mut self) -> MetricsReport {
        self.sys.refresh_cpu_all();
        self.sys.refresh_memory();

        let cpu = CpuMetrics {
            usage_percent: self.sys.global_cpu_usage() as f64,
            per_core_percent: self
                .sys
                .cpus()
                .iter()
                .map(|c| c.cpu_usage() as f64)
                .collect(),
        };

        let memory = MemoryMetrics {
            total_bytes: self.sys.total_memory(),
            used_bytes: self.sys.used_memory(),
            available_bytes: self.sys.available_memory(),
            swap_total_bytes: self.sys.total_swap(),
            swap_used_bytes: self.sys.used_swap(),
        };

        let la = System::load_average();
        let load = LoadMetrics {
            one: la.one,
            five: la.five,
            fifteen: la.fifteen,
        };

        let now = Instant::now();
        let elapsed = self
            .prev
            .as_ref()
            .map(|p| now.duration_since(p.at).as_secs_f64())
            .filter(|s| *s > 0.0);

        // Capacity from sysinfo, throughput from /proc deltas.
        let cur_disk_io = read_diskstats();
        let disks = self.build_disks(&cur_disk_io, elapsed);

        let cur_net = read_net_dev();
        let network = self.build_network(cur_net, elapsed);

        self.prev = Some(Prev {
            at: now,
            disk: cur_disk_io,
            net: cur_net,
        });

        MetricsReport {
            node_id: self.node_id.clone(),
            timestamp_unix: now_unix(),
            cpu,
            memory,
            load,
            disks,
            network,
            filesystems: Vec::new(),
        }
    }

    fn build_disks(
        &self,
        cur_io: &HashMap<String, (u64, u64)>,
        elapsed: Option<f64>,
    ) -> Vec<DiskMetrics> {
        let disks = Disks::new_with_refreshed_list();
        disks
            .iter()
            .map(|d| {
                let device = d.name().to_string_lossy().to_string();
                let short = device.rsplit('/').next().unwrap_or(&device).to_string();
                let (read_rate, write_rate) = match (elapsed, self.prev.as_ref()) {
                    (Some(secs), Some(prev)) => {
                        let cur = cur_io.get(&short).copied().unwrap_or((0, 0));
                        let old = prev.disk.get(&short).copied().unwrap_or(cur);
                        (rate(cur.0, old.0, secs), rate(cur.1, old.1, secs))
                    }
                    _ => (0, 0),
                };
                DiskMetrics {
                    device: device.clone(),
                    mount_point: d.mount_point().to_string_lossy().to_string(),
                    fs_type: d.file_system().to_string_lossy().to_string(),
                    total_bytes: d.total_space(),
                    available_bytes: d.available_space(),
                    read_bytes_per_sec: read_rate,
                    write_bytes_per_sec: write_rate,
                    io_utilization_percent: 0.0,
                }
            })
            .collect()
    }

    fn build_network(&self, cur: (u64, u64, u64, u64), elapsed: Option<f64>) -> NetworkMetrics {
        match (elapsed, self.prev.as_ref()) {
            (Some(secs), Some(prev)) => NetworkMetrics {
                rx_bytes_per_sec: rate(cur.0, prev.net.0, secs),
                tx_bytes_per_sec: rate(cur.1, prev.net.1, secs),
                rx_errors: cur.2.saturating_sub(prev.net.2),
                tx_errors: cur.3.saturating_sub(prev.net.3),
            },
            _ => NetworkMetrics::default(),
        }
    }
}

/// Per-second rate from two cumulative counter readings, guarding against
/// counter resets (reboot) with `saturating_sub`.
fn rate(cur: u64, old: u64, secs: f64) -> u64 {
    if secs <= 0.0 {
        return 0;
    }
    (cur.saturating_sub(old) as f64 / secs) as u64
}

/// Read cumulative read/write bytes per block device from `/proc/diskstats`.
/// Returns an empty map on non-Linux hosts or on any parse failure.
fn read_diskstats() -> HashMap<String, (u64, u64)> {
    const SECTOR_BYTES: u64 = 512;
    let mut out = HashMap::new();
    let Ok(content) = std::fs::read_to_string("/proc/diskstats") else {
        return out;
    };
    for line in content.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        // fields: major minor name reads _ sectors_read _ writes _ sectors_written ...
        if f.len() < 10 {
            continue;
        }
        let name = f[2];
        // Skip partitions (e.g. sda1) and virtual loop/ram devices.
        if name.starts_with("loop") || name.starts_with("ram") {
            continue;
        }
        let (Ok(rd_sectors), Ok(wr_sectors)) = (f[5].parse::<u64>(), f[9].parse::<u64>()) else {
            continue;
        };
        out.insert(
            name.to_string(),
            (rd_sectors * SECTOR_BYTES, wr_sectors * SECTOR_BYTES),
        );
    }
    out
}

/// Sum cumulative rx/tx bytes and errors across non-loopback interfaces from
/// `/proc/net/dev`. Returns zeros on non-Linux hosts or parse failure.
fn read_net_dev() -> (u64, u64, u64, u64) {
    let Ok(content) = std::fs::read_to_string("/proc/net/dev") else {
        return (0, 0, 0, 0);
    };
    let (mut rx, mut tx, mut rxe, mut txe) = (0u64, 0u64, 0u64, 0u64);
    for line in content.lines() {
        let Some((iface, rest)) = line.split_once(':') else {
            continue;
        };
        let iface = iface.trim();
        if iface == "lo" || iface.is_empty() {
            continue;
        }
        let cols: Vec<&str> = rest.split_whitespace().collect();
        // rx: bytes packets errs ... (cols 0,1,2); tx bytes at col 8, errs at 10
        if cols.len() < 11 {
            continue;
        }
        rx += cols[0].parse::<u64>().unwrap_or(0);
        rxe += cols[2].parse::<u64>().unwrap_or(0);
        tx += cols[8].parse::<u64>().unwrap_or(0);
        txe += cols[10].parse::<u64>().unwrap_or(0);
    }
    (rx, tx, rxe, txe)
}

/// Best-effort local IP discovery: open a UDP socket "towards" a public address
/// and read back the kernel-selected source address. No packets are sent.
fn local_ip() -> String {
    use std::net::UdpSocket;
    let socket = match UdpSocket::bind("0.0.0.0:0") {
        Ok(s) => s,
        Err(_) => return "0.0.0.0".to_string(),
    };
    if socket.connect("8.8.8.8:80").is_err() {
        return "0.0.0.0".to_string();
    }
    socket
        .local_addr()
        .map(|a| a.ip().to_string())
        .unwrap_or_else(|_| "0.0.0.0".to_string())
}
