//! `hpc-daemon` — the cluster management server.
//!
//! Hosts two network surfaces over one shared, async-safe state:
//!
//! * a **gRPC control plane** ([`grpc`]) that agents dial into, and
//! * a **REST/JSON API** ([`api`]) for the CLI, monitor and humans.
//!
//! State is persisted to an embedded redb database ([`store`]) and mirrored in
//! memory behind `Arc<RwLock<…>>` ([`state`]). A background task reaps nodes
//! that stop heartbeating.
#![forbid(unsafe_code)]

mod api;
mod grpc;
mod proto;
mod state;
mod store;

use std::path::PathBuf;
use std::time::Duration;

use anyhow::Context;
use clap::Parser;
use hpc_core::config::DaemonConfig;
use tokio::net::TcpListener;
use tonic::transport::Server;

use crate::state::SharedState;
use crate::store::Store;

/// Command-line arguments for the daemon.
#[derive(Debug, Parser)]
#[command(name = "hpc-daemon", version, about = "HPC cluster management daemon")]
struct Args {
    /// Path to the TOML configuration file. If absent, built-in defaults apply.
    #[arg(short, long, env = "HPC_DAEMON_CONFIG", default_value = "daemon.toml")]
    config: PathBuf,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let cfg: DaemonConfig = hpc_core::config::load_or_default(&args.config)
        .with_context(|| format!("loading config from {}", args.config.display()))?;

    hpc_core::telemetry::init(&cfg.log).context("initialising telemetry")?;
    cfg.validate().context("validating configuration")?;

    tracing::info!(
        version = hpc_core::VERSION,
        grpc = %cfg.grpc_addr,
        http = %cfg.http_addr,
        data_dir = %cfg.data_dir.display(),
        "starting hpc-daemon"
    );

    let store = Store::open(cfg.store_path()).context("opening state store")?;
    let state = SharedState::bootstrap(&cfg, store)
        .await
        .context("bootstrapping cluster state")?;

    // Background reaper: periodically mark silent nodes unreachable.
    let reaper = {
        let state = state.clone();
        let period = (cfg.node_timeout / 2).max(Duration::from_secs(1));
        async move {
            let mut ticker = tokio::time::interval(period);
            loop {
                ticker.tick().await;
                for node_id in state.reap_stale().await {
                    tracing::warn!(%node_id, "node marked unreachable (no heartbeat)");
                }
            }
        }
    };

    // gRPC control plane.
    let grpc = {
        let state = state.clone();
        let addr = cfg.grpc_addr;
        async move {
            Server::builder()
                .add_service(grpc::ClusterRpc::into_service(state))
                .serve(addr)
                .await
                .context("gRPC server failed")
        }
    };

    // REST API.
    let http = {
        let state = state.clone();
        let addr = cfg.http_addr;
        async move {
            let listener = TcpListener::bind(addr)
                .await
                .with_context(|| format!("binding HTTP listener on {addr}"))?;
            axum::serve(listener, api::router(state))
                .await
                .context("HTTP server failed")
        }
    };

    tokio::select! {
        r = grpc => { r?; }
        r = http => { r?; }
        _ = reaper => {}
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
