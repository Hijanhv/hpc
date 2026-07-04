//! Parsers for the Linux `/proc` pseudo-filesystem.
//!
//! Each `parse_*` function is a pure function over the file's text, so it can
//! be unit-tested with fixtures on any platform (including the macOS/CI hosts
//! that have no `/proc`). The `collect_*` functions read the real file and then
//! parse it; on a non-Linux host they return an [`HpcError::Io`] which the
//! bundler records as a "collector unavailable" note rather than failing.

use hpc_core::error::{HpcError, Result};
use serde::{Deserialize, Serialize};

/// Selected fields from `/proc/meminfo`, all in kibibytes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemInfo {
    pub mem_total_kb: u64,
    pub mem_free_kb: u64,
    pub mem_available_kb: u64,
    pub buffers_kb: u64,
    pub cached_kb: u64,
    pub swap_total_kb: u64,
    pub swap_free_kb: u64,
}

impl MemInfo {
    /// Fraction of memory reported available, in `0.0..=1.0`. Uses
    /// `MemAvailable` (the kernel's estimate of reclaimable + free memory).
    pub fn available_fraction(&self) -> f64 {
        if self.mem_total_kb == 0 {
            0.0
        } else {
            self.mem_available_kb as f64 / self.mem_total_kb as f64
        }
    }
}

/// Per-device counters from `/proc/diskstats`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiskStat {
    pub device: String,
    pub reads_completed: u64,
    pub sectors_read: u64,
    pub writes_completed: u64,
    pub sectors_written: u64,
}

/// Per-interface counters from `/proc/net/dev`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetDev {
    pub interface: String,
    pub rx_bytes: u64,
    pub rx_errs: u64,
    pub tx_bytes: u64,
    pub tx_errs: u64,
}

/// A single row of `/proc/mounts`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MountEntry {
    pub device: String,
    pub mount_point: String,
    pub fs_type: String,
    pub options: Vec<String>,
}

/// Kernel identity, from `/proc/version`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KernelInfo {
    /// The full, unparsed banner line.
    pub raw: String,
    /// The release token (e.g. `6.5.0-15-generic`) when it could be isolated.
    pub release: String,
}

impl KernelInfo {
    /// Best-effort `(major, minor)` parsed from [`Self::release`].
    pub fn version_pair(&self) -> Option<(u32, u32)> {
        let mut parts = self.release.split(['.', '-']);
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next()?.parse().ok()?;
        Some((major, minor))
    }
}

/// Parse `/proc/meminfo`. Unknown/missing keys default to zero.
pub fn parse_meminfo(text: &str) -> MemInfo {
    let mut m = MemInfo::default();
    for line in text.lines() {
        let Some((key, rest)) = line.split_once(':') else {
            continue;
        };
        // Value looks like "  16333764 kB"; take the first numeric token.
        let value = rest
            .split_whitespace()
            .next()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0);
        match key.trim() {
            "MemTotal" => m.mem_total_kb = value,
            "MemFree" => m.mem_free_kb = value,
            "MemAvailable" => m.mem_available_kb = value,
            "Buffers" => m.buffers_kb = value,
            "Cached" => m.cached_kb = value,
            "SwapTotal" => m.swap_total_kb = value,
            "SwapFree" => m.swap_free_kb = value,
            _ => {}
        }
    }
    m
}

/// Parse `/proc/diskstats`. Malformed lines are skipped.
pub fn parse_diskstats(text: &str) -> Vec<DiskStat> {
    let mut out = Vec::new();
    for line in text.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        // Layout: major minor name reads rd_merged sectors_read time_reading
        //         writes wr_merged sectors_written ...
        if f.len() < 10 {
            continue;
        }
        out.push(DiskStat {
            device: f[2].to_string(),
            reads_completed: f[3].parse().unwrap_or(0),
            sectors_read: f[5].parse().unwrap_or(0),
            writes_completed: f[7].parse().unwrap_or(0),
            sectors_written: f[9].parse().unwrap_or(0),
        });
    }
    out
}

/// Parse `/proc/net/dev`. The two header lines and malformed rows are skipped.
pub fn parse_net_dev(text: &str) -> Vec<NetDev> {
    let mut out = Vec::new();
    for line in text.lines() {
        let Some((iface, rest)) = line.split_once(':') else {
            continue; // header lines have no ':' in the interface column
        };
        let f: Vec<&str> = rest.split_whitespace().collect();
        // Receive: bytes packets errs drop fifo frame compressed multicast (8)
        // Transmit: bytes packets errs drop fifo colls carrier compressed
        if f.len() < 11 {
            continue;
        }
        out.push(NetDev {
            interface: iface.trim().to_string(),
            rx_bytes: f[0].parse().unwrap_or(0),
            rx_errs: f[2].parse().unwrap_or(0),
            tx_bytes: f[8].parse().unwrap_or(0),
            tx_errs: f[10].parse().unwrap_or(0),
        });
    }
    out
}

/// Parse `/proc/mounts` (same columns as `/etc/fstab`, space-separated).
pub fn parse_mounts(text: &str) -> Vec<MountEntry> {
    let mut out = Vec::new();
    for line in text.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 4 {
            continue;
        }
        out.push(MountEntry {
            device: f[0].to_string(),
            mount_point: f[1].to_string(),
            fs_type: f[2].to_string(),
            options: f[3].split(',').map(|s| s.to_string()).collect(),
        });
    }
    out
}

/// Parse the `/proc/version` banner into a [`KernelInfo`].
pub fn parse_version(text: &str) -> KernelInfo {
    let raw = text.trim().to_string();
    // "Linux version 6.5.0-15-generic (buildd@...) ..." -> 3rd whitespace token.
    let release = raw
        .split_whitespace()
        .nth(2)
        .unwrap_or_default()
        .to_string();
    KernelInfo { raw, release }
}

fn read_proc(path: &str) -> Result<String> {
    std::fs::read_to_string(path).map_err(|e| HpcError::io_at(path, e))
}

/// Read + parse `/proc/meminfo`.
pub fn collect_meminfo() -> Result<MemInfo> {
    Ok(parse_meminfo(&read_proc("/proc/meminfo")?))
}

/// Read + parse `/proc/diskstats`.
pub fn collect_diskstats() -> Result<Vec<DiskStat>> {
    Ok(parse_diskstats(&read_proc("/proc/diskstats")?))
}

/// Read + parse `/proc/net/dev`.
pub fn collect_net_dev() -> Result<Vec<NetDev>> {
    Ok(parse_net_dev(&read_proc("/proc/net/dev")?))
}

/// Read + parse `/proc/mounts`.
pub fn collect_mounts() -> Result<Vec<MountEntry>> {
    Ok(parse_mounts(&read_proc("/proc/mounts")?))
}

/// Read + parse `/proc/version`.
pub fn collect_kernel() -> Result<KernelInfo> {
    Ok(parse_version(&read_proc("/proc/version")?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meminfo_parses_known_keys() {
        let text = "\
MemTotal:       16333764 kB
MemFree:         1200000 kB
MemAvailable:    8000000 kB
Buffers:          250000 kB
Cached:          3000000 kB
SwapTotal:       2097148 kB
SwapFree:        2097148 kB
Hugepagesize:       2048 kB";
        let m = parse_meminfo(text);
        assert_eq!(m.mem_total_kb, 16_333_764);
        assert_eq!(m.mem_available_kb, 8_000_000);
        assert_eq!(m.swap_total_kb, 2_097_148);
        assert!((m.available_fraction() - 0.4898).abs() < 0.01);
    }

    #[test]
    fn diskstats_extracts_device_counters() {
        let text = "\
   8       0 sda 100 0 2000 30 200 0 4000 60 0 90 90
 259       0 nvme0n1 5 0 40 1 6 0 80 2 0 3 3";
        let stats = parse_diskstats(text);
        assert_eq!(stats.len(), 2);
        assert_eq!(stats[0].device, "sda");
        assert_eq!(stats[0].reads_completed, 100);
        assert_eq!(stats[0].sectors_read, 2000);
        assert_eq!(stats[1].device, "nvme0n1");
        assert_eq!(stats[1].sectors_written, 80);
    }

    #[test]
    fn net_dev_skips_headers_and_parses_rows() {
        let text = "\
Inter-|   Receive                                                |  Transmit
 face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed
    lo: 1000000     100    0    0    0     0          0         0  1000000     100    0    0    0     0       0          0
  eth0: 500000     400    2    0    0     0          0         0   750000     500    7    0    0     0       0          0";
        let devs = parse_net_dev(text);
        assert_eq!(devs.len(), 2);
        assert_eq!(devs[1].interface, "eth0");
        assert_eq!(devs[1].rx_bytes, 500_000);
        assert_eq!(devs[1].rx_errs, 2);
        assert_eq!(devs[1].tx_bytes, 750_000);
        assert_eq!(devs[1].tx_errs, 7);
    }

    #[test]
    fn mounts_split_options() {
        let text = "\
/dev/sda1 / ext4 rw,relatime 0 0
tmpfs /run tmpfs rw,nosuid,nodev,mode=755 0 0";
        let mounts = parse_mounts(text);
        assert_eq!(mounts.len(), 2);
        assert_eq!(mounts[0].fs_type, "ext4");
        assert!(mounts[0].options.contains(&"relatime".to_string()));
        assert_eq!(mounts[1].fs_type, "tmpfs");
        assert!(mounts[1].options.contains(&"nosuid".to_string()));
    }

    #[test]
    fn version_isolates_release_and_pair() {
        let k =
            parse_version("Linux version 6.5.0-15-generic (buildd@lcy02) (gcc 13) #15-Ubuntu SMP");
        assert_eq!(k.release, "6.5.0-15-generic");
        assert_eq!(k.version_pair(), Some((6, 5)));
    }
}
