//! Presentation helpers: consistent tables and human-friendly units.

use comfy_table::{presets::UTF8_FULL, Cell, Color, Table};
use hpc_core::types::NodeStatus;

/// Build a pre-styled table with the given header row.
pub fn table(headers: &[&str]) -> Table {
    let mut t = Table::new();
    t.load_preset(UTF8_FULL);
    t.set_header(headers.iter().map(|h| Cell::new(h).fg(Color::Cyan)));
    t
}

/// Format a byte count in binary units (KiB/MiB/…), 2 significant decimals.
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.2} {}", UNITS[unit])
    }
}

/// Format a per-second byte rate.
pub fn human_rate(bytes_per_sec: u64) -> String {
    format!("{}/s", human_bytes(bytes_per_sec))
}

/// A coloured cell for a node status.
pub fn status_cell(status: NodeStatus) -> Cell {
    let (text, color) = match status {
        NodeStatus::Healthy | NodeStatus::Registered => ("healthy", Color::Green),
        NodeStatus::Degraded => ("degraded", Color::Yellow),
        NodeStatus::Unreachable => ("unreachable", Color::Red),
        NodeStatus::Draining => ("draining", Color::Blue),
    };
    Cell::new(text).fg(color)
}
