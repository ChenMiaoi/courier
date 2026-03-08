//! Runtime bootstrap for filesystem and database prerequisites.
//!
//! Startup code goes through this module before touching higher-level features
//! so later layers can assume the basic runtime directories and schema already
//! exist.

use std::fs;

use crate::infra::config::RuntimeConfig;
use crate::infra::db::{self, DatabaseState};
use crate::infra::error::{CriewError, ErrorCode, Result};
use crate::infra::mail_store;

const THREAD_DATE_ORDER_MIGRATION_VERSION: i64 = 4;

#[derive(Debug, Clone)]
pub struct BootstrapState {
    pub db: DatabaseState,
}

pub fn prepare(config: &RuntimeConfig) -> Result<BootstrapState> {
    // Make filesystem state explicit up front so later failures point at the
    // actual feature operation, not at some missing directory deep in the call
    // stack.
    ensure_runtime_dirs(config)?;
    let db_state = db::initialize(&config.database_path)?;
    if db_state
        .applied_migrations
        .contains(&THREAD_DATE_ORDER_MIGRATION_VERSION)
    {
        let rebuilt = mail_store::rebuild_all_threads(&config.database_path)?;
        tracing::info!(
            database = %config.database_path.display(),
            rebuilt_threads = rebuilt,
            "rebuilt thread ordering after schema upgrade"
        );
    }

    Ok(BootstrapState { db: db_state })
}

fn ensure_runtime_dirs(config: &RuntimeConfig) -> Result<()> {
    if let Some(config_dir) = config.config_path.parent() {
        fs::create_dir_all(config_dir).map_err(|error| {
            CriewError::with_source(
                ErrorCode::Io,
                format!("failed to create config directory {}", config_dir.display()),
                error,
            )
        })?;
    }

    fs::create_dir_all(&config.data_dir).map_err(|error| {
        CriewError::with_source(
            ErrorCode::Io,
            format!(
                "failed to create data directory {}",
                config.data_dir.display()
            ),
            error,
        )
    })?;

    fs::create_dir_all(&config.raw_mail_dir).map_err(|error| {
        CriewError::with_source(
            ErrorCode::Io,
            format!(
                "failed to create raw mail directory {}",
                config.raw_mail_dir.display()
            ),
            error,
        )
    })?;

    fs::create_dir_all(&config.patch_dir).map_err(|error| {
        CriewError::with_source(
            ErrorCode::Io,
            format!(
                "failed to create patch directory {}",
                config.patch_dir.display()
            ),
            error,
        )
    })?;

    fs::create_dir_all(&config.log_dir).map_err(|error| {
        CriewError::with_source(
            ErrorCode::Io,
            format!(
                "failed to create log directory {}",
                config.log_dir.display()
            ),
            error,
        )
    })?;

    if let Some(db_dir) = config.database_path.parent() {
        fs::create_dir_all(db_dir).map_err(|error| {
            CriewError::with_source(
                ErrorCode::Io,
                format!("failed to create database directory {}", db_dir.display()),
                error,
            )
        })?;
    }

    Ok(())
}
