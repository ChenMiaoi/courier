//! Config-editor interactions rendered inside the TUI.
//!
//! Editing config in-process lets CRIEW validate and apply changes against
//! the same runtime normalization rules used at startup, instead of teaching
//! the UI a second config model.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use toml::Value as TomlValue;
use toml::value::Table as TomlTable;

use super::palette::PaletteCompletionContext;
use super::render::{centered_rect, truncate_with_ellipsis};
use super::*;

struct ConfigFileUpdate {
    rendered_value: String,
    runtime: RuntimeConfig,
}

impl AppState {
    pub(super) fn open_config_editor(&mut self, key_hint: Option<&str>) {
        self.palette.open = false;
        self.palette.input.clear();
        self.palette.clear_completion();
        self.palette.clear_local_result();
        self.config_editor.open = true;
        self.config_editor.mode = ConfigEditorMode::Browse;
        self.config_editor.input.clear();
        if let Some(key) = key_hint {
            if let Some(index) = config_editor_field_index(key) {
                self.config_editor.selected_field = index;
            } else {
                self.status = format!(
                    "config editor does not support {key}; opened {}",
                    self.selected_config_editor_field().key
                );
                return;
            }
        }
        self.status = format!(
            "config editor opened: {}",
            self.selected_config_editor_field().key
        );
    }

    pub(super) fn close_config_editor(&mut self) {
        self.config_editor.open = false;
        self.config_editor.mode = ConfigEditorMode::Browse;
        self.config_editor.input.clear();
        self.status = "config editor closed".to_string();
    }

    pub(super) fn selected_config_editor_field(&self) -> &'static ConfigEditorField {
        let index = self
            .config_editor
            .selected_field
            .min(CONFIG_EDITOR_FIELDS.len().saturating_sub(1));
        &CONFIG_EDITOR_FIELDS[index]
    }

    pub(super) fn move_config_editor_up(&mut self) {
        if self.config_editor.selected_field > 0 {
            self.config_editor.selected_field -= 1;
        }
    }

    pub(super) fn move_config_editor_down(&mut self) {
        if self.config_editor.selected_field + 1 < CONFIG_EDITOR_FIELDS.len() {
            self.config_editor.selected_field += 1;
        }
    }

    fn config_editor_seed_input(&self, key: &str) -> String {
        match read_config_key_from_file(&self.runtime.config_path, key) {
            Ok(Some(value)) => render_toml_value(&value),
            Ok(None) => config_value_suggestions(self, Some(&key.to_string()))
                .into_iter()
                .next()
                .map(|suggestion| suggestion.value)
                .unwrap_or_default(),
            Err(_) => String::new(),
        }
    }

    pub(super) fn start_config_editor_edit(&mut self) {
        let key = self.selected_config_editor_field().key;
        self.config_editor.mode = ConfigEditorMode::Edit;
        self.config_editor.input = self.config_editor_seed_input(key);
        self.status = format!("editing config {key}");
    }

    pub(super) fn cycle_config_editor_value(&mut self) {
        let key = self.selected_config_editor_field().key;
        let suggestions = config_value_suggestions(self, Some(&key.to_string()));
        if suggestions.is_empty() {
            self.status = format!("no preset values for {key}");
            return;
        }

        let current = if matches!(self.config_editor.mode, ConfigEditorMode::Edit) {
            self.config_editor.input.trim().to_string()
        } else {
            self.config_editor_seed_input(key)
        };

        let next_index = suggestions
            .iter()
            .position(|suggestion| suggestion.value == current)
            .map(|index| (index + 1) % suggestions.len())
            .unwrap_or(0);
        let next_value = suggestions[next_index].value.clone();

        if matches!(self.config_editor.mode, ConfigEditorMode::Edit) {
            self.config_editor.input = next_value;
            self.status = format!("preset value selected for {key}");
            return;
        }

        tracing::info!(
            op = "config.set",
            status = "started",
            key = %key,
            value_literal = %next_value,
            source = "config_editor_cycle"
        );
        match update_config_key_in_file(&self.runtime.config_path, key, &next_value) {
            Ok(update) => {
                apply_runtime_update(self, update.runtime);
                self.status = format!("config updated: {key} = {}", update.rendered_value);
                tracing::info!(
                    op = "config.set",
                    status = "succeeded",
                    key = %key,
                    value = %update.rendered_value,
                    config = %self.runtime.config_path.display(),
                    source = "config_editor_cycle"
                );
            }
            Err(error) => {
                tracing::error!(
                    op = "config.set",
                    status = "failed",
                    key = %key,
                    error = %error,
                    source = "config_editor_cycle"
                );
                self.status = format!("failed to set config key {key}: {error}");
            }
        }
    }

    pub(super) fn save_config_editor_value(&mut self) {
        let key = self.selected_config_editor_field().key;
        let value_literal = self.config_editor.input.trim().to_string();
        if value_literal.is_empty() {
            self.status = format!("empty value for {key}; press x to unset instead");
            return;
        }

        tracing::info!(
            op = "config.set",
            status = "started",
            key = %key,
            value_literal = %value_literal,
            source = "config_editor"
        );
        match update_config_key_in_file(&self.runtime.config_path, key, &value_literal) {
            Ok(update) => {
                apply_runtime_update(self, update.runtime);
                self.config_editor.mode = ConfigEditorMode::Browse;
                self.config_editor.input.clear();
                self.status = format!("config updated: {key} = {}", update.rendered_value);
                tracing::info!(
                    op = "config.set",
                    status = "succeeded",
                    key = %key,
                    value = %update.rendered_value,
                    config = %self.runtime.config_path.display(),
                    source = "config_editor"
                );
            }
            Err(error) => {
                tracing::error!(
                    op = "config.set",
                    status = "failed",
                    key = %key,
                    error = %error,
                    source = "config_editor"
                );
                self.status = format!("failed to set config key {key}: {error}");
            }
        }
    }

    pub(super) fn unset_selected_config_key(&mut self) {
        let key = self.selected_config_editor_field().key;
        tracing::info!(op = "config.unset", status = "started", key = %key);
        match remove_config_key_from_file(&self.runtime.config_path, key) {
            Ok(Some(runtime)) => {
                apply_runtime_update(self, runtime);
                self.config_editor.mode = ConfigEditorMode::Browse;
                self.config_editor.input.clear();
                self.status = format!("config key unset: {key}");
                tracing::info!(
                    op = "config.unset",
                    status = "succeeded",
                    key = %key,
                    config = %self.runtime.config_path.display()
                );
            }
            Ok(None) => {
                self.status = format!("config key already unset: {key}");
            }
            Err(error) => {
                tracing::error!(op = "config.unset", status = "failed", key = %key, error = %error);
                self.status = format!("failed to unset config key {key}: {error}");
            }
        }
    }
}

pub(super) fn run_palette_config(state: &mut AppState, command: &str) {
    tracing::debug!(command = %command, "user executed config command from palette");
    let mut segments = command.split_whitespace();
    let _ = segments.next();
    let action = segments.next().unwrap_or("show").to_ascii_lowercase();

    match action.as_str() {
        "show" => {
            if let Some(key) = segments.next() {
                show_config_key(state, key);
            } else {
                state.open_config_editor(None);
            }
        }
        "edit" => {
            state.open_config_editor(segments.next());
        }
        "get" => {
            let Some(key) = segments.next() else {
                state.status = "usage: config get <key>".to_string();
                return;
            };
            show_config_key(state, key);
        }
        "set" => {
            let Some(key) = segments.next() else {
                state.status = "usage: config set <key> <value>".to_string();
                return;
            };
            let value_literal = segments.collect::<Vec<_>>().join(" ");
            if value_literal.trim().is_empty() {
                state.status = "usage: config set <key> <value>".to_string();
                return;
            }

            tracing::info!(
                op = "config.set",
                status = "started",
                key = %key,
                value_literal = %value_literal
            );
            match update_config_key_in_file(&state.runtime.config_path, key, &value_literal) {
                Ok(update) => {
                    apply_runtime_update(state, update.runtime);
                    state.status = format!("config updated: {key} = {}", update.rendered_value);
                    tracing::info!(
                        op = "config.set",
                        status = "succeeded",
                        key = %key,
                        value = %update.rendered_value,
                        config = %state.runtime.config_path.display()
                    );
                }
                Err(error) => {
                    tracing::error!(
                        op = "config.set",
                        status = "failed",
                        key = %key,
                        error = %error
                    );
                    state.status = format!("failed to set config key {key}: {error}");
                }
            }
        }
        "help" => {
            state.status =
                "config usage: show [key] | edit [key] | get <key> | set <key> <value>".to_string();
        }
        _ => {
            state.status =
                "config usage: show [key] | edit [key] | get <key> | set <key> <value>".to_string();
        }
    }
}

fn show_config_key(state: &mut AppState, key: &str) {
    if key.trim().is_empty() {
        state.status = "usage: config get <key>".to_string();
        return;
    }

    let file_value = read_config_key_from_file(&state.runtime.config_path, key);
    match file_value {
        Ok(Some(value)) => {
            state.status = format!("config file {key} = {}", render_toml_value(&value));
        }
        Ok(None) => {
            if let Some(value) = effective_config_value(state, key) {
                state.status = format!("config effective {key} = {value} (default/runtime)");
            } else {
                state.status = format!("config key not found: {key}");
            }
        }
        Err(error) => {
            tracing::error!(key = %key, error = %error, "failed to read config key");
            state.status = format!("failed to read config key {key}: {error}");
        }
    }
}

fn read_config_key_from_file(
    config_path: &Path,
    key: &str,
) -> std::result::Result<Option<TomlValue>, String> {
    let table = read_config_table(config_path)?;
    Ok(lookup_config_key(&table, key).cloned())
}

fn update_config_key_in_file(
    config_path: &Path,
    key: &str,
    value_literal: &str,
) -> std::result::Result<ConfigFileUpdate, String> {
    let mut table = read_config_table(config_path)?;
    let value = parse_toml_value_literal(value_literal);
    set_config_key(&mut table, key, value)?;

    let rendered = lookup_config_key(&table, key)
        .map(render_toml_value)
        .unwrap_or_else(|| "<unknown>".to_string());
    let content = render_config_table(&table)?;
    // Validate against the real runtime loader before touching disk so the TUI
    // never writes a config it cannot immediately reload.
    let runtime = validate_updated_config(config_path, &content)?;
    write_config_content(config_path, &content)?;

    Ok(ConfigFileUpdate {
        rendered_value: rendered,
        runtime,
    })
}

fn remove_config_key_from_file(
    config_path: &Path,
    key: &str,
) -> std::result::Result<Option<RuntimeConfig>, String> {
    let mut table = read_config_table(config_path)?;
    let removed = remove_config_key(&mut table, key)?;
    if !removed {
        return Ok(None);
    }

    let content = render_config_table(&table)?;
    let runtime = validate_updated_config(config_path, &content)?;
    write_config_content(config_path, &content)?;

    Ok(Some(runtime))
}

fn read_config_table(config_path: &Path) -> std::result::Result<TomlTable, String> {
    let content = match fs::read_to_string(config_path) {
        Ok(content) => content,
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(format!("failed to read {}: {error}", config_path.display()));
        }
    };

    if content.trim().is_empty() {
        return Ok(TomlTable::new());
    }

    toml::from_str::<TomlTable>(&content)
        .map_err(|error| format!("failed to parse TOML in {}: {error}", config_path.display()))
}

fn render_config_table(table: &TomlTable) -> std::result::Result<String, String> {
    let mut content = toml::to_string_pretty(table)
        .map_err(|error| format!("failed to serialize config table: {error}"))?;
    if !content.ends_with('\n') {
        content.push('\n');
    }

    Ok(content)
}

fn write_config_content(config_path: &Path, content: &str) -> std::result::Result<(), String> {
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "failed to create config directory {}: {error}",
                parent.display()
            )
        })?;
    }

    fs::write(config_path, content).map_err(|error| {
        format!(
            "failed to write config file {}: {error}",
            config_path.display()
        )
    })
}

// Validate in memory before writing so the TUI never reports a failed reload
// after already leaving the on-disk config in a broken state.
fn validate_updated_config(
    config_path: &Path,
    content: &str,
) -> std::result::Result<RuntimeConfig, String> {
    crate::infra::config::load_from_document(config_path, content)
        .map_err(|error| error.to_string())
}

fn lookup_config_key<'a>(table: &'a TomlTable, key: &str) -> Option<&'a TomlValue> {
    let mut key_parts = key.split('.').filter(|part| !part.is_empty());
    let first = key_parts.next()?;
    let mut current = table.get(first)?;
    for segment in key_parts {
        current = current.as_table()?.get(segment)?;
    }
    Some(current)
}

fn set_config_key(
    table: &mut TomlTable,
    key: &str,
    value: TomlValue,
) -> std::result::Result<(), String> {
    let mut key_parts: Vec<&str> = key.split('.').filter(|part| !part.is_empty()).collect();
    if key_parts.is_empty() {
        return Err("empty key".to_string());
    }

    let Some(leaf) = key_parts.pop() else {
        return Err("empty key".to_string());
    };
    let mut current = table;
    for segment in key_parts {
        // Materialize missing intermediate tables on demand so editing one leaf
        // does not require the user to hand-create every parent section first.
        let node = current
            .entry(segment.to_string())
            .or_insert_with(|| TomlValue::Table(TomlTable::new()));
        if !node.is_table() {
            return Err(format!(
                "cannot set {key}: key segment {segment} already holds a value"
            ));
        }
        current = node
            .as_table_mut()
            .ok_or_else(|| format!("cannot set {key}: key segment {segment} is not a table"))?;
    }
    current.insert(leaf.to_string(), value);
    Ok(())
}

fn remove_config_key(table: &mut TomlTable, key: &str) -> std::result::Result<bool, String> {
    let key_parts: Vec<&str> = key.split('.').filter(|part| !part.is_empty()).collect();
    if key_parts.is_empty() {
        return Err("empty key".to_string());
    }

    Ok(remove_config_key_segments(table, &key_parts))
}

fn remove_config_key_segments(table: &mut TomlTable, segments: &[&str]) -> bool {
    if segments.len() == 1 {
        return table.remove(segments[0]).is_some();
    }

    let key = segments[0];
    let (removed, prune_child) = if let Some(child) = table.get_mut(key) {
        if let Some(child_table) = child.as_table_mut() {
            let removed = remove_config_key_segments(child_table, &segments[1..]);
            (removed, removed && child_table.is_empty())
        } else {
            (false, false)
        }
    } else {
        (false, false)
    };

    if prune_child {
        // Prune now-empty parent tables so repeated edits do not leave behind
        // confusing hollow sections in the generated config file.
        table.remove(key);
    }

    removed
}

fn parse_toml_value_literal(value_literal: &str) -> TomlValue {
    let literal = value_literal.trim();
    if literal.is_empty() {
        return TomlValue::String(String::new());
    }

    // Parse through a synthetic TOML snippet first so numbers, booleans, and
    // arrays preserve their real types instead of being forced into strings.
    let snippet = format!("value = {literal}");
    if let Ok(parsed) = toml::from_str::<TomlTable>(&snippet)
        && let Some(value) = parsed.get("value")
    {
        return value.clone();
    }

    TomlValue::String(literal.to_string())
}

fn render_toml_value(value: &TomlValue) -> String {
    value.to_string()
}

fn config_editor_field_index(key: &str) -> Option<usize> {
    CONFIG_EDITOR_FIELDS
        .iter()
        .position(|field| field.key.eq_ignore_ascii_case(key))
}

fn apply_runtime_update(state: &mut AppState, runtime: RuntimeConfig) {
    let old_inbox_auto_sync_interval_secs = state.runtime.inbox_auto_sync_interval_secs;
    let selected_path_hint = state.selected_kernel_tree_path();
    let enabled_mailboxes: HashSet<String> = state.enabled_mailboxes().into_iter().collect();
    let active_mailbox = state.active_thread_mailbox.clone();
    // Preserve current UI intent across config reloads so editing one setting
    // does not unexpectedly wipe mailbox enablement or the active tree focus.
    state.runtime = runtime;
    state.ui_state_path = ui_state::path_for_data_dir(&state.runtime.data_dir);
    state.subscriptions = default_subscriptions(
        &state.runtime,
        &enabled_mailboxes,
        Some(active_mailbox.as_str()),
        if state.runtime.imap.is_complete() && !state.imap_defaults_initialized {
            MyInboxDefault::EnableOnFirstOpen
        } else {
            MyInboxDefault::PreservePersistedChoice
        },
    );
    if state.runtime.imap.is_complete() {
        state.imap_defaults_initialized = true;
    }
    if let Some(index) = state
        .subscriptions
        .iter()
        .position(|item| same_mailbox_name(&item.mailbox, &state.active_thread_mailbox))
    {
        state.subscription_index = index;
        state.sync_subscription_row_to_selected_item();
    }
    state.reconcile_inbox_auto_sync();
    state.reconcile_subscription_auto_sync();
    if state.runtime.inbox_auto_sync_interval_secs != old_inbox_auto_sync_interval_secs {
        state.defer_inbox_auto_sync();
        state.defer_subscription_auto_sync();
    }
    state.refresh_kernel_tree_rows(selected_path_hint.as_deref());
    if matches!(state.ui_page, UiPage::CodeBrowser) && !state.supports_code_browser() {
        state.ui_page = UiPage::Mail;
        state.code_focus = CodePaneFocus::Tree;
    }
}

fn effective_config_value(state: &AppState, key: &str) -> Option<String> {
    match key {
        "config.path" => Some(state.runtime.config_path.display().to_string()),
        "storage.data_dir" => Some(state.runtime.data_dir.display().to_string()),
        "storage.database" => Some(state.runtime.database_path.display().to_string()),
        "storage.raw_mail_dir" => Some(state.runtime.raw_mail_dir.display().to_string()),
        "storage.patch_dir" => Some(state.runtime.patch_dir.display().to_string()),
        "logging.dir" => Some(state.runtime.log_dir.display().to_string()),
        "logging.filter" => Some(state.runtime.log_filter.clone()),
        "b4.path" => Some(
            state
                .runtime
                .b4_path
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "<none>".to_string()),
        ),
        "source.mailbox" | "imap.mailbox" => Some(state.runtime.source_mailbox.clone()),
        "imap.email" => state.runtime.imap.email.clone(),
        "imap.user" => state.runtime.imap.user.clone(),
        "imap.pass" => state.runtime.imap.pass.clone(),
        "imap.server" => state.runtime.imap.server.clone(),
        "imap.serverport" => state
            .runtime
            .imap
            .server_port
            .map(|value| value.to_string()),
        "imap.encryption" => state
            .runtime
            .imap
            .encryption
            .map(|value| value.as_str().to_string()),
        "imap.proxy" => state.runtime.imap.proxy.clone(),
        "source.lore_base_url" => Some(state.runtime.lore_base_url.clone()),
        "ui.startup_sync" => Some(state.runtime.startup_sync.to_string()),
        "ui.keymap" => Some(state.runtime.ui_keymap.as_str().to_string()),
        "ui.inbox_auto_sync_interval_secs" => {
            Some(state.runtime.inbox_auto_sync_interval_secs.to_string())
        }
        "kernel.trees" => Some(format!(
            "[{}]",
            state
                .runtime
                .kernel_trees
                .iter()
                .map(|path| format!("\"{}\"", path.display()))
                .collect::<Vec<_>>()
                .join(", ")
        )),
        "kernel.tree" => state
            .runtime
            .kernel_trees
            .first()
            .map(|path| path.display().to_string()),
        _ => None,
    }
}

pub(super) fn config_completion_suggestions(
    state: &AppState,
    context: &PaletteCompletionContext,
) -> Vec<PaletteSuggestion> {
    let action = context
        .tokens
        .get(1)
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_default();

    match context.active_index {
        1 => vec![
            PaletteSuggestion {
                value: "edit".to_string(),
                description: Some("Open visual config editor".to_string()),
            },
            PaletteSuggestion {
                value: "get".to_string(),
                description: Some("Read one config key".to_string()),
            },
            PaletteSuggestion {
                value: "help".to_string(),
                description: Some("Show config command usage".to_string()),
            },
            PaletteSuggestion {
                value: "set".to_string(),
                description: Some("Write one config key".to_string()),
            },
            PaletteSuggestion {
                value: "show".to_string(),
                description: Some("Show config file path or one key".to_string()),
            },
        ],
        2 => {
            let owned_editor_keys: Vec<&str>;
            // Suggest only keys relevant to the chosen action so palette
            // completion nudges users toward supported edits instead of every
            // config field the runtime happens to know about.
            let keys = if action == "set" {
                CONFIG_SET_KEYS
            } else if action == "edit" {
                owned_editor_keys = CONFIG_EDITOR_FIELDS.iter().map(|field| field.key).collect();
                &owned_editor_keys
            } else {
                CONFIG_GET_KEYS
            };
            keys.iter()
                .map(|key| PaletteSuggestion {
                    value: (*key).to_string(),
                    description: Some("Config key".to_string()),
                })
                .collect()
        }
        3 if action == "set" => config_value_suggestions(state, context.tokens.get(2)),
        _ => Vec::new(),
    }
}

fn config_value_suggestions(state: &AppState, key: Option<&String>) -> Vec<PaletteSuggestion> {
    let Some(key) = key.map(String::as_str) else {
        return Vec::new();
    };

    match key {
        "logging.filter" => ["trace", "debug", "info", "warn", "error"]
            .iter()
            .map(|value| PaletteSuggestion {
                value: (*value).to_string(),
                description: Some("Log filter".to_string()),
            })
            .collect(),
        "source.mailbox" | "imap.mailbox" => state
            .subscriptions
            .iter()
            .map(|subscription| PaletteSuggestion {
                value: subscription.mailbox.clone(),
                description: Some("Mailbox".to_string()),
            })
            .collect(),
        "imap.email" => vec![PaletteSuggestion {
            value: state
                .runtime
                .imap
                .email
                .as_ref()
                .map(|value| format!("\"{value}\""))
                .unwrap_or_else(|| "\"you@example.com\"".to_string()),
            description: Some("Self email; also default IMAP login".to_string()),
        }],
        "imap.user" => vec![PaletteSuggestion {
            value: state
                .runtime
                .imap
                .user
                .as_ref()
                .map(|value| format!("\"{value}\""))
                .or_else(|| {
                    state
                        .runtime
                        .imap
                        .email
                        .as_ref()
                        .map(|value| format!("\"{value}\""))
                })
                .unwrap_or_else(|| "\"you@example.com\"".to_string()),
            description: Some("IMAP login account; Gmail usually needs full email".to_string()),
        }],
        "imap.pass" => vec![PaletteSuggestion {
            value: "\"imap-pass\"".to_string(),
            description: Some("IMAP login password".to_string()),
        }],
        "imap.server" => vec![PaletteSuggestion {
            value: state
                .runtime
                .imap
                .server
                .as_ref()
                .map(|value| format!("\"{value}\""))
                .unwrap_or_else(|| "\"imap.example.com\"".to_string()),
            description: Some("IMAP server host".to_string()),
        }],
        "imap.serverport" => vec![PaletteSuggestion {
            value: state
                .runtime
                .imap
                .server_port
                .map(|value| value.to_string())
                .unwrap_or_else(|| "993".to_string()),
            description: Some("IMAP server port".to_string()),
        }],
        "imap.encryption" => ["tls", "ssl", "starttls", "none"]
            .iter()
            .map(|value| PaletteSuggestion {
                value: format!("\"{value}\""),
                description: Some("IMAP encryption".to_string()),
            })
            .collect(),
        "imap.proxy" => [
            "socks5://127.0.0.1:7890",
            "socks5://10.0.2.2:7890",
            "http://127.0.0.1:7890",
        ]
        .iter()
        .map(|value| PaletteSuggestion {
            value: format!("\"{value}\""),
            description: Some("IMAP proxy URL".to_string()),
        })
        .collect(),
        "source.lore_base_url" => vec![PaletteSuggestion {
            value: "https://lore.kernel.org".to_string(),
            description: Some("Lore base URL".to_string()),
        }],
        "kernel.trees" => vec![PaletteSuggestion {
            value: "[\"/path/to/linux\"]".to_string(),
            description: Some("TOML array".to_string()),
        }],
        "kernel.tree" => state
            .runtime
            .kernel_trees
            .first()
            .map(|path| {
                vec![PaletteSuggestion {
                    value: format!("\"{}\"", path.display()),
                    description: Some("Current kernel tree".to_string()),
                }]
            })
            .unwrap_or_default(),
        "storage.data_dir" => vec![PaletteSuggestion {
            value: format!("\"{}\"", state.runtime.data_dir.display()),
            description: Some("Current data dir".to_string()),
        }],
        "storage.database" => vec![PaletteSuggestion {
            value: format!("\"{}\"", state.runtime.database_path.display()),
            description: Some("Current database path".to_string()),
        }],
        "storage.raw_mail_dir" => vec![PaletteSuggestion {
            value: format!("\"{}\"", state.runtime.raw_mail_dir.display()),
            description: Some("Current raw mail dir".to_string()),
        }],
        "storage.patch_dir" => vec![PaletteSuggestion {
            value: format!("\"{}\"", state.runtime.patch_dir.display()),
            description: Some("Current patch dir".to_string()),
        }],
        "logging.dir" => vec![PaletteSuggestion {
            value: format!("\"{}\"", state.runtime.log_dir.display()),
            description: Some("Current log dir".to_string()),
        }],
        "ui.startup_sync" => ["true", "false"]
            .iter()
            .map(|value| PaletteSuggestion {
                value: (*value).to_string(),
                description: Some("Auto-sync on TUI startup".to_string()),
            })
            .collect(),
        "ui.keymap" => [
            ("default", "j/l focus, i/k move"),
            ("vim", "h/l focus, j/k move"),
            ("custom", "custom label with default navigation fallback"),
        ]
        .iter()
        .map(|(value, description)| PaletteSuggestion {
            value: (*value).to_string(),
            description: Some((*description).to_string()),
        })
        .collect(),
        "ui.inbox_auto_sync_interval_secs" => ["15", "30", "60", "300"]
            .iter()
            .map(|value| PaletteSuggestion {
                value: (*value).to_string(),
                description: Some("Seconds between My Inbox background sync runs".to_string()),
            })
            .collect(),
        "b4.path" => vec![PaletteSuggestion {
            value: "\"/usr/bin/b4\"".to_string(),
            description: Some("Path to b4 executable".to_string()),
        }],
        _ => Vec::new(),
    }
}

pub(super) fn draw_config_editor(frame: &mut Frame<'_>, state: &AppState) {
    let area = centered_rect(88, 76, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title("Runtime Config")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::LightGreen));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(10),
            Constraint::Length(
                if matches!(state.config_editor.mode, ConfigEditorMode::Edit) {
                    3
                } else {
                    2
                },
            ),
        ])
        .split(inner);

    let header = format!(
        "file: {} | :config opens this editor | changes are written back immediately",
        state.runtime.config_path.display()
    );
    frame.render_widget(Paragraph::new(header), sections[0]);

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(46), Constraint::Percentage(54)])
        .split(sections[1]);

    let file_table = read_config_table(&state.runtime.config_path).ok();
    let selected_index = state
        .config_editor
        .selected_field
        .min(CONFIG_EDITOR_FIELDS.len().saturating_sub(1));

    let mut list_state = ListState::default();
    list_state.select(Some(selected_index));

    let list_width = body[0].width.saturating_sub(4) as usize;
    let items: Vec<ListItem> = CONFIG_EDITOR_FIELDS
        .iter()
        .map(|field| {
            let file_value = file_table
                .as_ref()
                .and_then(|table| lookup_config_key(table, field.key))
                .map(render_toml_value);
            let effective_value = effective_config_value(state, field.key);
            let (value, source) = if let Some(value) = file_value {
                (value, "file")
            } else if let Some(value) = effective_value {
                (value, "default")
            } else {
                ("<unset>".to_string(), "unset")
            };
            let line = format!("{} = {} [{}]", field.key, value, source);
            ListItem::new(truncate_with_ellipsis(&line, list_width))
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .title("Fields")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Gray)),
        )
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );
    frame.render_stateful_widget(list, body[0], &mut list_state);

    let field = state.selected_config_editor_field();
    let file_value = file_table
        .as_ref()
        .and_then(|table| lookup_config_key(table, field.key))
        .map(render_toml_value)
        .unwrap_or_else(|| "<unset>".to_string());
    let effective_value =
        effective_config_value(state, field.key).unwrap_or_else(|| "<unset>".to_string());
    let suggestions = config_value_suggestions(state, Some(&field.key.to_string()));
    let mut details = vec![
        format!("Key: {}", field.key),
        format!("About: {}", field.description),
        String::new(),
        format!("File value: {}", file_value),
        format!("Effective: {}", effective_value),
        // Show both the literal file value and the effective runtime value so
        // users can see when defaults or derived behavior are masking an unset
        // field in the config file itself.
        format!(
            "Source: {}",
            if file_value == "<unset>" {
                "runtime default / derived fallback"
            } else {
                "explicit value from config file"
            }
        ),
    ];
    if !suggestions.is_empty() {
        details.push(String::new());
        details.push("Presets:".to_string());
        for suggestion in suggestions.iter().take(6) {
            match suggestion.description.as_deref() {
                Some(description) if !description.is_empty() => {
                    details.push(format!("  {}  ({description})", suggestion.value));
                }
                _ => details.push(format!("  {}", suggestion.value)),
            }
        }
    }
    details.push(String::new());
    details.push("Unset removes the explicit key and falls back to runtime defaults.".to_string());

    let detail = Paragraph::new(details.join("\n"))
        .block(
            Block::default()
                .title("Selected Field")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Gray)),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(detail, body[1]);

    if matches!(state.config_editor.mode, ConfigEditorMode::Edit) {
        let footer = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(sections[2]);
        frame.render_widget(
            Paragraph::new(format!("Editing {} as TOML literal", field.key)),
            footer[0],
        );
        let prompt = format!("> {}", state.config_editor.input);
        frame.render_widget(Paragraph::new(prompt), footer[1]);
        frame.render_widget(
            Paragraph::new(
                "Enter save | Esc cancel | Tab cycle presets | strings may be bare or quoted",
            ),
            footer[2],
        );
        if footer[1].width > 0 {
            let cursor_col =
                (2 + state.config_editor.input.chars().count()).min(footer[1].width as usize - 1);
            frame.set_cursor_position((footer[1].x + cursor_col as u16, footer[1].y));
        }
    } else {
        let footer = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Length(1)])
            .split(sections[2]);
        frame.render_widget(
            Paragraph::new("i/k or ↑/↓ move | Enter/e edit | Tab presets | x unset | Esc close"),
            footer[0],
        );
        frame.render_widget(
            Paragraph::new(
                "Use TOML literals for arrays/paths when needed, e.g. [\"/path/to/linux\"].",
            ),
            footer[1],
        );
    }
}
