//! Unified error type for the whole framework.
//!
//! Every fallible library function in the workspace returns
//! [`Result<T>`](crate::Result), whose error arm is [`HpcError`]. Concrete
//! error sources (I/O, TOML, serde, gRPC transport…) are captured as distinct
//! variants so callers can match on them, and `#[from]` conversions keep the
//! `?` operator ergonomic without ever resorting to `unwrap()`.

use std::path::PathBuf;

/// The crate-wide result alias.
pub type Result<T> = std::result::Result<T, HpcError>;

/// The single error type surfaced by every `hpc-*` library crate.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum HpcError {
    /// An underlying I/O failure, annotated with the path that caused it when
    /// one is available.
    #[error("i/o error{}: {source}", .path.as_ref().map(|p| format!(" at {}", p.display())).unwrap_or_default())]
    Io {
        path: Option<PathBuf>,
        #[source]
        source: std::io::Error,
    },

    /// A configuration file could not be parsed.
    #[error("failed to parse configuration: {0}")]
    ConfigParse(#[from] toml::de::Error),

    /// A configuration value could not be serialized back to TOML.
    #[error("failed to serialize configuration: {0}")]
    ConfigSerialize(#[from] toml::ser::Error),

    /// A configuration file was structurally valid but semantically invalid.
    #[error("invalid configuration: {0}")]
    ConfigInvalid(String),

    /// JSON (de)serialization failure — used for the persisted state records.
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    /// The embedded key/value store reported an error.
    #[error("state store error: {0}")]
    Store(String),

    /// A requested entity (node, deployment, filesystem…) does not exist.
    #[error("not found: {0}")]
    NotFound(String),

    /// The requested operation is invalid for the current state.
    #[error("invalid state: {0}")]
    InvalidState(String),

    /// A gRPC / transport-layer failure.
    #[error("rpc error: {0}")]
    Rpc(String),

    /// Metrics collection failed.
    #[error("metrics collection error: {0}")]
    Metrics(String),

    /// A benchmark could not be executed.
    #[error("benchmark error: {0}")]
    Bench(String),

    /// A command (deploy / filesystem op) failed to execute.
    #[error("command execution failed: {0}")]
    Command(String),

    /// A value could not be represented in the target protocol type.
    #[error("protocol conversion error: {0}")]
    Conversion(String),
}

impl HpcError {
    /// Construct an [`HpcError::Io`] without an associated path.
    pub fn io(source: std::io::Error) -> Self {
        HpcError::Io { path: None, source }
    }

    /// Construct an [`HpcError::Io`] annotated with the offending path.
    pub fn io_at(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        HpcError::Io {
            path: Some(path.into()),
            source,
        }
    }

    /// Convenience constructor for [`HpcError::Store`].
    pub fn store(msg: impl std::fmt::Display) -> Self {
        HpcError::Store(msg.to_string())
    }

    /// Convenience constructor for [`HpcError::InvalidState`].
    pub fn invalid_state(msg: impl std::fmt::Display) -> Self {
        HpcError::InvalidState(msg.to_string())
    }

    /// Convenience constructor for [`HpcError::Conversion`].
    pub fn conversion(msg: impl std::fmt::Display) -> Self {
        HpcError::Conversion(msg.to_string())
    }

    /// True when the error is plausibly transient and the operation could be
    /// retried (used by the agent's reconnect loop).
    pub fn is_retryable(&self) -> bool {
        matches!(self, HpcError::Rpc(_) | HpcError::Io { .. })
    }
}
