//! The daemon-side implementation of `ClusterService`.
//!
//! Agents are gRPC clients that dial in here. This module translates each RPC
//! into an operation on [`SharedState`], converting protobuf messages to domain
//! types on the way in and back on the way out. The one interesting piece is
//! [`stream_commands`](ClusterRpc::stream_commands): a server-streaming RPC that
//! bridges an agent's live command channel to the wire, so the daemon can push
//! deploy/filesystem work to a node that only ever dialled *out*.

use std::pin::Pin;

use hpc_core::types::now_unix;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::{Stream, StreamExt};
use tonic::{Request, Response, Status};

use crate::proto::pb;
use crate::proto::pb::cluster_service_server::{ClusterService, ClusterServiceServer};
use crate::state::{NodeCommand, SharedState};

/// gRPC front-end over [`SharedState`].
#[derive(Debug, Clone)]
pub struct ClusterRpc {
    state: SharedState,
}

impl ClusterRpc {
    /// Wrap shared state into a ready-to-serve gRPC service.
    pub fn into_service(state: SharedState) -> ClusterServiceServer<ClusterRpc> {
        ClusterServiceServer::new(ClusterRpc { state })
    }
}

fn to_pb_command(cmd: NodeCommand) -> pb::Command {
    let (command_id, payload) = match cmd {
        NodeCommand::Deploy(d) => {
            let id = d.deployment_id.clone();
            (id, pb::command::Payload::Deploy(d.into()))
        }
        NodeCommand::Fs(f) => {
            let id = f.command_id.clone();
            (id, pb::command::Payload::Fs(f.into()))
        }
    };
    pb::Command {
        command_id,
        issued_at_unix: now_unix(),
        payload: Some(payload),
    }
}

#[tonic::async_trait]
impl ClusterService for ClusterRpc {
    async fn register_node(
        &self,
        request: Request<pb::NodeInfo>,
    ) -> Result<Response<pb::RegisterAck>, Status> {
        let info = request.into_inner().into();
        let record = self
            .state
            .register_node(info)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;
        Ok(Response::new(pb::RegisterAck {
            accepted: true,
            assigned_node_id: record.info.node_id,
            cluster_epoch: self.state.epoch(),
            metrics_interval_secs: self.state.metrics_interval_secs(),
            message: "registered".into(),
        }))
    }

    async fn heartbeat(
        &self,
        request: Request<pb::NodeRef>,
    ) -> Result<Response<pb::HeartbeatAck>, Status> {
        let node_ref = request.into_inner();
        // A mismatched epoch means the daemon restarted; tell the agent to
        // re-register so its static info is repopulated.
        let mut directives = Vec::new();
        if node_ref.cluster_epoch != 0 && node_ref.cluster_epoch != self.state.epoch() {
            directives.push("reregister".to_string());
        }
        match self.state.heartbeat(&node_ref.node_id).await {
            Ok(()) => {}
            Err(_) => directives.push("reregister".to_string()),
        }
        Ok(Response::new(pb::HeartbeatAck {
            ok: true,
            cluster_epoch: self.state.epoch(),
            directives,
        }))
    }

    async fn report_metrics(
        &self,
        request: Request<tonic::Streaming<pb::MetricsReport>>,
    ) -> Result<Response<pb::MetricsAck>, Status> {
        let mut stream = request.into_inner();
        let mut received = 0u64;
        while let Some(item) = stream.next().await {
            let report = item?;
            self.state
                .ingest_metrics(report.into())
                .await
                .map_err(|e| Status::internal(e.to_string()))?;
            received += 1;
        }
        tracing::debug!(received, "metrics batch ingested");
        Ok(Response::new(pb::MetricsAck {
            received: received > 0,
            server_time_unix: now_unix(),
        }))
    }

    type StreamCommandsStream =
        Pin<Box<dyn Stream<Item = Result<pb::Command, Status>> + Send + 'static>>;

    async fn stream_commands(
        &self,
        request: Request<pb::NodeRef>,
    ) -> Result<Response<Self::StreamCommandsStream>, Status> {
        let node_id = request.into_inner().node_id;
        if self.state.get_node(&node_id).await.is_none() {
            return Err(Status::not_found(format!("unknown node {node_id}")));
        }

        let mut commands = self.state.attach_command_channel(node_id.clone()).await;
        let (out_tx, out_rx) = mpsc::channel::<Result<pb::Command, Status>>(64);
        let state = self.state.clone();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    // Detect the client hanging up even when idle.
                    _ = out_tx.closed() => break,
                    maybe = commands.recv() => match maybe {
                        Some(cmd) => {
                            if out_tx.send(Ok(to_pb_command(cmd))).await.is_err() {
                                break;
                            }
                        }
                        None => break,
                    },
                }
            }
            state.detach_command_channel(&node_id).await;
        });

        let stream = ReceiverStream::new(out_rx);
        Ok(Response::new(Box::pin(stream)))
    }

    async fn report_command_result(
        &self,
        request: Request<pb::CommandResult>,
    ) -> Result<Response<pb::Ack>, Status> {
        let outcome = request.into_inner().into();
        self.state
            .record_outcome(outcome)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;
        Ok(Response::new(pb::Ack {
            ok: true,
            message: "recorded".into(),
        }))
    }
}
