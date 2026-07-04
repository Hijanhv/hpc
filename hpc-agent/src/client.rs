//! The agent's gRPC client and session lifecycle.
//!
//! The agent is a pure gRPC *client*: it dials the daemon and then runs a
//! single session that multiplexes three concerns over one HTTP/2 channel:
//!
//! 1. **metrics** — sample the node every `metrics_interval` and push a report,
//! 2. **heartbeat** — a cheap liveness ping every `heartbeat_interval`, and
//! 3. **commands** — a long-lived server stream on which the daemon pushes
//!    deploy/filesystem work; each command is executed and its outcome reported.
//!
//! Any transport error tears down the session; [`run`] then reconnects with
//! capped exponential backoff. This is the standard resilient-agent pattern.

use std::time::Duration;

use hpc_core::config::AgentConfig;
use hpc_core::error::{HpcError, Result};
use hpc_core::types::{DeploySpec, FsSpec};
use tonic::transport::{Channel, Endpoint};

use crate::executor::Executor;
use crate::metrics::Collector;
use crate::proto::pb;
use crate::proto::pb::cluster_service_client::ClusterServiceClient;

/// Longest backoff between reconnect attempts.
const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// Run the agent until the process is terminated. Reconnects on failure.
pub async fn run(cfg: AgentConfig) -> Result<()> {
    let node_id = resolve_node_id(&cfg);
    tracing::info!(%node_id, endpoint = %cfg.daemon_endpoint, "agent starting");

    let mut collector = Collector::new(node_id.clone(), cfg.role);
    let mut node_info = collector.node_info(hpc_core::VERSION);
    node_info.labels = cfg.labels.clone();
    node_info.role = cfg.role;
    let executor = Executor::new(node_id.clone(), cfg.allow_exec);

    let mut backoff = cfg.reconnect_backoff;
    loop {
        match session(&cfg, &node_info, &mut collector, &executor).await {
            Ok(()) => {
                tracing::warn!("daemon closed the command stream; reconnecting");
            }
            Err(e) => {
                tracing::warn!(error = %e, backoff = ?backoff, "session ended with error; reconnecting");
            }
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(MAX_BACKOFF);
        // On a clean stream close we reset backoff; on repeated errors it grows.
    }
}

/// One connected session. Returns `Ok(())` if the daemon closes the command
/// stream, or an error on any transport/RPC failure (triggering reconnect).
async fn session(
    cfg: &AgentConfig,
    node_info: &hpc_core::types::NodeInfo,
    collector: &mut Collector,
    executor: &Executor,
) -> Result<()> {
    let mut client = connect(&cfg.daemon_endpoint).await?;

    let pb_info: pb::NodeInfo = node_info.clone().into();
    let ack = client
        .register_node(pb_info)
        .await
        .map_err(status_to_err)?
        .into_inner();
    let epoch = ack.cluster_epoch;
    let metrics_interval = if ack.metrics_interval_secs > 0 {
        Duration::from_secs(ack.metrics_interval_secs as u64)
    } else {
        cfg.metrics_interval
    };
    tracing::info!(
        node = %node_info.node_id,
        epoch,
        interval = ?metrics_interval,
        "registered with daemon"
    );

    // Open the daemon->agent command stream on a cloned client handle.
    let mut cmd_client = client.clone();
    let mut inbound = cmd_client
        .stream_commands(pb::NodeRef {
            node_id: node_info.node_id.clone(),
            cluster_epoch: epoch,
        })
        .await
        .map_err(status_to_err)?
        .into_inner();

    let mut metrics_timer = tokio::time::interval(metrics_interval);
    let mut heartbeat_timer = tokio::time::interval(cfg.heartbeat_interval);

    loop {
        tokio::select! {
            _ = metrics_timer.tick() => {
                let report = collector.collect();
                let pb_report: pb::MetricsReport = report.into();
                client
                    .report_metrics(tokio_stream::once(pb_report))
                    .await
                    .map_err(status_to_err)?;
            }
            _ = heartbeat_timer.tick() => {
                let ack = client
                    .heartbeat(pb::NodeRef { node_id: node_info.node_id.clone(), cluster_epoch: epoch })
                    .await
                    .map_err(status_to_err)?
                    .into_inner();
                if ack.directives.iter().any(|d| d == "reregister") {
                    tracing::info!("daemon requested re-registration");
                    return Ok(());
                }
            }
            message = inbound.message() => {
                match message.map_err(status_to_err)? {
                    Some(command) => handle_command(&mut client, executor, command).await?,
                    None => return Ok(()), // daemon closed the stream
                }
            }
        }
    }
}

/// Execute one command and report its outcome back to the daemon.
async fn handle_command(
    client: &mut ClusterServiceClient<Channel>,
    executor: &Executor,
    command: pb::Command,
) -> Result<()> {
    let outcome = match command.payload {
        Some(pb::command::Payload::Deploy(d)) => {
            let spec: DeploySpec = d.into();
            executor.run_deploy(&spec).await
        }
        Some(pb::command::Payload::Fs(f)) => {
            let spec: FsSpec = f.into();
            executor.run_fs(&spec).await
        }
        None => {
            tracing::warn!(command_id = %command.command_id, "command with empty payload");
            return Ok(());
        }
    };

    let result: pb::CommandResult = outcome.into();
    client
        .report_command_result(result)
        .await
        .map_err(status_to_err)?;
    Ok(())
}

/// Establish a channel to the daemon's gRPC endpoint.
async fn connect(endpoint: &str) -> Result<ClusterServiceClient<Channel>> {
    let channel = Endpoint::from_shared(endpoint.to_string())
        .map_err(|e| HpcError::Rpc(format!("invalid endpoint {endpoint}: {e}")))?
        .connect_timeout(Duration::from_secs(5))
        .connect()
        .await
        .map_err(|e| HpcError::Rpc(format!("connecting to {endpoint}: {e}")))?;
    Ok(ClusterServiceClient::new(channel))
}

fn status_to_err(status: tonic::Status) -> HpcError {
    HpcError::Rpc(format!("{}: {}", status.code(), status.message()))
}

/// Resolve the node id: explicit config wins, else the machine hostname, else a
/// stable fallback so registration always succeeds.
fn resolve_node_id(cfg: &AgentConfig) -> String {
    if !cfg.node_id.trim().is_empty() {
        return cfg.node_id.clone();
    }
    sysinfo::System::host_name().unwrap_or_else(|| "unknown-node".to_string())
}
