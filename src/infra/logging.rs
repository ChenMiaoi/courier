//! Tracing subscriber initialization.
//!
//! Logging is process-global state, so this module owns the once-only setup and
//! keeps the background writer guard alive for the lifetime of the process.

use std::path::Path;
use std::sync::{Once, OnceLock};

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::prelude::*;

use crate::infra::error::{CourierError, ErrorCode, Result};

static INIT: Once = Once::new();
static LOG_GUARD: OnceLock<WorkerGuard> = OnceLock::new();

pub fn init(default_filter: &str, log_dir: &Path) -> Result<()> {
    let mut result = Ok(());

    INIT.call_once(|| {
        if let Err(err) = std::fs::create_dir_all(log_dir) {
            result = Err(CourierError::with_source(
                ErrorCode::LoggingInit,
                format!("failed to create log directory {}", log_dir.display()),
                err,
            ));
            return;
        }

        let env_filter = EnvFilter::try_from_default_env()
            .or_else(|_| EnvFilter::try_new(default_filter))
            .unwrap_or_else(|_| EnvFilter::new("info"));

        let appender = tracing_appender::rolling::daily(log_dir, "courier.log");
        let (non_blocking, guard) = tracing_appender::non_blocking(appender);
        // Dropping the guard early can lose buffered log lines during shutdown,
        // so store it in a process-wide cell once initialization succeeds.
        let _ = LOG_GUARD.set(guard);
        let error_appender = tracing_appender::rolling::never(log_dir, "error.log");

        let normal_layer = tracing_subscriber::fmt::layer()
            .with_target(false)
            .with_ansi(false)
            .with_writer(non_blocking)
            .compact()
            .with_filter(env_filter);

        let error_file_layer = tracing_subscriber::fmt::layer()
            .with_target(false)
            .with_ansi(false)
            .with_writer(error_appender)
            .compact()
            .with_filter(LevelFilter::ERROR);

        if let Err(err) = tracing_subscriber::registry()
            .with(normal_layer)
            .with(error_file_layer)
            .try_init()
        {
            result = Err(CourierError::new(
                ErrorCode::LoggingInit,
                format!("failed to initialize tracing subscriber: {err}"),
            ));
        }
    });

    result
}
