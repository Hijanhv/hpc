//! `hpc bench …` — run local I/O benchmarks and render saved reports.

use std::path::PathBuf;

use anyhow::{Context, Result};
use comfy_table::Cell;
use hpc_core::types::{BenchReport, IoPattern};

use crate::output::{human_bytes, table};

/// `hpc bench run …` — execute the requested I/O scenarios against `path`.
pub async fn run(
    path: PathBuf,
    block_size: u64,
    file_size: u64,
    patterns: Vec<IoPattern>,
    fsync: bool,
    json: Option<PathBuf>,
) -> Result<()> {
    println!(
        "running {} scenario(s) against {} (file {}, block {})…",
        patterns.len(),
        path.display(),
        human_bytes(file_size),
        human_bytes(block_size),
    );
    let opts = hpc_bench::BenchOptions {
        path,
        block_size,
        file_size,
        patterns,
        fsync,
    };
    let report = hpc_bench::run(opts)
        .await
        .context("running benchmark suite")?;
    print_report(&report);

    if let Some(out) = json {
        let text = serde_json::to_string_pretty(&report).context("serialising report")?;
        std::fs::write(&out, text).with_context(|| format!("writing {}", out.display()))?;
        println!("\nreport written to {}", out.display());
    }
    Ok(())
}

/// `hpc bench report <file>` — pretty-print a previously-saved JSON report.
pub fn report(path: PathBuf) -> Result<()> {
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let report: BenchReport =
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    print_report(&report);
    Ok(())
}

fn print_report(report: &BenchReport) {
    println!(
        "\nbenchmark report for {} (started @ {})",
        report.target_path, report.started_at_unix
    );
    let mut t = table(&[
        "PATTERN",
        "BLOCK",
        "THROUGHPUT",
        "IOPS",
        "p50",
        "p99",
        "p99.9",
        "max",
    ]);
    for r in &report.results {
        t.add_row(vec![
            Cell::new(r.pattern.to_string()),
            Cell::new(human_bytes(r.block_size_bytes)),
            Cell::new(format!("{:.1} MiB/s", r.throughput_mib_s)),
            Cell::new(format!("{:.0}", r.iops)),
            Cell::new(format!("{} µs", r.latency.p50_us)),
            Cell::new(format!("{} µs", r.latency.p99_us)),
            Cell::new(format!("{} µs", r.latency.p999_us)),
            Cell::new(format!("{} µs", r.latency.max_us)),
        ]);
    }
    println!("{t}");
}
