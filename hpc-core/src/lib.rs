//! # hpc-core
//!
//! Foundational library for the HPC filesystem management framework. It has no
//! knowledge of gRPC, HTTP or storage engines — it defines the vocabulary the
//! rest of the workspace is built from:
//!
//! * [`error`] — the single [`HpcError`]/[`Result`] type used everywhere, so no
//!   library function ever needs to `unwrap()`.
//! * [`types`] — transport-agnostic domain types (nodes, metrics, commands,
//!   benchmark results) that are `serde`-serialisable.
//! * [`config`] — strongly-typed, TOML-backed configuration for each binary.
//! * [`telemetry`] — one-line `tracing` initialisation.
//!
//! Every downstream crate depends on this one; keeping these concerns in a
//! leaf crate is what lets the daemon, agent, CLI, monitor and bench suite
//! share a single, consistent data model.
#![forbid(unsafe_code)]
#![warn(missing_debug_implementations)]

pub mod config;
pub mod error;
pub mod telemetry;
pub mod types;

pub use error::{HpcError, Result};

/// The workspace version, surfaced by agents as their `agent_version`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
