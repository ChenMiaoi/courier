use std::fs;
use std::path::{Path, PathBuf};

use directories::UserDirs;
use serde::Deserialize;

use crate::infra::error::{CourierError, ErrorCode, Result};

const DEFAULT_MAILBOX: &str = "linux-kernel";
const DEFAULT_LORE_BASE_URL: &str = "https://lore.kernel.org";

#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub config_path: PathBuf,
    pub data_dir: PathBuf,
    pub database_path: PathBuf,
    pub raw_mail_dir: PathBuf,
    pub patch_dir: PathBuf,
    pub log_dir: PathBuf,
    pub b4_path: Option<PathBuf>,
    pub log_filter: String,
    pub imap_mailbox: String,
    pub lore_base_url: String,
    pub kernel_trees: Vec<PathBuf>,
}

#[derive(Debug, Default, Deserialize)]
struct FileConfig {
    #[serde(default)]
    storage: StorageConfig,
    #[serde(default)]
    b4: B4Config,
    #[serde(default)]
    logging: LoggingConfig,
    #[serde(default)]
    source: SourceConfig,
    #[serde(default)]
    imap: ImapCompatConfig,
    #[serde(default)]
    kernel: KernelConfig,
}

#[derive(Debug, Default, Deserialize)]
struct StorageConfig {
    data_dir: Option<PathBuf>,
    database: Option<PathBuf>,
    patch_dir: Option<PathBuf>,
    raw_mail_dir: Option<PathBuf>,
}

#[derive(Debug, Default, Deserialize)]
struct B4Config {
    path: Option<PathBuf>,
}

#[derive(Debug, Default, Deserialize)]
struct LoggingConfig {
    filter: Option<String>,
    dir: Option<PathBuf>,
}

#[derive(Debug, Default, Deserialize)]
struct SourceConfig {
    mailbox: Option<String>,
    lore_base_url: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct ImapCompatConfig {
    mailbox: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct KernelConfig {
    tree: Option<PathBuf>,
    #[serde(default)]
    trees: Vec<PathBuf>,
}

pub fn load(config_override: Option<&Path>) -> Result<RuntimeConfig> {
    let home_dir = UserDirs::new()
        .map(|dirs| dirs.home_dir().to_path_buf())
        .ok_or_else(|| {
            CourierError::new(
                ErrorCode::ConfigRead,
                "failed to determine HOME directory for courier runtime",
            )
        })?;

    let default_root = home_dir.join(".courier");

    let config_path = config_override
        .map(Path::to_path_buf)
        .unwrap_or_else(|| default_root.join("config.toml"));

    let file_config = if config_path.exists() {
        parse_config_file(&config_path)?
    } else {
        FileConfig::default()
    };

    let config_base_dir = config_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| default_root.clone());

    let data_dir = file_config
        .storage
        .data_dir
        .map(|path| resolve_path(&config_base_dir, path))
        .unwrap_or_else(|| default_root.clone());

    let database_path = file_config
        .storage
        .database
        .map(|path| resolve_path(&config_base_dir, path))
        .unwrap_or_else(|| data_dir.join("db/courier.db"));

    let raw_mail_dir = file_config
        .storage
        .raw_mail_dir
        .map(|path| resolve_path(&config_base_dir, path))
        .unwrap_or_else(|| data_dir.join("mail/raw"));

    let patch_dir = file_config
        .storage
        .patch_dir
        .map(|path| resolve_path(&config_base_dir, path))
        .unwrap_or_else(|| data_dir.join("patches"));

    let log_dir = file_config
        .logging
        .dir
        .map(|path| resolve_path(&config_base_dir, path))
        .unwrap_or_else(|| data_dir.join("logs"));

    let b4_path = file_config
        .b4
        .path
        .map(|path| resolve_path(&config_base_dir, path));

    let log_filter = file_config
        .logging
        .filter
        .unwrap_or_else(|| "info".to_string());

    let imap_mailbox = file_config
        .source
        .mailbox
        .or(file_config.imap.mailbox)
        .unwrap_or_else(|| DEFAULT_MAILBOX.to_string());

    let lore_base_url = file_config
        .source
        .lore_base_url
        .unwrap_or_else(|| DEFAULT_LORE_BASE_URL.to_string());

    let mut kernel_trees = Vec::new();
    if let Some(tree) = file_config.kernel.tree {
        kernel_trees.push(resolve_path(&config_base_dir, tree));
    }
    for tree in file_config.kernel.trees {
        let resolved = resolve_path(&config_base_dir, tree);
        if !kernel_trees.iter().any(|existing| existing == &resolved) {
            kernel_trees.push(resolved);
        }
    }

    Ok(RuntimeConfig {
        config_path,
        data_dir,
        database_path,
        raw_mail_dir,
        patch_dir,
        log_dir,
        b4_path,
        log_filter,
        imap_mailbox,
        lore_base_url,
        kernel_trees,
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
data_dir = "./state"
database = "./state/db/courier.sqlite"
patch_dir = "./state/patches"
raw_mail_dir = "./state/raw"

[b4]
path = "./bin/b4"

[logging]
filter = "debug"
dir = "./state/logs"

[source]
mailbox = "linux-kernel"
lore_base_url = "https://lore.kernel.org"

[kernel]
tree = "./linux"
trees = ["./linux-next"]
"#,
        )
        .expect("write config");

        let loaded = load(Some(&config_path)).expect("load config");

        assert_eq!(loaded.data_dir, base.join("./state"));
        assert_eq!(loaded.database_path, base.join("./state/db/courier.sqlite"));
        assert_eq!(loaded.patch_dir, base.join("./state/patches"));
        assert_eq!(loaded.raw_mail_dir, base.join("./state/raw"));
        assert_eq!(loaded.log_dir, base.join("./state/logs"));
        assert_eq!(loaded.b4_path, Some(base.join("./bin/b4")));
        assert_eq!(loaded.log_filter, "debug");
        assert_eq!(loaded.imap_mailbox, "linux-kernel");
        assert_eq!(loaded.lore_base_url, "https://lore.kernel.org");
        assert_eq!(
            loaded.kernel_trees,
            vec![base.join("./linux"), base.join("./linux-next")]
        );

        let _ = fs::remove_dir_all(base);
    }
}
