//! Generated gRPC types plus conversions to/from [`hpc_core::types`].
//!
//! The daemon speaks the protobuf wire types at the gRPC boundary but works in
//! terms of the domain types everywhere else. Keeping every conversion in one
//! place means the rest of the daemon never touches a raw `i32` enum field.

use std::collections::BTreeMap;

use hpc_core::types as t;

/// The protobuf-generated module (`package hpc.v1`).
pub mod pb {
    tonic::include_proto!("hpc.v1");
}

// ---------------------------------------------------------------------------
// Enum helpers (protobuf stores enums as i32)
// ---------------------------------------------------------------------------

/// Convert a domain [`NodeRole`](t::NodeRole) into the protobuf discriminant.
pub fn role_to_pb(r: t::NodeRole) -> i32 {
    let v = match r {
        t::NodeRole::Unspecified => pb::NodeRole::Unspecified,
        t::NodeRole::Storage => pb::NodeRole::Storage,
        t::NodeRole::Compute => pb::NodeRole::Compute,
        t::NodeRole::Metadata => pb::NodeRole::Metadata,
        t::NodeRole::Gateway => pb::NodeRole::Gateway,
    };
    v as i32
}

/// Convert a protobuf discriminant back into a domain [`NodeRole`](t::NodeRole),
/// treating unknown values as `Unspecified`.
pub fn role_from_pb(v: i32) -> t::NodeRole {
    match pb::NodeRole::try_from(v).unwrap_or_default() {
        pb::NodeRole::Unspecified => t::NodeRole::Unspecified,
        pb::NodeRole::Storage => t::NodeRole::Storage,
        pb::NodeRole::Compute => t::NodeRole::Compute,
        pb::NodeRole::Metadata => t::NodeRole::Metadata,
        pb::NodeRole::Gateway => t::NodeRole::Gateway,
    }
}

/// Convert a domain [`DeployAction`](t::DeployAction) to its protobuf value.
pub fn deploy_action_to_pb(a: t::DeployAction) -> i32 {
    let v = match a {
        t::DeployAction::Install => pb::DeployAction::Install,
        t::DeployAction::Upgrade => pb::DeployAction::Upgrade,
        t::DeployAction::Rollback => pb::DeployAction::Rollback,
        t::DeployAction::Remove => pb::DeployAction::Remove,
    };
    v as i32
}

/// Convert a protobuf deploy action to the domain type (defaulting to `Install`).
pub fn deploy_action_from_pb(v: i32) -> t::DeployAction {
    match pb::DeployAction::try_from(v).unwrap_or_default() {
        pb::DeployAction::Unspecified | pb::DeployAction::Install => t::DeployAction::Install,
        pb::DeployAction::Upgrade => t::DeployAction::Upgrade,
        pb::DeployAction::Rollback => t::DeployAction::Rollback,
        pb::DeployAction::Remove => t::DeployAction::Remove,
    }
}

/// Convert a domain [`FsAction`](t::FsAction) to its protobuf value.
pub fn fs_action_to_pb(a: t::FsAction) -> i32 {
    let v = match a {
        t::FsAction::Mount => pb::FsAction::Mount,
        t::FsAction::Unmount => pb::FsAction::Unmount,
        t::FsAction::Remount => pb::FsAction::Remount,
        t::FsAction::Check => pb::FsAction::Check,
        t::FsAction::Format => pb::FsAction::Format,
    };
    v as i32
}

/// Convert a protobuf filesystem action to the domain type (defaulting to `Mount`).
pub fn fs_action_from_pb(v: i32) -> t::FsAction {
    match pb::FsAction::try_from(v).unwrap_or_default() {
        pb::FsAction::Unspecified | pb::FsAction::Mount => t::FsAction::Mount,
        pb::FsAction::Unmount => t::FsAction::Unmount,
        pb::FsAction::Remount => t::FsAction::Remount,
        pb::FsAction::Check => t::FsAction::Check,
        pb::FsAction::Format => t::FsAction::Format,
    }
}

fn map_to_btree(m: std::collections::HashMap<String, String>) -> BTreeMap<String, String> {
    m.into_iter().collect()
}

fn btree_to_map(m: BTreeMap<String, String>) -> std::collections::HashMap<String, String> {
    m.into_iter().collect()
}

// ---------------------------------------------------------------------------
// NodeInfo
// ---------------------------------------------------------------------------

impl From<pb::NodeInfo> for t::NodeInfo {
    fn from(p: pb::NodeInfo) -> Self {
        t::NodeInfo {
            node_id: p.node_id,
            hostname: p.hostname,
            ip_address: p.ip_address,
            role: role_from_pb(p.role),
            cpu_cores: p.cpu_cores,
            total_memory_bytes: p.total_memory_bytes,
            total_disk_bytes: p.total_disk_bytes,
            agent_version: p.agent_version,
            kernel_version: p.kernel_version,
            os: p.os,
            started_at_unix: p.started_at_unix,
            labels: map_to_btree(p.labels),
        }
    }
}

impl From<t::NodeInfo> for pb::NodeInfo {
    fn from(n: t::NodeInfo) -> Self {
        pb::NodeInfo {
            role: role_to_pb(n.role),
            node_id: n.node_id,
            hostname: n.hostname,
            ip_address: n.ip_address,
            cpu_cores: n.cpu_cores,
            total_memory_bytes: n.total_memory_bytes,
            total_disk_bytes: n.total_disk_bytes,
            agent_version: n.agent_version,
            kernel_version: n.kernel_version,
            os: n.os,
            started_at_unix: n.started_at_unix,
            labels: btree_to_map(n.labels),
        }
    }
}

// ---------------------------------------------------------------------------
// MetricsReport
// ---------------------------------------------------------------------------

impl From<pb::MetricsReport> for t::MetricsReport {
    fn from(p: pb::MetricsReport) -> Self {
        t::MetricsReport {
            node_id: p.node_id,
            timestamp_unix: p.timestamp_unix,
            cpu: p.cpu.map(Into::into).unwrap_or_default(),
            memory: p.memory.map(Into::into).unwrap_or_default(),
            load: p.load.map(Into::into).unwrap_or_default(),
            disks: p.disks.into_iter().map(Into::into).collect(),
            network: p.network.map(Into::into).unwrap_or_default(),
            filesystems: p.filesystems.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<t::MetricsReport> for pb::MetricsReport {
    fn from(m: t::MetricsReport) -> Self {
        pb::MetricsReport {
            node_id: m.node_id,
            timestamp_unix: m.timestamp_unix,
            cpu: Some(m.cpu.into()),
            memory: Some(m.memory.into()),
            load: Some(m.load.into()),
            disks: m.disks.into_iter().map(Into::into).collect(),
            network: Some(m.network.into()),
            filesystems: m.filesystems.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<pb::CpuMetrics> for t::CpuMetrics {
    fn from(p: pb::CpuMetrics) -> Self {
        t::CpuMetrics {
            usage_percent: p.usage_percent,
            per_core_percent: p.per_core_percent,
        }
    }
}
impl From<t::CpuMetrics> for pb::CpuMetrics {
    fn from(c: t::CpuMetrics) -> Self {
        pb::CpuMetrics {
            usage_percent: c.usage_percent,
            per_core_percent: c.per_core_percent,
        }
    }
}

impl From<pb::MemoryMetrics> for t::MemoryMetrics {
    fn from(p: pb::MemoryMetrics) -> Self {
        t::MemoryMetrics {
            total_bytes: p.total_bytes,
            used_bytes: p.used_bytes,
            available_bytes: p.available_bytes,
            swap_total_bytes: p.swap_total_bytes,
            swap_used_bytes: p.swap_used_bytes,
        }
    }
}
impl From<t::MemoryMetrics> for pb::MemoryMetrics {
    fn from(m: t::MemoryMetrics) -> Self {
        pb::MemoryMetrics {
            total_bytes: m.total_bytes,
            used_bytes: m.used_bytes,
            available_bytes: m.available_bytes,
            swap_total_bytes: m.swap_total_bytes,
            swap_used_bytes: m.swap_used_bytes,
        }
    }
}

impl From<pb::LoadMetrics> for t::LoadMetrics {
    fn from(p: pb::LoadMetrics) -> Self {
        t::LoadMetrics {
            one: p.one,
            five: p.five,
            fifteen: p.fifteen,
        }
    }
}
impl From<t::LoadMetrics> for pb::LoadMetrics {
    fn from(l: t::LoadMetrics) -> Self {
        pb::LoadMetrics {
            one: l.one,
            five: l.five,
            fifteen: l.fifteen,
        }
    }
}

impl From<pb::DiskMetrics> for t::DiskMetrics {
    fn from(p: pb::DiskMetrics) -> Self {
        t::DiskMetrics {
            device: p.device,
            mount_point: p.mount_point,
            fs_type: p.fs_type,
            total_bytes: p.total_bytes,
            available_bytes: p.available_bytes,
            read_bytes_per_sec: p.read_bytes_per_sec,
            write_bytes_per_sec: p.write_bytes_per_sec,
            io_utilization_percent: p.io_utilization_percent,
        }
    }
}
impl From<t::DiskMetrics> for pb::DiskMetrics {
    fn from(d: t::DiskMetrics) -> Self {
        pb::DiskMetrics {
            device: d.device,
            mount_point: d.mount_point,
            fs_type: d.fs_type,
            total_bytes: d.total_bytes,
            available_bytes: d.available_bytes,
            read_bytes_per_sec: d.read_bytes_per_sec,
            write_bytes_per_sec: d.write_bytes_per_sec,
            io_utilization_percent: d.io_utilization_percent,
        }
    }
}

impl From<pb::NetworkMetrics> for t::NetworkMetrics {
    fn from(p: pb::NetworkMetrics) -> Self {
        t::NetworkMetrics {
            rx_bytes_per_sec: p.rx_bytes_per_sec,
            tx_bytes_per_sec: p.tx_bytes_per_sec,
            rx_errors: p.rx_errors,
            tx_errors: p.tx_errors,
        }
    }
}
impl From<t::NetworkMetrics> for pb::NetworkMetrics {
    fn from(n: t::NetworkMetrics) -> Self {
        pb::NetworkMetrics {
            rx_bytes_per_sec: n.rx_bytes_per_sec,
            tx_bytes_per_sec: n.tx_bytes_per_sec,
            rx_errors: n.rx_errors,
            tx_errors: n.tx_errors,
        }
    }
}

impl From<pb::FilesystemMetrics> for t::FilesystemMetrics {
    fn from(p: pb::FilesystemMetrics) -> Self {
        t::FilesystemMetrics {
            name: p.name,
            mount_point: p.mount_point,
            mounted: p.mounted,
            total_bytes: p.total_bytes,
            used_bytes: p.used_bytes,
            inodes_total: p.inodes_total,
            inodes_used: p.inodes_used,
        }
    }
}
impl From<t::FilesystemMetrics> for pb::FilesystemMetrics {
    fn from(f: t::FilesystemMetrics) -> Self {
        pb::FilesystemMetrics {
            name: f.name,
            mount_point: f.mount_point,
            mounted: f.mounted,
            total_bytes: f.total_bytes,
            used_bytes: f.used_bytes,
            inodes_total: f.inodes_total,
            inodes_used: f.inodes_used,
        }
    }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

impl From<t::DeploySpec> for pb::DeployCommand {
    fn from(d: t::DeploySpec) -> Self {
        pb::DeployCommand {
            deployment_id: d.deployment_id,
            action: deploy_action_to_pb(d.action),
            component: d.component,
            version: d.version,
            target_path: d.target_path,
            options: btree_to_map(d.options),
        }
    }
}
impl From<pb::DeployCommand> for t::DeploySpec {
    fn from(p: pb::DeployCommand) -> Self {
        t::DeploySpec {
            deployment_id: p.deployment_id,
            action: deploy_action_from_pb(p.action),
            component: p.component,
            version: p.version,
            target_path: p.target_path,
            options: map_to_btree(p.options),
        }
    }
}

impl From<t::FsSpec> for pb::FsCommand {
    fn from(f: t::FsSpec) -> Self {
        pb::FsCommand {
            command_id: f.command_id,
            action: fs_action_to_pb(f.action),
            device: f.device,
            mount_point: f.mount_point,
            fs_type: f.fs_type,
            mount_options: f.mount_options,
            force: f.force,
        }
    }
}
impl From<pb::FsCommand> for t::FsSpec {
    fn from(p: pb::FsCommand) -> Self {
        t::FsSpec {
            command_id: p.command_id,
            action: fs_action_from_pb(p.action),
            device: p.device,
            mount_point: p.mount_point,
            fs_type: p.fs_type,
            mount_options: p.mount_options,
            force: p.force,
        }
    }
}

impl From<pb::CommandResult> for t::CommandOutcome {
    fn from(p: pb::CommandResult) -> Self {
        t::CommandOutcome {
            command_id: p.command_id,
            node_id: p.node_id,
            success: p.success,
            exit_code: p.exit_code,
            message: p.message,
            stdout: p.stdout,
            stderr: p.stderr,
            completed_at_unix: p.completed_at_unix,
        }
    }
}
impl From<t::CommandOutcome> for pb::CommandResult {
    fn from(o: t::CommandOutcome) -> Self {
        pb::CommandResult {
            command_id: o.command_id,
            node_id: o.node_id,
            success: o.success,
            exit_code: o.exit_code,
            message: o.message,
            stdout: o.stdout,
            stderr: o.stderr,
            completed_at_unix: o.completed_at_unix,
        }
    }
}
