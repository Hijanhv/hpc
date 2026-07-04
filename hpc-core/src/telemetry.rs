//! Tracing/telemetry initialisation shared by every binary.
//!
//! Call [`init`] exactly once at process start. It wires up
//! `tracing-subscriber` with an env-filter (overridable via `RUST_LOG`), and
//! either a human-readable or JSON formatter depending on [`LogConfig`].

use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

use crate::config::LogConfig;
use crate::error::{HpcError, Result};

/// Initialise the global tracing subscriber from `cfg`.
///
/// The `RUST_LOG` environment variable, when set, takes precedence over
/// `cfg.filter` so operators can crank up verbosity without editing config.
///
/// Returns an error (rather than panicking) if a subscriber is already
/// installed or the filter directive is malformed.
pub fn init(cfg: &LogConfig) -> Result<()> {
    let filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(&cfg.filter))
        .map_err(|e| HpcError::ConfigInvalid(format!("invalid log filter: {e}")))?;

    let registry = tracing_subscriber::registry().with(filter);

    if cfg.json {
        let fmt = tracing_subscriber::fmt::layer()
            .json()
            .with_current_span(true)
            .with_file(cfg.with_location)
            .with_line_number(cfg.with_location);
        registry
            .with(fmt)
            .try_init()
            .map_err(|e| HpcError::InvalidState(format!("tracing already initialised: {e}")))
    } else {
        let fmt = tracing_subscriber::fmt::layer()
            .with_target(true)
            .with_file(cfg.with_location)
            .with_line_number(cfg.with_location);
        registry
            .with(fmt)
            .try_init()
            .map_err(|e| HpcError::InvalidState(format!("tracing already initialised: {e}")))
    }
}
