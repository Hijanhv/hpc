//! `hpc-agent` — the per-node agent.
//!
//! A small, resilient gRPC client that registers with the daemon, streams
//! resource metrics collected from [`sysinfo`] and `/proc`, and executes the
//! deploy/filesystem commands the daemon pushes down its command stream. See
//! [`client::run`] for the session lifecycle.
#![forbid(unsafe_code)]

mod client;
mod executor;
mod metrics;
mod proto;

use std::path::PathBuf;

use anyhow::Context;
use clap::Parser;
use hpc_core::config::AgentConfig;

/// Command-line arguments for the agent.
#[derive(Debug, Parser)]
#[command(name = "hpc-agent", version, about = "HPC per-node agent")]
struct Args {
    /// Path to the TOML configuration file. If absent, built-in defaults apply.
    #[arg(short, long, env = "HPC_AGENT_CONFIG", default_value = "agent.toml")]
    config: PathBuf,

    /// Override the daemon gRPC endpoint (e.g. http://10.0.0.1:7443).
    #[arg(long, env = "HPC_DAEMON_ENDPOINT")]
    endpoint: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let mut cfg: AgentConfig = hpc_core::config::load_or_default(&args.config)
        .with_context(|| format!("loading config from {}", args.config.display()))?;
    if let Some(endpoint) = args.endpoint {
        cfg.daemon_endpoint = endpoint;
    }

    hpc_core::telemetry::init(&cfg.log).context("initialising telemetry")?;
    tracing::info!(version = hpc_core::VERSION, "starting hpc-agent");

    tokio::select! {
        result = client::run(cfg) => {
            result.context("agent session loop exited")?;
        }
        _ = shutdown_signal() => {
            tracing::info!("shutdown signal received; exiting");
        }
    }
    Ok(())
}

/// Resolve when the process is asked to terminate (Ctrl-C / SIGTERM).
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sig) => {
                sig.recv().await;
            }
            Err(e) => tracing::warn!(error = %e, "failed to install SIGTERM handler"),
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
}
