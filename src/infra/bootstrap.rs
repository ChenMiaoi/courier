use std::fs;

use crate::infra::config::RuntimeConfig;
use crate::infra::db::{self, DatabaseState};
use crate::infra::error::{CourierError, ErrorCode, Result};

#[derive(Debug, Clone)]
pub struct BootstrapState {
    pub db: DatabaseState,
}

pub fn prepare(config: &RuntimeConfig) -> Result<BootstrapState> {
    ensure_runtime_dirs(config)?;
    let db_state = db::initialize(&config.database_path)?;

    Ok(BootstrapState { db: db_state })
}

fn ensure_runtime_dirs(config: &RuntimeConfig) -> Result<()> {
    if let Some(config_dir) = config.config_path.parent() {
        fs::create_dir_all(config_dir).map_err(|error| {
            CourierError::with_source(
                ErrorCode::Io,
                format!("failed to create config directory {}", config_dir.display()),
                error,
            )
        })?;
    }

    fs::create_dir_all(&config.data_dir).map_err(|error| {
        CourierError::with_source(
            ErrorCode::Io,
            format!(
                "failed to create data directory {}",
                config.data_dir.display()
            ),
            error,
        )
    })?;

    if let Some(db_dir) = config.database_path.parent() {
        fs::create_dir_all(db_dir).map_err(|error| {
            CourierError::with_source(
                ErrorCode::Io,
                format!("failed to create database directory {}", db_dir.display()),
                error,
            )
        })?;
    }

    Ok(())
}
