use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::infra::error::{CourierError, ErrorCode, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiState {
    #[serde(default)]
    pub enabled_mailboxes: Vec<String>,
    #[serde(default = "default_true")]
    pub enabled_group_expanded: bool,
    #[serde(default = "default_true")]
    pub disabled_group_expanded: bool,
    #[serde(default)]
    pub active_mailbox: Option<String>,
}

fn default_true() -> bool {
    true
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            enabled_mailboxes: Vec::new(),
            enabled_group_expanded: true,
            disabled_group_expanded: true,
            active_mailbox: None,
        }
    }
}

impl UiState {
    pub fn normalized_enabled_mailboxes(&self) -> Vec<String> {
        let mut seen = HashSet::new();
        let mut mailboxes = Vec::new();
        for mailbox in &self.enabled_mailboxes {
            let normalized = mailbox.trim();
            if normalized.is_empty() {
                continue;
            }
            if seen.insert(normalized.to_string()) {
                mailboxes.push(normalized.to_string());
            }
        }
        mailboxes.sort();
        mailboxes
    }
}

pub fn path_for_data_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("ui-state.toml")
}

pub fn load(path: &Path) -> Result<Option<UiState>> {
    if !path.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(path).map_err(|error| {
        CourierError::with_source(
            ErrorCode::Io,
            format!("failed to read ui state {}", path.display()),
            error,
        )
    })?;

    let state = toml::from_str::<UiState>(&content).map_err(|error| {
        CourierError::with_source(
            ErrorCode::ConfigParse,
            format!("failed to parse ui state {}", path.display()),
            error,
        )
    })?;

    Ok(Some(state))
}

pub fn save(path: &Path, state: &UiState) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            CourierError::with_source(
                ErrorCode::Io,
                format!("failed to create ui state directory {}", parent.display()),
                error,
            )
        })?;
    }

    let content = toml::to_string_pretty(state).map_err(|error| {
        CourierError::with_source(
            ErrorCode::ConfigParse,
            format!("failed to serialize ui state {}", path.display()),
            error,
        )
    })?;

    fs::write(path, content).map_err(|error| {
        CourierError::with_source(
            ErrorCode::Io,
            format!("failed to write ui state {}", path.display()),
            error,
        )
    })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{UiState, load, path_for_data_dir, save};

    fn temp_dir(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("courier-ui-state-{label}-{nonce}"));
        fs::create_dir_all(&path).expect("create temp dir");
        path
    }

    #[test]
    fn roundtrip_ui_state_file() {
        let root = temp_dir("roundtrip");
        let path = path_for_data_dir(&root);
        let state = UiState {
            enabled_mailboxes: vec!["bpf".to_string(), "linux-mm".to_string(), "bpf".to_string()],
            enabled_group_expanded: false,
            disabled_group_expanded: true,
            active_mailbox: Some("bpf".to_string()),
        };

        save(&path, &state).expect("save state");
        let loaded = load(&path).expect("load state").expect("state exists");

        assert_eq!(
            loaded.normalized_enabled_mailboxes(),
            vec!["bpf".to_string(), "linux-mm".to_string()]
        );
        assert!(!loaded.enabled_group_expanded);
        assert!(loaded.disabled_group_expanded);
        assert_eq!(loaded.active_mailbox.as_deref(), Some("bpf"));

        let _ = fs::remove_dir_all(root);
    }
}
