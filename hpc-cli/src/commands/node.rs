//! `hpc node …` — inspect and act on cluster nodes.

use anyhow::Result;
use comfy_table::Cell;
use hpc_core::types::{now_unix, DeployAction};

use crate::client::ApiClient;
use crate::output::{human_bytes, human_rate, status_cell, table};

/// `hpc node list` — one row per node with summarised health.
pub async fn list(api: &ApiClient) -> Result<()> {
    let nodes = api.list_nodes().await?;
    let status = api.cluster_status().await?;

    if nodes.is_empty() {
        println!("no nodes registered");
        return Ok(());
    }

    let mut t = table(&["NODE", "ROLE", "STATUS", "CPU", "MEM", "DISK", "LAST SEEN"]);
    for n in &nodes {
        let (cpu, mem, disk) = match &n.latest_metrics {
            Some(m) => {
                let cpu = format!("{:.0}%", m.cpu.usage_percent);
                let mem = format!("{:.0}%", m.memory.used_fraction() * 100.0);
                let disk = m
                    .disks
                    .iter()
                    .map(|d| d.used_fraction())
                    .fold(0.0_f64, f64::max);
                (cpu, mem, format!("{:.0}%", disk * 100.0))
            }
            None => ("-".into(), "-".into(), "-".into()),
        };
        t.add_row(vec![
            Cell::new(&n.info.node_id),
            Cell::new(format!("{:?}", n.info.role).to_lowercase()),
            status_cell(n.status),
            Cell::new(cpu),
            Cell::new(mem),
            Cell::new(disk),
            Cell::new(format!(
                "{}s ago",
                now_unix().saturating_sub(n.last_seen_unix)
            )),
        ]);
    }
    println!("{t}");
    println!(
        "\n{} node(s): {} healthy, {} degraded, {} unreachable, {} connected",
        status.total_nodes,
        status.healthy,
        status.degraded,
        status.unreachable,
        status.connected_streams
    );
    Ok(())
}

/// `hpc node status <id>` — detailed view of a single node.
pub async fn status(api: &ApiClient, id: &str) -> Result<()> {
    let node = api.get_node(id).await?;
    let info = &node.info;

    let mut t = table(&["FIELD", "VALUE"]);
    t.add_row(vec!["node id", &info.node_id]);
    t.add_row(vec!["hostname", &info.hostname]);
    t.add_row(vec!["address", &info.ip_address]);
    t.add_row(vec!["role", &format!("{:?}", info.role).to_lowercase()]);
    t.add_row(vec!["status", &format!("{:?}", node.status).to_lowercase()]);
    t.add_row(vec!["os", &info.os]);
    t.add_row(vec!["kernel", &info.kernel_version]);
    t.add_row(vec!["agent", &info.agent_version]);
    t.add_row(vec!["cpu cores", &info.cpu_cores.to_string()]);
    t.add_row(vec!["memory", &human_bytes(info.total_memory_bytes)]);
    println!("{t}");

    match node.latest_metrics {
        Some(m) => {
            println!("\nlatest sample @ {}:", m.timestamp_unix);
            println!(
                "  cpu {:.1}%   mem {:.1}%   load {:.2}/{:.2}/{:.2}",
                m.cpu.usage_percent,
                m.memory.used_fraction() * 100.0,
                m.load.one,
                m.load.five,
                m.load.fifteen
            );
            if !m.disks.is_empty() {
                let mut dt = table(&["DEVICE", "MOUNT", "FS", "USED", "TOTAL", "READ", "WRITE"]);
                for d in &m.disks {
                    dt.add_row(vec![
                        Cell::new(&d.device),
                        Cell::new(&d.mount_point),
                        Cell::new(&d.fs_type),
                        Cell::new(format!("{:.0}%", d.used_fraction() * 100.0)),
                        Cell::new(human_bytes(d.total_bytes)),
                        Cell::new(human_rate(d.read_bytes_per_sec)),
                        Cell::new(human_rate(d.write_bytes_per_sec)),
                    ]);
                }
                println!("{dt}");
            }
        }
        None => println!("\nno metrics reported yet"),
    }
    Ok(())
}

/// `hpc node deploy <id> …` — issue a deploy command to a node.
#[allow(clippy::too_many_arguments)]
pub async fn deploy(
    api: &ApiClient,
    id: &str,
    component: &str,
    version: &str,
    action: DeployAction,
    target_path: &str,
    options: Vec<(String, String)>,
) -> Result<()> {
    let opts: serde_json::Map<String, serde_json::Value> = options
        .into_iter()
        .map(|(k, v)| (k, serde_json::Value::String(v)))
        .collect();
    let body = serde_json::json!({
        "action": action,
        "component": component,
        "version": version,
        "target_path": target_path,
        "options": opts,
    });
    let ack = api.deploy(id, &body).await?;
    println!(
        "deploy accepted: command={} node={} (outcome is asynchronous; see `hpc node status`)",
        ack.command_id, ack.node_id
    );
    Ok(())
}
