# hpc-core

The foundational library every other crate in the workspace depends on. It has
no knowledge of gRPC, HTTP, or storage engines — it defines the shared
vocabulary and cross-cutting concerns so the daemon, agent, CLI, monitor, and
bench suite all speak the same data model.

## Modules

- **`error`** — the single `HpcError` enum (`thiserror`) and the crate-wide
  `Result<T>` alias. Distinct variants for I/O (with path context), TOML
  parse/serialize, JSON, store, not-found, invalid-state, RPC, metrics, bench,
  command, and conversion failures, each with `#[from]` conversions so `?` works
  everywhere. `HpcError::is_retryable()` drives the agent's reconnect logic.
- **`types`** — transport-agnostic, `serde`-serialisable domain types:
  `NodeInfo`, `NodeRecord`, `NodeStatus`, `NodeRole`, `MetricsReport` (CPU /
  memory / load / disk / network / filesystem), `DeploySpec`, `FsSpec`,
  `CommandOutcome`, and the benchmark result types (`BenchReport`,
  `BenchResult`, `LatencyStats`, `IoPattern`). Includes helpers like
  `MemoryMetrics::used_fraction()` and `now_unix()`.
- **`config`** — strongly-typed configuration structs (`DaemonConfig`,
  `AgentConfig`, `MonitorConfig`, `Thresholds`, `LogConfig`) with `serde`
  defaults, `humantime` duration parsing, `load()` / `load_or_default()`
  helpers, and cross-field `validate()` methods.
- **`telemetry`** — one-call `tracing-subscriber` initialisation honouring
  `RUST_LOG`, with a human or JSON formatter.

## Design notes

- **No panics.** No function here calls `unwrap()`/`expect()`; even the
  `Default` impls that parse socket addresses fall back with `unwrap_or_else`.
- **Leaf crate.** Keeping types/config/error/telemetry in a dependency-light
  leaf is what lets every binary share one consistent model and one error type.

```rust
use hpc_core::config::{self, DaemonConfig};

let cfg: DaemonConfig = config::load_or_default("daemon.toml")?;
hpc_core::telemetry::init(&cfg.log)?;
cfg.validate()?;
```

Run the unit tests with `cargo test -p hpc-core`.
