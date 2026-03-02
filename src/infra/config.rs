use std::fs;
use std::path::{Path, PathBuf};

use directories::ProjectDirs;
use serde::Deserialize;

use crate::infra::error::{CourierError, ErrorCode, Result};

#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub config_path: PathBuf,
    pub data_dir: PathBuf,
    pub database_path: PathBuf,
    pub b4_path: Option<PathBuf>,
    pub log_filter: String,
}

#[derive(Debug, Default, Deserialize)]
struct FileConfig {
    #[serde(default)]
    storage: StorageConfig,
    #[serde(default)]
    b4: B4Config,
    #[serde(default)]
    logging: LoggingConfig,
}

#[derive(Debug, Default, Deserialize)]
struct StorageConfig {
    data_dir: Option<PathBuf>,
    database: Option<PathBuf>,
}

#[derive(Debug, Default, Deserialize)]
struct B4Config {
    path: Option<PathBuf>,
}

#[derive(Debug, Default, Deserialize)]
struct LoggingConfig {
    filter: Option<String>,
}

pub fn load(config_override: Option<&Path>) -> Result<RuntimeConfig> {
    let project_dirs = ProjectDirs::from("org", "courier", "courier").ok_or_else(|| {
        CourierError::new(
            ErrorCode::ConfigRead,
            "failed to derive project directories from current platform",
        )
    })?;

    let config_path = config_override
        .map(Path::to_path_buf)
        .unwrap_or_else(|| project_dirs.config_dir().join("config.toml"));

    let file_config = if config_path.exists() {
        parse_config_file(&config_path)?
    } else {
        FileConfig::default()
    };

    let config_base_dir = config_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| project_dirs.config_dir().to_path_buf());

    let data_dir = file_config
        .storage
        .data_dir
        .map(|path| resolve_path(&config_base_dir, path))
        .unwrap_or_else(|| project_dirs.data_local_dir().to_path_buf());

    let database_path = file_config
        .storage
        .database
        .map(|path| resolve_path(&config_base_dir, path))
        .unwrap_or_else(|| data_dir.join("courier.db"));

    let b4_path = file_config
        .b4
        .path
        .map(|path| resolve_path(&config_base_dir, path));

    let log_filter = file_config
        .logging
        .filter
        .unwrap_or_else(|| "info".to_string());

    Ok(RuntimeConfig {
        config_path,
        data_dir,
        database_path,
        b4_path,
        log_filter,
    })
}

fn parse_config_file(path: &Path) -> Result<FileConfig> {
    let content = fs::read_to_string(path).map_err(|error| {
        CourierError::with_source(
            ErrorCode::ConfigRead,
            format!("failed to read config file {}", path.display()),
            error,
        )
    })?;

    toml::from_str::<FileConfig>(&content).map_err(|error| {
        CourierError::with_source(
            ErrorCode::ConfigParse,
            format!("failed to parse TOML config {}", path.display()),
            error,
        )
    })
}

fn resolve_path(base_dir: &Path, candidate: PathBuf) -> PathBuf {
    if candidate.is_absolute() {
        candidate
    } else {
        base_dir.join(candidate)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::load;

    fn temp_dir(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("courier-{label}-{nonce}"));
        fs::create_dir_all(&path).expect("create temp dir");
        path
    }

    #[test]
    fn resolves_relative_paths_from_config_dir() {
        let base = temp_dir("config-relative");
        let config_path = base.join("config.toml");

        fs::write(
            &config_path,
            r#"
[storage]
data_dir = "./data"
database = "./state/courier.sqlite"

[b4]
path = "./bin/b4"

[logging]
filter = "debug"
"#,
        )
        .expect("write config");

        let loaded = load(Some(&config_path)).expect("load config");

        assert_eq!(loaded.data_dir, base.join("./data"));
        assert_eq!(loaded.database_path, base.join("./state/courier.sqlite"));
        assert_eq!(loaded.b4_path, Some(base.join("./bin/b4")));
        assert_eq!(loaded.log_filter, "debug");

        let _ = fs::remove_dir_all(base);
    }
}
