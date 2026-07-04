//! `hpc-monitor` — Prometheus exporter and degradation detector.
//!
//! Scrapes the daemon's REST API on an interval, translates cluster state into
//! Prometheus metrics ([`metrics`]), applies threshold policy to flag degraded
//! nodes ([`health`]), and exposes everything at `/metrics` for Prometheus to
//! scrape in turn.
#![forbid(unsafe_code)]

mod health;
mod metrics;

use std::path::PathBuf;

use anyhow::Context;
use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use clap::Parser;
use hpc_core::config::MonitorConfig;
use tokio::net::TcpListener;

use crate::metrics::Metrics;

/// Command-line arguments for the monitor.
#[derive(Debug, Parser)]
#[command(
    name = "hpc-monitor",
    version,
    about = "HPC Prometheus exporter & health monitor"
)]
struct Args {
    /// Path to the TOML configuration file. If absent, built-in defaults apply.
    #[arg(
        short,
        long,
        env = "HPC_MONITOR_CONFIG",
        default_value = "monitor.toml"
    )]
    config: PathBuf,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let cfg: MonitorConfig = hpc_core::config::load_or_default(&args.config)
        .with_context(|| format!("loading config from {}", args.config.display()))?;

    hpc_core::telemetry::init(&cfg.log).context("initialising telemetry")?;
    cfg.thresholds.validate().context("validating thresholds")?;
    tracing::info!(
        version = hpc_core::VERSION,
        metrics_addr = %cfg.metrics_addr,
        daemon = %cfg.daemon_api,
        "starting hpc-monitor"
    );

    let metrics = Metrics::new().context("initialising prometheus metrics")?;

    // Background scrape + degradation loop.
    let scrape = {
        let metrics = metrics.clone();
        let cfg = cfg.clone();
        tokio::spawn(async move { health::run(cfg, metrics).await })
    };

    // Prometheus exposition server.
    let app = Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/metrics", get(metrics_handler))
        .with_state(metrics);
    let listener = TcpListener::bind(cfg.metrics_addr)
        .await
        .with_context(|| format!("binding metrics listener on {}", cfg.metrics_addr))?;

    tokio::select! {
        r = axum::serve(listener, app) => {
            r.context("metrics server failed")?;
        }
        _ = scrape => {
            tracing::error!("scrape loop exited unexpectedly");
        }
        _ = shutdown_signal() => {
            tracing::info!("shutdown signal received; exiting");
        }
    }
    Ok(())
}

async fn metrics_handler(State(metrics): State<Metrics>) -> Response {
    match metrics.render() {
        Ok(body) => ([(header::CONTENT_TYPE, "text/plain; version=0.0.4")], body).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "failed to render metrics");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
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
