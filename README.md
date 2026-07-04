# hpc — HPC Filesystem Management Framework

A production-shaped control plane for managing parallel/distributed filesystems
across an HPC cluster: a central **daemon** tracks cluster state and issues
work, lightweight **agents** run on every node to report metrics and execute
filesystem/deploy commands, a **CLI** drives it all, a **monitor** exposes
Prometheus metrics with degradation detection, and a **benchmark** suite
measures I/O with real latency histograms.

Written in async Rust. Every crate propagates errors properly (no `unwrap()` in
library code), traces with `tracing`, and is configured with `serde` + TOML.
Daemon ⇄ agent communication is gRPC over `tonic`.

It also carries the systems-level and operational surface a real deployment
needs: a **C FFI bridge** for raw block I/O (`hpc-ffi`, built with `cc` +
`bindgen`), a `/proc`-based **multi-platform diagnostics** bundler (`hpc-diag`),
a set of production **operations scripts** (`scripts/`), and **CI/CD** for both
GitHub Actions and Jenkins. Contribution follows a Gerrit-style patch-based
review workflow — see [`CONTRIBUTING.md`](CONTRIBUTING.md).

```
                            ┌──────────────────────────────────────────────┐
                            │                 hpc-daemon                   │
   hpc (CLI) ───REST/JSON──▶│  axum REST API   +   Arc<RwLock<ClusterState>>│
                            │  gRPC ClusterService     │        │           │
 hpc-monitor ──scrape REST─▶│  redb (durable state) ◀──┘        │           │
   │  /metrics              └───────────▲───────────────────────┼───────────┘
   ▼                          gRPC      │                       │ push commands
 Prometheus              register/metrics│                      │ (server stream)
                          heartbeat      │                      ▼
                            ┌────────────┴───────────────────────────────────┐
                            │  hpc-agent (one per node)                       │
                            │  sysinfo + /proc metrics · executes fs/deploy   │
                            └─────────────────────────────────────────────────┘

 hpc-bench ── async I/O benchmark suite (seq/rand read/write, HdrHistogram) ──▶ used by `hpc bench`
 hpc-ffi  ── C POSIX I/O shim (cc + bindgen): raw pread/pwrite/fsync ──▶ used by hpc-bench
 hpc-diag ── /proc parsing + platform-difference detection ──▶ JSON diagnostic bundle
 hpc-core ── shared types · config · error · tracing (every crate depends on it)

 scripts/            ── setup · deploy-agent (ssh) · bench-run · log-collect · ci-local
 .github/workflows/  ── ci.yml (fmt·clippy·test·build) · bench.yml (criterion → PR comment)
 Jenkinsfile         ── declarative pipeline: Checkout→Build→Test→Bench→Package
```

## Architecture

```mermaid
flowchart TB
    subgraph Operators
        CLI["hpc-cli<br/>(node / fs / bench)"]
        PROM["Prometheus"]
    end

    subgraph ControlPlane["Control plane"]
        DAEMON["hpc-daemon<br/>axum REST + gRPC server<br/>Arc&lt;RwLock&lt;ClusterState&gt;&gt;"]
        STORE[("redb<br/>durable state")]
        MON["hpc-monitor<br/>exporter + degradation detector"]
    end

    subgraph Nodes["Cluster nodes"]
        A1["hpc-agent #1<br/>sysinfo + /proc"]
        A2["hpc-agent #2"]
        A3["hpc-agent #N"]
    end

    CLI -- "REST/JSON" --> DAEMON
    MON -- "scrape REST" --> DAEMON
    PROM -- "scrape /metrics" --> MON
    DAEMON <--> STORE
    A1 -- "register · metrics · heartbeat (gRPC)" --> DAEMON
    A2 --> DAEMON
    A3 --> DAEMON
    DAEMON -- "deploy / fs commands (server stream)" --> A1
    DAEMON --> A2
    DAEMON --> A3
```

### Why agents dial *out*

Node agents are gRPC **clients**; the daemon hosts the server. Storage/compute
nodes commonly sit behind restrictive firewalls, so having agents dial out
removes any inbound-connectivity requirement. The daemon still needs to *push*
work, so on registration each agent opens a long-lived **server-streaming** RPC
(`StreamCommands`) down which the daemon writes deploy and filesystem commands.
Agents report each command's outcome back with a unary RPC. See
[`proto/hpc.proto`](proto/hpc.proto).

## Crates

| Crate | Kind | Responsibility |
|-------|------|----------------|
| [`hpc-core`](hpc-core/) | lib | Shared domain types, TOML config, `thiserror` error type, `tracing` setup. Every crate depends on it. |
| [`hpc-daemon`](hpc-daemon/) | bin | Management server: gRPC `ClusterService`, `Arc<RwLock<…>>` cluster state, redb persistence, axum REST API. |
| [`hpc-agent`](hpc-agent/) | bin | Per-node agent: `sysinfo` + `/proc` metrics collection, gRPC client, command executor. |
| [`hpc-cli`](hpc-cli/) | bin (`hpc`) | Operator CLI: `node` (list/deploy/status), `fs` (mount/unmount/status), `bench` (run/report). |
| [`hpc-monitor`](hpc-monitor/) | bin | Prometheus `/metrics` endpoint, scrape loop, threshold-based degradation detection. |
| [`hpc-bench`](hpc-bench/) | lib + bench | Async I/O benchmark suite: sequential + random read/write, latency histograms, Criterion benches. Also drives the raw FFI path via `hpc-ffi`. |
| [`hpc-ffi`](hpc-ffi/) | lib | C-interop layer: a native POSIX block-I/O shim (`pread`/`pwrite`/`fsync`) compiled with `cc`, bound with `bindgen`, wrapped in a safe Rust API. The one crate with `unsafe`. |
| [`hpc-diag`](hpc-diag/) | lib + bin | Multi-platform diagnostics: parses `/proc/{meminfo,diskstats,net/dev,mounts,version}`, detects platform differences that could explain bugs, and emits a JSON bundle. |

## The control-plane protocol

`ClusterService` (defined in [`proto/hpc.proto`](proto/hpc.proto)):

| RPC | Direction | Purpose |
|-----|-----------|---------|
| `RegisterNode(NodeInfo) → RegisterAck` | agent → daemon | Announce a node (idempotent). |
| `Heartbeat(NodeRef) → HeartbeatAck` | agent → daemon | Cheap liveness; carries directives (e.g. `reregister`). |
| `ReportMetrics(stream MetricsReport) → MetricsAck` | agent → daemon | Client-streamed resource samples. |
| `StreamCommands(NodeRef) → stream Command` | daemon → agent | Server-streamed deploy/fs commands. |
| `ReportCommandResult(CommandResult) → Ack` | agent → daemon | Terminal command outcome. |

The four message types the framework centres on — `NodeInfo`, `MetricsReport`,
`DeployCommand`, `FsCommand` — plus their acks and a `Command` envelope
(`oneof { DeployCommand, FsCommand }`) are all defined there.

## Quick start

```bash
# 1. Build everything
cargo build --release

# 2. Run the daemon (defaults: gRPC :7443, REST :8080, state in /var/lib/hpc)
./target/release/hpc-daemon --config configs/daemon.toml

# 3. Run an agent on each node (dials the daemon)
./target/release/hpc-agent --config configs/agent.toml
#    …or override the endpoint inline:
./target/release/hpc-agent --endpoint http://daemon-host:7443

# 4. Drive it with the CLI
export HPC_API=http://127.0.0.1:8080
./target/release/hpc node list
./target/release/hpc node status my-node
./target/release/hpc fs mount my-node --device /dev/sdb1 --mount-point /mnt/scratch --fs-type xfs --opt noatime
./target/release/hpc fs status my-node

# 5. Expose metrics for Prometheus
./target/release/hpc-monitor --config configs/monitor.toml   # serves :9090/metrics

# 6. Benchmark a filesystem
./target/release/hpc bench run --path /mnt/scratch --file-size 268435456 --json report.json
./target/release/hpc bench report report.json
```

Example configuration files live in [`configs/`](configs/). Every field has a
built-in default, so a missing config file still yields a working process.

### What a live session looks like

```
$ hpc node list
┌─────────────┬─────────┬──────────┬─────┬─────┬──────┬───────────┐
│ NODE        ┆ ROLE    ┆ STATUS   ┆ CPU ┆ MEM ┆ DISK ┆ LAST SEEN │
╞═════════════╪═════════╪══════════╪═════╪═════╪══════╪═══════════╡
│ storage-01  ┆ storage ┆ healthy  ┆ 12% ┆ 41% ┆ 63%  ┆ 1s ago    │
│ storage-02  ┆ storage ┆ degraded ┆ 48% ┆ 86% ┆ 99%  ┆ 1s ago    │
└─────────────┴─────────┴──────────┴─────┴─────┴──────┴───────────┘

$ hpc fs mount storage-01 --device /dev/sdb1 --mount-point /mnt/scratch --fs-type xfs --opt noatime
mount accepted: command=fs-… node=storage-01 (/dev/sdb1 -> /mnt/scratch)
```

## Safety model

Filesystem operations are destructive and need root. The agent defaults to
`allow_exec = false`: commands are validated and the exact argv is logged and
returned as a **dry-run**, so the whole system is safe to run on a laptop or in
CI. Set `allow_exec = true` to actually spawn `mount`/`umount`/`fsck`. The
destructive `format` action additionally requires `force = true`, enforced both
at the REST boundary and in the executor.

## C FFI bridge (`hpc-ffi`)

`hpc-ffi` wraps a tiny native shim ([`hpc-ffi/src/hpc_io.c`](hpc-ffi/src/hpc_io.c))
that does positioned POSIX I/O — `open`, `pread`, `pwrite`, `fsync` — behind a
safe Rust API. The build script compiles the C with the [`cc`](https://docs.rs/cc)
crate and generates the `extern "C"` declarations with
[`bindgen`](https://docs.rs/bindgen), so the layering is:

```
safe Rust (BlockFile)  →  bindgen-generated decls  →  raw POSIX syscalls (C)
```

The shim returns a negated errno on failure, which the wrapper turns back into a
`std::io::Error` and surfaces as `HpcError::Ffi`. This is the **only** crate in
the workspace that contains `unsafe`; every `unsafe` block carries a `// SAFETY:`
justification, and the rest of the workspace keeps `#![forbid(unsafe_code)]`.
`hpc-bench` uses it for a synchronous raw-I/O benchmark (`ffi_raw`) to contrast
with the async buffered path.

## Diagnostics & bug analysis (`hpc-diag`)

`hpc-diag` gathers a machine-readable snapshot of a node for incident triage:

```bash
hpc-diag collect --output diag.json          # pretty JSON bundle to a file
hpc-diag collect --output - --compact | jq   # stream to stdout for piping
```

It parses `/proc/meminfo`, `/proc/diskstats`, `/proc/net/dev`, `/proc/mounts`
and `/proc/version` into typed structs, folds in any structured `DiagReport`s
contributed by other crates, and runs heuristics that flag **platform
differences that could explain bugs** — memory pressure, pre-`io_uring`
kernels, heterogeneous filesystems, ephemeral (`tmpfs`/`overlay`) state
directories, a read-only root. Collection never aborts on a missing source: on a
non-Linux host each unavailable collector is recorded as a warning inside the
bundle, so you always get a well-formed document describing exactly what could
and could not be observed.

## Operations scripts (`scripts/`)

Production-shaped Bash (`set -euo pipefail`, usage docs, colored output):

| Script | Purpose |
|--------|---------|
| [`setup.sh`](scripts/setup.sh) | Verify the Rust toolchain, install native deps (`protoc`, `libclang`), prime the cargo cache. |
| [`deploy-agent.sh`](scripts/deploy-agent.sh) | Build and deploy `hpc-agent` to a remote node over SSH (atomic binary swap, optional start). |
| [`bench-run.sh`](scripts/bench-run.sh) | Run `hpc bench` and archive a timestamped report + host metadata. |
| [`log-collect.sh`](scripts/log-collect.sh) | Pull logs and live diagnostics from a set of nodes into a single tar bundle. |
| [`ci-local.sh`](scripts/ci-local.sh) | Run the full CI gate (fmt · clippy · test · build) locally before pushing. |

## CI/CD

- **GitHub Actions** — [`ci.yml`](.github/workflows/ci.yml) runs fmt, clippy
  (`-D warnings`), tests and a release build on every push and PR, with cargo
  caching; [`bench.yml`](.github/workflows/bench.yml) runs the Criterion
  benchmarks and posts the numbers as a PR comment (and stores a baseline on
  `main`).
- **Jenkins** — [`Jenkinsfile`](Jenkinsfile) is a declarative pipeline
  (`Checkout → Toolchain → Lint → Build → Test → Bench → Package`) that archives
  the release binaries and benchmark output and reports status back to GitHub.

Both enforce the same gate as `scripts/ci-local.sh`. Review follows a
Gerrit-style patch-based workflow documented in [`CONTRIBUTING.md`](CONTRIBUTING.md).

## Engineering conventions

- **No `unwrap()` in library code.** Everything returns
  `hpc_core::Result<T>` (a `thiserror` enum) and propagates with `?`. Binaries
  use `anyhow` for top-level context; `unwrap`/`expect` appear only in tests and
  the Criterion harness.
- **Async-safe shared state.** The daemon's cluster state is
  `Arc<RwLock<ClusterState>>`, written through to redb on every mutation and
  rehydrated on restart.
- **Tracing everywhere**, configurable via TOML or `RUST_LOG`, human or JSON.
- **One source of truth for types.** `hpc-core` owns the domain model; the
  daemon/agent convert to/from protobuf only at the gRPC boundary.

## Development

```bash
scripts/setup.sh                # one-time: toolchain + native deps + cargo fetch
scripts/ci-local.sh             # the full gate: fmt · clippy · test · build

# …or the individual steps:
cargo test --workspace          # unit + integration tests
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
cargo bench -p hpc-bench        # Criterion I/O micro-benchmarks (async + raw FFI)
```

Requires a recent stable Rust (1.82+), `protoc` (the Protocol Buffers compiler)
on `PATH` for the daemon/agent build scripts, and `libclang` for `hpc-ffi`'s
`bindgen` step. `scripts/setup.sh` installs all three.

## License

Dual-licensed under either of Apache-2.0 or MIT at your option.
