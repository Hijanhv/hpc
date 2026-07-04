# hpc-daemon

The cluster management server. It hosts two network surfaces over one shared,
async-safe state object and persists everything to an embedded database.

## What it does

- **gRPC control plane** (`ClusterService`, port `7443` by default) — agents
  dial in to register, heartbeat, stream metrics, receive pushed commands, and
  report command outcomes. Built with `tonic`; the protocol is
  [`proto/hpc.proto`](../proto/hpc.proto), compiled by `build.rs` via
  `tonic-prost-build`.
- **REST/JSON API** (`axum`, port `8080` by default) — the human- and
  tooling-facing surface used by the CLI and monitor.
- **Durable state** — an embedded [`redb`](https://docs.rs/redb) database holds
  node records and a command-outcome audit log; in-memory state is rehydrated
  from it on startup.
- **Liveness reaping** — a background task marks nodes `Unreachable` when they
  stop heartbeating within `node_timeout`.

## Module layout

| File | Role |
|------|------|
| `main.rs` | Startup: config, telemetry, store, state, and the select over gRPC + HTTP + reaper + shutdown. |
| `proto.rs` | Generated protobuf types plus every conversion to/from `hpc_core::types`. |
| `state.rs` | `SharedState` = `Arc<RwLock<ClusterState>>` + per-node command channels. All the concurrency lives here. |
| `store.rs` | `redb` persistence (nodes + command outcomes). |
| `grpc.rs` | `ClusterService` implementation, including the server-streaming `StreamCommands` bridge. |
| `api.rs` | axum router, handlers, and `HpcError → HTTP status` mapping. |

## Shared-state model

`SharedState` is cheap to clone and wraps two `Arc<RwLock<…>>` maps:

1. the **node table** (`NodeId → NodeRecord`), written through to redb, and
2. a registry of **live command channels** (`NodeId → mpsc::Sender<NodeCommand>`).

When an agent opens `StreamCommands`, the daemon parks the receiving half and
returns a stream; a REST `POST …/deploy` or `…/fs` looks up the sender and
pushes a `Command` down it. This is how a firewalled, dial-out-only agent still
receives pushed work.

## REST API

| Method & path | Purpose |
|---------------|---------|
| `GET /health` | Liveness probe. |
| `GET /api/v1/cluster/status` | Aggregate counts + connected streams. |
| `GET /api/v1/nodes` | All node records. |
| `GET /api/v1/nodes/{id}` | One node record. |
| `DELETE /api/v1/nodes/{id}` | Deregister a node. |
| `GET /api/v1/nodes/{id}/metrics` | Latest metrics sample. |
| `POST /api/v1/nodes/{id}/deploy` | Dispatch a deploy command. |
| `POST /api/v1/nodes/{id}/fs` | Dispatch a filesystem command. |
| `GET /api/v1/outcomes` | Command-outcome audit log. |

## Run it

```bash
hpc-daemon --config configs/daemon.toml
# or with defaults + env overrides:
HPC_DAEMON_CONFIG=/etc/hpc/daemon.toml hpc-daemon
```

See [`configs/daemon.toml`](../configs/daemon.toml) for all options.
