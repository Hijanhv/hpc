//! `hpc fs …` — filesystem/mount operations and status.

use anyhow::Result;
use comfy_table::Cell;

use crate::client::ApiClient;
use crate::output::{human_bytes, table};

/// `hpc fs mount <id> …` — request a mount on a node.
pub async fn mount(
    api: &ApiClient,
    id: &str,
    device: &str,
    mount_point: &str,
    fs_type: &str,
    options: Vec<String>,
) -> Result<()> {
    let body = serde_json::json!({
        "action": "mount",
        "device": device,
        "mount_point": mount_point,
        "fs_type": fs_type,
        "mount_options": options,
    });
    let ack = api.fs_command(id, &body).await?;
    println!(
        "mount accepted: command={} node={} ({} -> {})",
        ack.command_id, ack.node_id, device, mount_point
    );
    Ok(())
}

/// `hpc fs unmount <id> …` — request an unmount on a node.
pub async fn unmount(api: &ApiClient, id: &str, mount_point: &str) -> Result<()> {
    let body = serde_json::json!({
        "action": "unmount",
        "mount_point": mount_point,
    });
    let ack = api.fs_command(id, &body).await?;
    println!(
        "unmount accepted: command={} node={} ({})",
        ack.command_id, ack.node_id, mount_point
    );
    Ok(())
}

/// `hpc fs status <id>` — show the filesystems/disks a node currently reports.
pub async fn status(api: &ApiClient, id: &str) -> Result<()> {
    let metrics = api.node_metrics(id).await?;
    if metrics.disks.is_empty() && metrics.filesystems.is_empty() {
        println!("node {id} reports no filesystems");
        return Ok(());
    }

    let mut t = table(&["MOUNT", "FS", "DEVICE", "USED", "AVAIL", "TOTAL", "USE%"]);
    for d in &metrics.disks {
        let used = d.total_bytes.saturating_sub(d.available_bytes);
        t.add_row(vec![
            Cell::new(&d.mount_point),
            Cell::new(&d.fs_type),
            Cell::new(&d.device),
            Cell::new(human_bytes(used)),
            Cell::new(human_bytes(d.available_bytes)),
            Cell::new(human_bytes(d.total_bytes)),
            Cell::new(format!("{:.0}%", d.used_fraction() * 100.0)),
        ]);
    }
    for f in &metrics.filesystems {
        t.add_row(vec![
            Cell::new(&f.mount_point),
            Cell::new(&f.name),
            Cell::new(if f.mounted { "mounted" } else { "unmounted" }),
            Cell::new(human_bytes(f.used_bytes)),
            Cell::new(human_bytes(f.total_bytes.saturating_sub(f.used_bytes))),
            Cell::new(human_bytes(f.total_bytes)),
            Cell::new("-"),
        ]);
    }
    println!("{t}");
    Ok(())
}
