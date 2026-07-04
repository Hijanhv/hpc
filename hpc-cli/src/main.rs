//! `hpc` — the operator command-line interface.
//!
//! Talks to a running `hpc-daemon` over its REST API for `node` and `fs`
//! subcommands, and drives the `hpc-bench` suite directly for `bench`.
#![forbid(unsafe_code)]

mod client;
mod commands;
mod output;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use hpc_core::config::LogConfig;
use hpc_core::types::{DeployAction, IoPattern};

use crate::client::ApiClient;

/// Default benchmark file size: 64 MiB.
const DEFAULT_FILE_SIZE: u64 = 64 * 1024 * 1024;

#[derive(Debug, Parser)]
#[command(name = "hpc", version, about = "Operator CLI for the HPC framework")]
struct Cli {
    /// Base URL of the daemon REST API.
    #[arg(
        long,
        env = "HPC_API",
        default_value = "http://127.0.0.1:8080",
        global = true
    )]
    api: String,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Inspect and manage cluster nodes.
    Node {
        #[command(subcommand)]
        cmd: NodeCmd,
    },
    /// Filesystem / mount operations.
    Fs {
        #[command(subcommand)]
        cmd: FsCmd,
    },
    /// Run and inspect I/O benchmarks.
    Bench {
        #[command(subcommand)]
        cmd: BenchCmd,
    },
}

#[derive(Debug, Subcommand)]
enum NodeCmd {
    /// List all registered nodes.
    List,
    /// Show detailed status for one node.
    Status {
        /// Node id.
        id: String,
    },
    /// Issue a deploy command to a node.
    Deploy {
        /// Node id.
        id: String,
        /// Component to deploy, e.g. `lustre-ost`.
        #[arg(long)]
        component: String,
        /// Target version.
        #[arg(long, default_value = "")]
        version: String,
        /// Deploy action.
        #[arg(long, value_enum, default_value_t = DeployActionArg::Install)]
        action: DeployActionArg,
        /// Installation prefix.
        #[arg(long, default_value = "")]
        target_path: String,
        /// Extra `key=value` options (repeatable).
        #[arg(long = "opt", value_parser = parse_kv)]
        opt: Vec<(String, String)>,
    },
}

#[derive(Debug, Subcommand)]
enum FsCmd {
    /// Mount a filesystem on a node.
    Mount {
        /// Node id.
        id: String,
        /// Block device or network source.
        #[arg(long)]
        device: String,
        /// Mount point.
        #[arg(long)]
        mount_point: String,
        /// Filesystem type (ext4, xfs, lustre, nfs…).
        #[arg(long, default_value = "")]
        fs_type: String,
        /// Mount options (repeatable), e.g. `--opt noatime`.
        #[arg(long = "opt")]
        opt: Vec<String>,
    },
    /// Unmount a filesystem on a node.
    Unmount {
        /// Node id.
        id: String,
        /// Mount point to unmount.
        #[arg(long)]
        mount_point: String,
    },
    /// Show the filesystems a node currently reports.
    Status {
        /// Node id.
        id: String,
    },
}

#[derive(Debug, Subcommand)]
enum BenchCmd {
    /// Run I/O benchmarks against a path.
    Run {
        /// Directory to benchmark in.
        #[arg(long, default_value = ".")]
        path: PathBuf,
        /// Block size in bytes.
        #[arg(long, default_value_t = 4096)]
        block_size: u64,
        /// Test file size in bytes.
        #[arg(long, default_value_t = DEFAULT_FILE_SIZE)]
        file_size: u64,
        /// Patterns to run (comma-separated). Defaults to all four.
        #[arg(long, value_enum, value_delimiter = ',',
              default_values_t = vec![PatternArg::SeqWrite, PatternArg::SeqRead, PatternArg::RandWrite, PatternArg::RandRead])]
        pattern: Vec<PatternArg>,
        /// fsync after each write scenario.
        #[arg(long)]
        fsync: bool,
        /// Also write the report as JSON to this path.
        #[arg(long)]
        json: Option<PathBuf>,
    },
    /// Pretty-print a previously-saved JSON report.
    Report {
        /// Path to a JSON report produced by `bench run --json`.
        path: PathBuf,
    },
}

/// clap mirror of [`DeployAction`].
#[derive(Debug, Clone, Copy, ValueEnum)]
enum DeployActionArg {
    Install,
    Upgrade,
    Rollback,
    Remove,
}

impl From<DeployActionArg> for DeployAction {
    fn from(a: DeployActionArg) -> Self {
        match a {
            DeployActionArg::Install => DeployAction::Install,
            DeployActionArg::Upgrade => DeployAction::Upgrade,
            DeployActionArg::Rollback => DeployAction::Rollback,
            DeployActionArg::Remove => DeployAction::Remove,
        }
    }
}

/// clap mirror of [`IoPattern`].
#[derive(Debug, Clone, Copy, ValueEnum)]
enum PatternArg {
    SeqRead,
    SeqWrite,
    RandRead,
    RandWrite,
}

impl From<PatternArg> for IoPattern {
    fn from(p: PatternArg) -> Self {
        match p {
            PatternArg::SeqRead => IoPattern::SequentialRead,
            PatternArg::SeqWrite => IoPattern::SequentialWrite,
            PatternArg::RandRead => IoPattern::RandomRead,
            PatternArg::RandWrite => IoPattern::RandomWrite,
        }
    }
}

fn parse_kv(s: &str) -> std::result::Result<(String, String), String> {
    match s.split_once('=') {
        Some((k, v)) if !k.is_empty() => Ok((k.to_string(), v.to_string())),
        _ => Err(format!("expected key=value, got `{s}`")),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Quiet by default so command output is clean; RUST_LOG can raise it.
    let log = LogConfig {
        filter: "warn".to_string(),
        json: false,
        with_location: false,
    };
    let _ = hpc_core::telemetry::init(&log);

    let cli = Cli::parse();
    match cli.command {
        Command::Node { cmd } => run_node(&cli.api, cmd).await,
        Command::Fs { cmd } => run_fs(&cli.api, cmd).await,
        Command::Bench { cmd } => run_bench(cmd).await,
    }
}

async fn run_node(api_url: &str, cmd: NodeCmd) -> Result<()> {
    let api = ApiClient::new(api_url)?;
    match cmd {
        NodeCmd::List => commands::node::list(&api).await,
        NodeCmd::Status { id } => commands::node::status(&api, &id).await,
        NodeCmd::Deploy {
            id,
            component,
            version,
            action,
            target_path,
            opt,
        } => {
            commands::node::deploy(
                &api,
                &id,
                &component,
                &version,
                action.into(),
                &target_path,
                opt,
            )
            .await
        }
    }
}

async fn run_fs(api_url: &str, cmd: FsCmd) -> Result<()> {
    let api = ApiClient::new(api_url)?;
    match cmd {
        FsCmd::Mount {
            id,
            device,
            mount_point,
            fs_type,
            opt,
        } => commands::fs::mount(&api, &id, &device, &mount_point, &fs_type, opt).await,
        FsCmd::Unmount { id, mount_point } => commands::fs::unmount(&api, &id, &mount_point).await,
        FsCmd::Status { id } => commands::fs::status(&api, &id).await,
    }
}

async fn run_bench(cmd: BenchCmd) -> Result<()> {
    match cmd {
        BenchCmd::Run {
            path,
            block_size,
            file_size,
            pattern,
            fsync,
            json,
        } => {
            let patterns = pattern.into_iter().map(IoPattern::from).collect();
            commands::bench::run(path, block_size, file_size, patterns, fsync, json).await
        }
        BenchCmd::Report { path } => commands::bench::report(path),
    }
}
