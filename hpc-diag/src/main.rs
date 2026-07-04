//! `hpc-diag` — collect a multi-platform diagnostic bundle as JSON.
//!
//! ```text
//! hpc-diag collect --output diag.json     # write a pretty JSON bundle
//! hpc-diag collect --output - --compact   # stream compact JSON to stdout
//! ```
//!
//! The bundle captures the live `/proc` snapshot plus any anomalies the
//! heuristics flag; see [`hpc_diag`] for the data model.
#![forbid(unsafe_code)]

use anyhow::Context;
use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "hpc-diag",
    version,
    about = "HPC multi-platform diagnostics & bug-analysis bundler"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Collect a full diagnostic bundle and write it as JSON.
    Collect {
        /// Output path, or `-` to write to stdout.
        #[arg(short, long, default_value = "diag.json")]
        output: String,
        /// Emit compact single-line JSON instead of pretty-printed.
        #[arg(long)]
        compact: bool,
    },
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Command::Collect { output, compact } => run_collect(&output, compact),
    }
}

fn run_collect(output: &str, compact: bool) -> anyhow::Result<()> {
    let bundle = hpc_diag::collect();
    let json = if compact {
        bundle.to_json().context("serializing bundle")?
    } else {
        bundle.to_json_pretty().context("serializing bundle")?
    };

    if output == "-" {
        // Bundle goes to stdout; keep stdout clean for piping into `jq`.
        println!("{json}");
    } else {
        std::fs::write(output, &json).with_context(|| format!("writing bundle to {output}"))?;
        // Status/summary goes to stderr so it never contaminates the artifact.
        eprintln!(
            "hpc-diag: wrote {output} — {} report(s), {} anomaly(ies)",
            bundle.reports.len(),
            bundle.anomalies.len()
        );
    }
    Ok(())
}
