use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{self, Stdout};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::app::sync as sync_worker;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::{Frame, Terminal};
use toml::Value as TomlValue;
use toml::value::Table as TomlTable;

use crate::domain::subscriptions::VGER_SUBSCRIPTIONS;
use crate::infra::bootstrap::BootstrapState;
use crate::infra::config::RuntimeConfig;
use crate::infra::error::{CourierError, ErrorCode, Result};
use crate::infra::mail_store::{self, ThreadRow};
use crate::infra::ui_state::{self, UiState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pane {
    Subscriptions,
    Threads,
    Preview,
}

impl Pane {
    fn title(self) -> &'static str {
        match self {
            Self::Subscriptions => "Subscriptions",
            Self::Threads => "Threads",
            Self::Preview => "Preview",
        }
    }

    fn next(self) -> Self {
        match self {
            Self::Subscriptions => Self::Threads,
            Self::Threads => Self::Preview,
            Self::Preview => Self::Subscriptions,
        }
    }

    fn previous(self) -> Self {
        match self {
            Self::Subscriptions => Self::Preview,
            Self::Threads => Self::Subscriptions,
            Self::Preview => Self::Threads,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct PaletteCommand {
    name: &'static str,
    description: &'static str,
}

#[derive(Debug, Clone)]
struct PaletteSuggestion {
    value: String,
    description: Option<String>,
}

const PALETTE_COMMANDS: &[PaletteCommand] = &[
    PaletteCommand {
        name: "quit",
        description: "Exit Courier",
    },
    PaletteCommand {
        name: "exit",
        description: "Exit Courier",
    },
    PaletteCommand {
        name: "help",
        description: "Show available commands",
    },
    PaletteCommand {
        name: "sync",
        description: "Sync mailbox now",
    },
    PaletteCommand {
        name: "config",
        description: "Show or update runtime config",
    },
];

const PALETTE_SYNC_RECONNECT_ATTEMPTS: u8 = 3;
const PREVIEW_TAB_SPACES: &str = "    ";
const KERNEL_TREE_MAX_ROWS: usize = 2048;
const CODE_PREVIEW_MAX_BYTES: usize = 256 * 1024;
const CODE_PREVIEW_MAX_LINES: usize = 800;
const CONFIG_GET_KEYS: &[&str] = &[
    "config.path",
    "storage.data_dir",
    "storage.database",
    "storage.raw_mail_dir",
    "storage.patch_dir",
    "logging.dir",
    "logging.filter",
    "b4.path",
    "source.mailbox",
    "imap.mailbox",
    "source.lore_base_url",
    "kernel.tree",
    "kernel.trees",
];
const CONFIG_SET_KEYS: &[&str] = &[
    "storage.data_dir",
    "storage.database",
    "storage.raw_mail_dir",
    "storage.patch_dir",
    "logging.dir",
    "logging.filter",
    "b4.path",
    "source.mailbox",
    "imap.mailbox",
    "source.lore_base_url",
    "kernel.tree",
    "kernel.trees",
];

#[derive(Debug, Default)]
struct CommandPaletteState {
    open: bool,
    input: String,
    suggestions: Vec<PaletteSuggestion>,
    show_suggestions: bool,
    last_tab_input: String,
}

impl CommandPaletteState {
    fn clear_completion(&mut self) {
        self.suggestions.clear();
        self.show_suggestions = false;
        self.last_tab_input.clear();
    }
}

#[derive(Debug, Default)]
struct SearchState {
    active: bool,
    input: String,
    applied_query: String,
}

#[derive(Debug, Clone)]
struct SubscriptionItem {
    mailbox: String,
    description: String,
    enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SubscriptionRowKind {
    EnabledHeader,
    DisabledHeader,
    Item(usize),
}

#[derive(Debug, Clone)]
struct SubscriptionRow {
    kind: SubscriptionRowKind,
    text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UiPage {
    Mail,
    CodeBrowser,
}

impl UiPage {
    fn toggled(self) -> Self {
        match self {
            Self::Mail => Self::CodeBrowser,
            Self::CodeBrowser => Self::Mail,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CodePaneFocus {
    Tree,
    Source,
}

impl CodePaneFocus {
    fn next(self) -> Self {
        match self {
            Self::Tree => Self::Source,
            Self::Source => Self::Tree,
        }
    }

    fn previous(self) -> Self {
        match self {
            Self::Tree => Self::Source,
            Self::Source => Self::Tree,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KernelTreeRowKind {
    RootDirectory,
    Directory,
    File,
    RootFile,
    MissingPath,
}

#[derive(Debug, Clone)]
struct KernelTreeRow {
    path: PathBuf,
    name: String,
    depth: usize,
    kind: KernelTreeRowKind,
    expandable: bool,
    expanded: bool,
}

impl KernelTreeRow {
    fn is_file(&self) -> bool {
        matches!(
            self.kind,
            KernelTreeRowKind::File | KernelTreeRowKind::RootFile
        )
    }

    fn display_text(&self) -> String {
        match self.kind {
            KernelTreeRowKind::RootDirectory => {
                let marker = if self.expandable {
                    if self.expanded { "▼" } else { "▶" }
                } else {
                    "•"
                };
                format!("{marker} [root] {}", self.path.display())
            }
            KernelTreeRowKind::Directory => {
                let marker = if self.expandable {
                    if self.expanded { "▼" } else { "▶" }
                } else {
                    "•"
                };
                format!("{}{} {}/", "  ".repeat(self.depth), marker, self.name)
            }
            KernelTreeRowKind::File => {
                format!("{}  {}", "  ".repeat(self.depth), self.name)
            }
            KernelTreeRowKind::RootFile => format!("[file] {}", self.path.display()),
            KernelTreeRowKind::MissingPath => format!("[missing] {}", self.path.display()),
        }
    }
}

#[derive(Debug)]
struct AppState {
    runtime: RuntimeConfig,
    ui_state_path: PathBuf,
    active_thread_mailbox: String,
    ui_page: UiPage,
    focus: Pane,
    code_focus: CodePaneFocus,
    subscriptions: Vec<SubscriptionItem>,
    enabled_group_expanded: bool,
    disabled_group_expanded: bool,
    threads: Vec<ThreadRow>,
    filtered_thread_indices: Vec<usize>,
    subscription_index: usize,
    subscription_row_index: usize,
    kernel_tree_rows: Vec<KernelTreeRow>,
    kernel_tree_expanded_paths: HashSet<PathBuf>,
    kernel_tree_row_index: usize,
    code_preview_scroll: u16,
    thread_index: usize,
    preview_scroll: u16,
    started_at: Instant,
    status: String,
    palette: CommandPaletteState,
    search: SearchState,
}

impl AppState {
    fn new(threads: Vec<ThreadRow>, runtime: RuntimeConfig) -> Self {
        Self::new_with_ui_state(threads, runtime, None)
    }

    fn new_with_ui_state(
        threads: Vec<ThreadRow>,
        runtime: RuntimeConfig,
        persisted: Option<UiState>,
    ) -> Self {
        let ui_state_path = ui_state::path_for_data_dir(&runtime.data_dir);
        let enabled_mailboxes = persisted
            .as_ref()
            .map(UiState::normalized_enabled_mailboxes)
            .unwrap_or_default();
        let enabled_mailboxes: HashSet<String> = enabled_mailboxes.into_iter().collect();
        let active_thread_mailbox = persisted
            .as_ref()
            .and_then(|state| state.active_mailbox.as_ref())
            .map(|mailbox| mailbox.trim())
            .filter(|mailbox| !mailbox.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| runtime.imap_mailbox.clone());
        let subscriptions = default_subscriptions(
            &runtime.imap_mailbox,
            &enabled_mailboxes,
            Some(active_thread_mailbox.as_str()),
        );
        let kernel_tree_expanded_paths = default_kernel_tree_expanded_paths(&runtime.kernel_trees);
        let kernel_tree_rows =
            build_kernel_tree_rows(&runtime.kernel_trees, &kernel_tree_expanded_paths);
        let mut state = Self {
            active_thread_mailbox,
            runtime,
            ui_state_path,
            ui_page: UiPage::Mail,
            focus: Pane::Subscriptions,
            code_focus: CodePaneFocus::Tree,
            subscriptions,
            enabled_group_expanded: persisted
                .as_ref()
                .map(|state| state.enabled_group_expanded)
                .unwrap_or(true),
            disabled_group_expanded: persisted
                .as_ref()
                .map(|state| state.disabled_group_expanded)
                .unwrap_or(true),
            threads,
            filtered_thread_indices: Vec::new(),
            subscription_index: 0,
            subscription_row_index: 0,
            kernel_tree_rows,
            kernel_tree_expanded_paths,
            kernel_tree_row_index: 0,
            code_preview_scroll: 0,
            thread_index: 0,
            preview_scroll: 0,
            started_at: Instant::now(),
            status: "ready".to_string(),
            palette: CommandPaletteState::default(),
            search: SearchState::default(),
        };
        if let Some(index) = state
            .subscriptions
            .iter()
            .position(|item| item.mailbox == state.active_thread_mailbox)
        {
            state.subscription_index = index;
        }
        state.apply_thread_filter();
        state.sync_subscription_row_to_selected_item();
        state
    }

    fn apply_thread_filter(&mut self) {
        let query = self.search.applied_query.trim().to_ascii_lowercase();

        self.filtered_thread_indices = self
            .threads
            .iter()
            .enumerate()
            .filter_map(|(index, row)| {
                if query.is_empty()
                    || row.subject.to_ascii_lowercase().contains(&query)
                    || row.from_addr.to_ascii_lowercase().contains(&query)
                    || row.message_id.to_ascii_lowercase().contains(&query)
                {
                    Some(index)
                } else {
                    None
                }
            })
            .collect();

        if self.thread_index >= self.filtered_thread_indices.len() {
            self.thread_index = self.filtered_thread_indices.len().saturating_sub(1);
        }

        if !query.is_empty() {
            self.status = format!(
                "search '{}': {} matches",
                self.search.applied_query,
                self.filtered_thread_indices.len()
            );
        }
    }

    fn replace_threads(&mut self, threads: Vec<ThreadRow>) {
        self.threads = threads;
        self.thread_index = 0;
        self.preview_scroll = 0;
        self.apply_thread_filter();
    }

    fn enabled_mailboxes(&self) -> Vec<String> {
        self.subscriptions
            .iter()
            .filter(|item| item.enabled)
            .map(|item| item.mailbox.clone())
            .collect()
    }

    fn to_ui_state(&self) -> UiState {
        UiState {
            enabled_mailboxes: self.enabled_mailboxes(),
            enabled_group_expanded: self.enabled_group_expanded,
            disabled_group_expanded: self.disabled_group_expanded,
            active_mailbox: Some(self.active_thread_mailbox.clone()),
        }
    }

    fn persist_ui_state(&self) {
        if let Err(error) = ui_state::save(&self.ui_state_path, &self.to_ui_state()) {
            tracing::warn!(
                path = %self.ui_state_path.display(),
                error = %error,
                "failed to persist ui state"
            );
        }
    }

    fn subscription_rows(&self) -> Vec<SubscriptionRow> {
        let enabled_count = self
            .subscriptions
            .iter()
            .filter(|item| item.enabled)
            .count();
        let disabled_count = self.subscriptions.len().saturating_sub(enabled_count);

        let mut rows = Vec::new();
        let enabled_marker = if self.enabled_group_expanded {
            "▼"
        } else {
            "▶"
        };
        rows.push(SubscriptionRow {
            kind: SubscriptionRowKind::EnabledHeader,
            text: format!("{enabled_marker} enabled ({enabled_count})"),
        });

        if self.enabled_group_expanded {
            for (index, item) in self
                .subscriptions
                .iter()
                .enumerate()
                .filter(|(_, item)| item.enabled)
            {
                rows.push(SubscriptionRow {
                    kind: SubscriptionRowKind::Item(index),
                    text: format!("  {}", subscription_line(item)),
                });
            }
        }

        let disabled_marker = if self.disabled_group_expanded {
            "▼"
        } else {
            "▶"
        };
        rows.push(SubscriptionRow {
            kind: SubscriptionRowKind::DisabledHeader,
            text: format!("{disabled_marker} disabled ({disabled_count})"),
        });

        if self.disabled_group_expanded {
            for (index, item) in self
                .subscriptions
                .iter()
                .enumerate()
                .filter(|(_, item)| !item.enabled)
            {
                rows.push(SubscriptionRow {
                    kind: SubscriptionRowKind::Item(index),
                    text: format!("  {}", subscription_line(item)),
                });
            }
        }

        rows
    }

    fn selected_subscription_row_kind(&self) -> Option<SubscriptionRowKind> {
        let rows = self.subscription_rows();
        if rows.is_empty() {
            return None;
        }

        let selected = self
            .subscription_row_index
            .min(rows.len().saturating_sub(1));
        rows.get(selected).map(|row| row.kind)
    }

    fn selected_subscription_index(&self) -> Option<usize> {
        match self.selected_subscription_row_kind() {
            Some(SubscriptionRowKind::Item(index)) => Some(index),
            _ => None,
        }
    }

    fn sync_subscription_row_to_selected_item(&mut self) {
        let rows = self.subscription_rows();
        if rows.is_empty() {
            self.subscription_row_index = 0;
            return;
        }

        self.subscription_row_index = rows
            .iter()
            .position(
                |row| matches!(row.kind, SubscriptionRowKind::Item(index) if index == self.subscription_index),
            )
            .unwrap_or(0);
    }

    fn clamp_subscription_row_selection(&mut self) {
        let rows = self.subscription_rows();
        if rows.is_empty() {
            self.subscription_row_index = 0;
            return;
        }

        if self.subscription_row_index >= rows.len() {
            self.subscription_row_index = rows.len().saturating_sub(1);
        }

        if let Some(SubscriptionRowKind::Item(index)) =
            rows.get(self.subscription_row_index).map(|row| row.kind)
        {
            self.subscription_index = index;
        }
    }

    fn set_current_subscription_enabled(&mut self, enabled: bool) {
        let Some(selected_index) = self.selected_subscription_index() else {
            self.status = "move to a subscription item, then press y/n".to_string();
            return;
        };

        let mailbox = self.subscriptions[selected_index].mailbox.clone();
        if let Some(item) = self.subscriptions.get_mut(selected_index) {
            item.enabled = enabled;
        }

        self.sort_subscriptions_keep_selected(&mailbox);
        let marker = if enabled { "enabled" } else { "disabled" };
        self.status = format!("{marker} subscription {mailbox}");
        self.persist_ui_state();
    }

    fn sort_subscriptions_keep_selected(&mut self, selected_mailbox: &str) {
        self.subscriptions.sort_by(|left, right| {
            right
                .enabled
                .cmp(&left.enabled)
                .then_with(|| left.mailbox.cmp(&right.mailbox))
        });

        self.subscription_index = self
            .subscriptions
            .iter()
            .position(|item| item.mailbox == selected_mailbox)
            .unwrap_or(0);
        self.sync_subscription_row_to_selected_item();
    }

    fn toggle_selected_subscription_group(&mut self) {
        match self.selected_subscription_row_kind() {
            Some(SubscriptionRowKind::EnabledHeader) => {
                self.enabled_group_expanded = !self.enabled_group_expanded;
                let state = if self.enabled_group_expanded {
                    "expanded"
                } else {
                    "collapsed"
                };
                self.status = format!("enabled group {state}");
                self.clamp_subscription_row_selection();
                self.persist_ui_state();
            }
            Some(SubscriptionRowKind::DisabledHeader) => {
                self.disabled_group_expanded = !self.disabled_group_expanded;
                let state = if self.disabled_group_expanded {
                    "expanded"
                } else {
                    "collapsed"
                };
                self.status = format!("disabled group {state}");
                self.clamp_subscription_row_selection();
                self.persist_ui_state();
            }
            _ => {}
        }
    }

    fn handle_subscription_enter(&mut self) {
        match self.selected_subscription_row_kind() {
            Some(SubscriptionRowKind::EnabledHeader)
            | Some(SubscriptionRowKind::DisabledHeader) => {
                self.toggle_selected_subscription_group()
            }
            Some(SubscriptionRowKind::Item(_)) => self.open_threads_for_selected_subscription(),
            None => {}
        }
    }

    fn open_threads_for_selected_subscription(&mut self) {
        let Some(selected_index) = self.selected_subscription_index() else {
            self.status = "press Enter on a subscription item".to_string();
            return;
        };
        let Some(item) = self.subscriptions.get(selected_index) else {
            return;
        };
        let mailbox = item.mailbox.clone();
        let enabled = item.enabled;

        if !enabled {
            self.status = format!("subscription {} is disabled, press y to enable", mailbox);
            return;
        }

        match mail_store::load_thread_rows_by_mailbox(&self.runtime.database_path, &mailbox, 500) {
            Ok(rows) if !rows.is_empty() => {
                self.active_thread_mailbox = mailbox.clone();
                self.replace_threads(rows);
                self.status = format!("showing threads for {}", mailbox);
                self.persist_ui_state();
            }
            Ok(_) => {
                let request = sync_worker::SyncRequest {
                    mailbox: mailbox.clone(),
                    fixture_dir: None,
                    uidvalidity: None,
                    reconnect_attempts: PALETTE_SYNC_RECONNECT_ATTEMPTS,
                };

                match sync_worker::run(&self.runtime, request) {
                    Ok(summary) => match mail_store::load_thread_rows_by_mailbox(
                        &self.runtime.database_path,
                        &mailbox,
                        500,
                    ) {
                        Ok(fresh_rows) => {
                            self.active_thread_mailbox = mailbox.clone();
                            self.replace_threads(fresh_rows);
                            self.status = format!(
                                "synced {}: fetched={} inserted={} updated={}",
                                mailbox, summary.fetched, summary.inserted, summary.updated
                            );
                            self.persist_ui_state();
                        }
                        Err(error) => {
                            self.status = format!(
                                "sync ok but failed to reload threads for {}: {error}",
                                mailbox
                            );
                        }
                    },
                    Err(error) => {
                        self.status = format!("failed to sync {}: {error}", mailbox);
                    }
                }
            }
            Err(error) => {
                self.status = format!("failed to load threads for {}: {error}", mailbox);
            }
        }
    }

    fn selected_thread(&self) -> Option<&ThreadRow> {
        self.filtered_thread_indices
            .get(self.thread_index)
            .and_then(|index| self.threads.get(*index))
    }

    fn selected_kernel_tree_row(&self) -> Option<&KernelTreeRow> {
        self.kernel_tree_rows.get(
            self.kernel_tree_row_index
                .min(self.kernel_tree_rows.len().saturating_sub(1)),
        )
    }

    fn selected_kernel_tree_path(&self) -> Option<PathBuf> {
        self.selected_kernel_tree_row().map(|row| row.path.clone())
    }

    fn selected_kernel_tree_file_path(&self) -> Option<&Path> {
        self.selected_kernel_tree_row()
            .filter(|row| row.is_file())
            .map(|row| row.path.as_path())
    }

    fn refresh_kernel_tree_rows(&mut self, selected_path_hint: Option<&Path>) {
        self.kernel_tree_rows =
            build_kernel_tree_rows(&self.runtime.kernel_trees, &self.kernel_tree_expanded_paths);
        if self.kernel_tree_rows.is_empty() {
            self.kernel_tree_row_index = 0;
            return;
        }

        if let Some(path) = selected_path_hint
            && let Some(index) = self
                .kernel_tree_rows
                .iter()
                .position(|row| row.path == path)
        {
            self.kernel_tree_row_index = index;
            return;
        }

        if self.kernel_tree_row_index >= self.kernel_tree_rows.len() {
            self.kernel_tree_row_index = self.kernel_tree_rows.len().saturating_sub(1);
        }
    }

    fn supports_code_browser(&self) -> bool {
        !self.runtime.kernel_trees.is_empty()
    }

    fn toggle_ui_page(&mut self) {
        if matches!(self.ui_page, UiPage::Mail) && !self.supports_code_browser() {
            self.status =
                "no kernel tree configured; set [kernel].tree or [kernel].trees".to_string();
            return;
        }

        self.ui_page = self.ui_page.toggled();
        match self.ui_page {
            UiPage::Mail => {
                self.status = "switched to mail page".to_string();
            }
            UiPage::CodeBrowser => {
                self.refresh_kernel_tree_rows(self.selected_kernel_tree_path().as_deref());
                self.code_preview_scroll = 0;
                self.status = "switched to code browser page".to_string();
            }
        }
    }

    fn move_subscription_up(&mut self) {
        let rows = self.subscription_rows();
        if rows.is_empty() {
            return;
        }
        if self.subscription_row_index >= rows.len() {
            self.subscription_row_index = rows.len().saturating_sub(1);
        }
        if self.subscription_row_index > 0 {
            self.subscription_row_index -= 1;
        }
        if let Some(SubscriptionRowKind::Item(index)) =
            rows.get(self.subscription_row_index).map(|row| row.kind)
        {
            self.subscription_index = index;
        }
    }

    fn move_subscription_down(&mut self) {
        let rows = self.subscription_rows();
        if rows.is_empty() {
            return;
        }
        if self.subscription_row_index >= rows.len() {
            self.subscription_row_index = rows.len().saturating_sub(1);
        } else if self.subscription_row_index + 1 < rows.len() {
            self.subscription_row_index += 1;
        }
        if let Some(SubscriptionRowKind::Item(index)) =
            rows.get(self.subscription_row_index).map(|row| row.kind)
        {
            self.subscription_index = index;
        }
    }

    fn move_kernel_tree_up(&mut self) {
        let previous_file = self.selected_kernel_tree_file_path().map(Path::to_path_buf);
        if self.kernel_tree_row_index > 0 {
            self.kernel_tree_row_index -= 1;
        }
        let next_file = self.selected_kernel_tree_file_path().map(Path::to_path_buf);
        if previous_file != next_file {
            self.code_preview_scroll = 0;
        }
    }

    fn move_kernel_tree_down(&mut self) {
        let previous_file = self.selected_kernel_tree_file_path().map(Path::to_path_buf);
        if self.kernel_tree_row_index + 1 < self.kernel_tree_rows.len() {
            self.kernel_tree_row_index += 1;
        }
        let next_file = self.selected_kernel_tree_file_path().map(Path::to_path_buf);
        if previous_file != next_file {
            self.code_preview_scroll = 0;
        }
    }

    fn handle_kernel_tree_enter(&mut self) {
        let Some(row) = self.selected_kernel_tree_row().cloned() else {
            self.status = "kernel tree is empty".to_string();
            return;
        };

        if !row.expandable {
            self.code_preview_scroll = 0;
            self.status = format!("selected {}", row.path.display());
            return;
        }

        if row.expanded {
            self.kernel_tree_expanded_paths.remove(&row.path);
            self.status = format!("collapsed {}", row.path.display());
        } else {
            self.kernel_tree_expanded_paths.insert(row.path.clone());
            self.status = format!("expanded {}", row.path.display());
        }
        self.refresh_kernel_tree_rows(Some(&row.path));
        self.code_preview_scroll = 0;
    }

    fn move_focus_next(&mut self) {
        match self.ui_page {
            UiPage::Mail => {
                self.focus = self.focus.next();
            }
            UiPage::CodeBrowser => {
                self.code_focus = self.code_focus.next();
            }
        }
    }

    fn move_focus_previous(&mut self) {
        match self.ui_page {
            UiPage::Mail => {
                self.focus = self.focus.previous();
            }
            UiPage::CodeBrowser => {
                self.code_focus = self.code_focus.previous();
            }
        }
    }

    fn move_up(&mut self) {
        match self.ui_page {
            UiPage::Mail => match self.focus {
                Pane::Subscriptions => {
                    self.move_subscription_up();
                }
                Pane::Threads => {
                    if self.thread_index > 0 {
                        self.thread_index -= 1;
                        self.preview_scroll = 0;
                    }
                }
                Pane::Preview => {
                    self.preview_scroll = self.preview_scroll.saturating_sub(1);
                }
            },
            UiPage::CodeBrowser => match self.code_focus {
                CodePaneFocus::Tree => self.move_kernel_tree_up(),
                CodePaneFocus::Source => {
                    self.code_preview_scroll = self.code_preview_scroll.saturating_sub(1);
                }
            },
        }
    }

    fn move_down(&mut self) {
        match self.ui_page {
            UiPage::Mail => match self.focus {
                Pane::Subscriptions => {
                    self.move_subscription_down();
                }
                Pane::Threads => {
                    if self.thread_index + 1 < self.filtered_thread_indices.len() {
                        self.thread_index += 1;
                        self.preview_scroll = 0;
                    }
                }
                Pane::Preview => {
                    self.preview_scroll = self.preview_scroll.saturating_add(1);
                }
            },
            UiPage::CodeBrowser => match self.code_focus {
                CodePaneFocus::Tree => self.move_kernel_tree_down(),
                CodePaneFocus::Source => {
                    self.code_preview_scroll = self.code_preview_scroll.saturating_add(1);
                }
            },
        }
    }

    fn open_search(&mut self) {
        self.search.active = true;
        self.search.input = self.search.applied_query.clone();
        self.status = "search mode".to_string();
    }

    fn close_search(&mut self) {
        self.search.active = false;
        self.search.input.clear();
        self.status = "search cancelled".to_string();
    }

    fn apply_search(&mut self) {
        self.search.active = false;
        self.search.applied_query = self.search.input.trim().to_string();
        self.thread_index = 0;
        self.apply_thread_filter();
    }

    fn toggle_palette(&mut self) {
        self.palette.open = !self.palette.open;
        if self.palette.open {
            self.palette.clear_completion();
            self.status = "command palette opened".to_string();
        } else {
            self.palette.input.clear();
            self.palette.clear_completion();
            self.status = "command palette closed".to_string();
        }
    }

    fn close_palette(&mut self) {
        self.palette.open = false;
        self.palette.input.clear();
        self.palette.clear_completion();
        self.status = "command palette closed".to_string();
    }
}

enum LoopAction {
    Continue,
    Exit,
}

pub fn run(config: &RuntimeConfig, bootstrap: &BootstrapState) -> Result<()> {
    let ui_state_path = ui_state::path_for_data_dir(&config.data_dir);
    let persisted_ui_state = load_persisted_ui_state(&ui_state_path);
    let initial_mailbox = persisted_ui_state
        .as_ref()
        .and_then(|state| state.active_mailbox.as_ref())
        .map(|mailbox| mailbox.trim())
        .filter(|mailbox| !mailbox.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| config.imap_mailbox.clone());
    let threads =
        mail_store::load_thread_rows_by_mailbox(&config.database_path, &initial_mailbox, 500)?;
    let mut terminal = setup_terminal()?;
    let guard = TerminalGuard;
    let mut state = if let Some(persisted) = persisted_ui_state {
        AppState::new_with_ui_state(threads, config.clone(), Some(persisted))
    } else {
        AppState::new(threads, config.clone())
    };
    if state.filtered_thread_indices.is_empty() {
        state.status = "no synced thread data, run `courier sync` first".to_string();
    }

    let result = tui_loop(&mut terminal, &mut state, config, bootstrap);

    drop(guard);
    result
}

fn load_persisted_ui_state(path: &std::path::Path) -> Option<UiState> {
    match ui_state::load(path) {
        Ok(state) => state,
        Err(error) => {
            tracing::warn!(
                path = %path.display(),
                error = %error,
                "failed to load persisted ui state"
            );
            None
        }
    }
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode().map_err(|error| {
        CourierError::with_source(ErrorCode::Tui, "failed to enable raw mode", error)
    })?;

    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).map_err(|error| {
        CourierError::with_source(ErrorCode::Tui, "failed to enter alternate screen", error)
    })?;

    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend).map_err(|error| {
        CourierError::with_source(
            ErrorCode::Tui,
            "failed to initialize terminal backend",
            error,
        )
    })?;

    Ok(terminal)
}

fn tui_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    state: &mut AppState,
    config: &RuntimeConfig,
    bootstrap: &BootstrapState,
) -> Result<()> {
    loop {
        terminal
            .draw(|frame| draw(frame, state, config, bootstrap))
            .map_err(|error| {
                CourierError::with_source(ErrorCode::Tui, "failed to render frame", error)
            })?;

        if event::poll(Duration::from_millis(200)).map_err(|error| {
            CourierError::with_source(ErrorCode::Tui, "failed to poll terminal events", error)
        })? {
            let event = event::read().map_err(|error| {
                CourierError::with_source(ErrorCode::Tui, "failed to read terminal event", error)
            })?;

            if let Event::Key(key) = event {
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                if let LoopAction::Exit = handle_key_event(state, key) {
                    break;
                }
            }
        }
    }

    Ok(())
}

fn handle_key_event(state: &mut AppState, key: KeyEvent) -> LoopAction {
    if state.palette.open {
        if is_palette_toggle(key) {
            state.close_palette();
            return LoopAction::Continue;
        }
        return handle_palette_key_event(state, key);
    }

    if state.search.active {
        return handle_search_key_event(state, key);
    }

    if is_palette_open_shortcut(key) {
        state.toggle_palette();
        return LoopAction::Continue;
    }

    match key.code {
        KeyCode::Char('/') => {
            if matches!(state.ui_page, UiPage::Mail) {
                state.open_search();
            } else {
                state.status = "search is only available on mail page".to_string();
            }
        }
        KeyCode::Char(character)
            if matches!(state.ui_page, UiPage::Mail)
                && matches!(state.focus, Pane::Subscriptions)
                && character.eq_ignore_ascii_case(&'y') =>
        {
            state.set_current_subscription_enabled(true);
        }
        KeyCode::Char(character)
            if matches!(state.ui_page, UiPage::Mail)
                && matches!(state.focus, Pane::Subscriptions)
                && character.eq_ignore_ascii_case(&'n') =>
        {
            state.set_current_subscription_enabled(false);
        }
        KeyCode::Tab => state.toggle_ui_page(),
        KeyCode::Char('j') => state.move_focus_previous(),
        KeyCode::Char('l') => state.move_focus_next(),
        KeyCode::Char('i') => state.move_up(),
        KeyCode::Char('k') => state.move_down(),
        KeyCode::Enter => match state.ui_page {
            UiPage::Mail => match state.focus {
                Pane::Subscriptions => state.handle_subscription_enter(),
                Pane::Threads => {
                    if let Some(thread) = state.selected_thread() {
                        state.status = format!("selected {}", thread.message_id);
                    }
                }
                Pane::Preview => {}
            },
            UiPage::CodeBrowser => {
                if matches!(state.code_focus, CodePaneFocus::Tree) {
                    state.handle_kernel_tree_enter();
                }
            }
        },
        KeyCode::Esc => {
            state.status = "open command palette with : (preferred) or Ctrl+`".to_string();
        }
        KeyCode::Char('q') => {
            state.status = "q emergency exit disabled; use command palette quit/exit".to_string();
        }
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.status = "Ctrl+C is disabled, use command palette quit/exit".to_string();
        }
        _ => {}
    }

    LoopAction::Continue
}

fn handle_search_key_event(state: &mut AppState, key: KeyEvent) -> LoopAction {
    match key.code {
        KeyCode::Esc => state.close_search(),
        KeyCode::Enter => state.apply_search(),
        KeyCode::Backspace => {
            state.search.input.pop();
        }
        KeyCode::Char(character)
            if !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER) =>
        {
            state.search.input.push(character);
        }
        _ => {}
    }

    LoopAction::Continue
}

fn handle_palette_key_event(state: &mut AppState, key: KeyEvent) -> LoopAction {
    match key.code {
        KeyCode::Esc => {
            state.close_palette();
        }
        KeyCode::Enter => {
            let command = state.palette.input.trim().to_ascii_lowercase();
            state.palette.input.clear();
            state.palette.clear_completion();

            if command.is_empty() {
                state.status = "empty command".to_string();
                return LoopAction::Continue;
            }

            match command.as_str() {
                "quit" | "exit" => return LoopAction::Exit,
                "help" => {
                    state.status =
                        "commands: quit, exit, help, sync [mailbox], config ...".to_string();
                }
                value if value.split_whitespace().next() == Some("sync") => {
                    run_palette_sync(state, value);
                }
                value if value.split_whitespace().next() == Some("config") => {
                    run_palette_config(state, value);
                }
                _ => {
                    state.status = format!("unknown command: {command}");
                }
            }
        }
        KeyCode::Backspace => {
            state.palette.input.pop();
            state.palette.clear_completion();
        }
        KeyCode::Tab => {
            apply_palette_completion(state);
        }
        KeyCode::Char(character)
            if !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER) =>
        {
            state.palette.input.push(character);
            state.palette.clear_completion();
        }
        _ => {}
    }

    LoopAction::Continue
}

fn run_palette_sync(state: &mut AppState, command: &str) {
    let mailbox_override = command
        .split_whitespace()
        .nth(1)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);

    let mailboxes = if let Some(mailbox) = mailbox_override {
        vec![mailbox]
    } else {
        let enabled = state.enabled_mailboxes();
        if enabled.is_empty() {
            vec![state.runtime.imap_mailbox.clone()]
        } else {
            enabled
        }
    };

    let mut success = 0usize;
    let mut failed = 0usize;
    let mut total_fetched = 0usize;
    let mut total_inserted = 0usize;
    let mut total_updated = 0usize;
    let mut first_error: Option<String> = None;

    for mailbox in mailboxes {
        let request = sync_worker::SyncRequest {
            mailbox: mailbox.clone(),
            fixture_dir: None,
            uidvalidity: None,
            reconnect_attempts: PALETTE_SYNC_RECONNECT_ATTEMPTS,
        };

        match sync_worker::run(&state.runtime, request) {
            Ok(summary) => {
                success += 1;
                total_fetched += summary.fetched;
                total_inserted += summary.inserted;
                total_updated += summary.updated;
            }
            Err(error) => {
                failed += 1;
                if first_error.is_none() {
                    first_error = Some(format!("{mailbox}: {error}"));
                }
            }
        }
    }

    if success > 0
        && let Ok(rows) = mail_store::load_thread_rows_by_mailbox(
            &state.runtime.database_path,
            &state.active_thread_mailbox,
            500,
        )
    {
        state.replace_threads(rows);
    }

    state.status = if failed == 0 {
        format!(
            "sync ok: mailboxes={} fetched={} inserted={} updated={}",
            success, total_fetched, total_inserted, total_updated
        )
    } else if success == 0 {
        format!(
            "sync failed: {}",
            first_error.unwrap_or_else(|| "unknown error".to_string())
        )
    } else {
        format!(
            "sync partial: ok={} failed={} fetched={} inserted={} updated={} first_error={}",
            success,
            failed,
            total_fetched,
            total_inserted,
            total_updated,
            first_error.unwrap_or_else(|| "unknown error".to_string())
        )
    };
}

fn run_palette_config(state: &mut AppState, command: &str) {
    let mut segments = command.split_whitespace();
    let _ = segments.next();
    let action = segments.next().unwrap_or("show").to_ascii_lowercase();

    match action.as_str() {
        "show" => {
            if let Some(key) = segments.next() {
                show_config_key(state, key);
            } else {
                state.status = format!(
                    "config file: {} | use: config get <key>, config set <key> <value>",
                    state.runtime.config_path.display()
                );
            }
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

            match update_config_key_in_file(&state.runtime.config_path, key, &value_literal) {
                Ok(rendered_value) => match reload_runtime_from_config(state) {
                    Ok(()) => {
                        state.status = format!("config updated: {key} = {rendered_value}");
                    }
                    Err(error) => {
                        state.status =
                            format!("config file updated but runtime reload failed: {error}");
                    }
                },
                Err(error) => {
                    state.status = format!("failed to set config key {key}: {error}");
                }
            }
        }
        "help" => {
            state.status = "config usage: show [key] | get <key> | set <key> <value>".to_string();
        }
        _ => {
            state.status = "config usage: show [key] | get <key> | set <key> <value>".to_string();
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
) -> std::result::Result<String, String> {
    let mut table = read_config_table(config_path)?;
    let value = parse_toml_value_literal(value_literal);
    set_config_key(&mut table, key, value)?;
    write_config_table(config_path, &table)?;

    let rendered = lookup_config_key(&table, key)
        .map(render_toml_value)
        .unwrap_or_else(|| "<unknown>".to_string());
    Ok(rendered)
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

fn write_config_table(config_path: &Path, table: &TomlTable) -> std::result::Result<(), String> {
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "failed to create config directory {}: {error}",
                parent.display()
            )
        })?;
    }

    let mut content = toml::to_string_pretty(table)
        .map_err(|error| format!("failed to serialize config table: {error}"))?;
    if !content.ends_with('\n') {
        content.push('\n');
    }

    fs::write(config_path, content).map_err(|error| {
        format!(
            "failed to write config file {}: {error}",
            config_path.display()
        )
    })
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

    let leaf = key_parts.pop().expect("leaf key exists");
    let mut current = table;
    for segment in key_parts {
        let node = current
            .entry(segment.to_string())
            .or_insert_with(|| TomlValue::Table(TomlTable::new()));
        if !node.is_table() {
            *node = TomlValue::Table(TomlTable::new());
        }
        current = node
            .as_table_mut()
            .ok_or_else(|| format!("key segment {segment} is not a table"))?;
    }
    current.insert(leaf.to_string(), value);
    Ok(())
}

fn parse_toml_value_literal(value_literal: &str) -> TomlValue {
    let literal = value_literal.trim();
    if literal.is_empty() {
        return TomlValue::String(String::new());
    }

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

fn reload_runtime_from_config(state: &mut AppState) -> std::result::Result<(), String> {
    let selected_path_hint = state.selected_kernel_tree_path();
    match crate::infra::config::load(Some(&state.runtime.config_path)) {
        Ok(runtime) => {
            state.runtime = runtime;
            state.ui_state_path = ui_state::path_for_data_dir(&state.runtime.data_dir);
            state.refresh_kernel_tree_rows(selected_path_hint.as_deref());
            if matches!(state.ui_page, UiPage::CodeBrowser) && !state.supports_code_browser() {
                state.ui_page = UiPage::Mail;
                state.code_focus = CodePaneFocus::Tree;
            }
            Ok(())
        }
        Err(error) => Err(error.to_string()),
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
        "source.mailbox" | "imap.mailbox" => Some(state.runtime.imap_mailbox.clone()),
        "source.lore_base_url" => Some(state.runtime.lore_base_url.clone()),
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

fn apply_palette_completion(state: &mut AppState) {
    let input_before_completion = state.palette.input.clone();
    let context = parse_palette_completion_context(&state.palette.input);
    let mut suggestions = palette_completion_suggestions(state, &context);
    let prefix_lower = context.active_token.to_ascii_lowercase();
    suggestions.retain(|suggestion| {
        suggestion
            .value
            .to_ascii_lowercase()
            .starts_with(&prefix_lower)
    });
    suggestions.sort_by(|left, right| left.value.cmp(&right.value));
    suggestions.dedup_by(|left, right| left.value == right.value);

    if suggestions.is_empty() {
        state.palette.clear_completion();
        state.status = "no completion candidates".to_string();
        return;
    }

    let completion_values: Vec<String> = suggestions
        .iter()
        .map(|suggestion| suggestion.value.clone())
        .collect();

    if completion_values.len() == 1 {
        let candidate = completion_values[0].clone();
        state.palette.input = format!("{}{} ", context.prefix, candidate);
        state.palette.clear_completion();
        state.status = format!("completed: {candidate}");
        return;
    }

    let common_prefix = longest_common_prefix(&completion_values);
    if common_prefix.len() > context.active_token.len() {
        state.palette.input = format!("{}{}", context.prefix, common_prefix);
    }

    let show_suggestions =
        context.active_token.is_empty() || state.palette.last_tab_input == input_before_completion;
    state.palette.suggestions = suggestions;
    state.palette.show_suggestions = show_suggestions;
    state.palette.last_tab_input = state.palette.input.clone();

    let summary = state
        .palette
        .suggestions
        .iter()
        .map(|suggestion| suggestion.value.as_str())
        .take(5)
        .collect::<Vec<_>>()
        .join(", ");

    state.status = if show_suggestions {
        format!(
            "completion options: {}",
            if summary.is_empty() {
                "<none>".to_string()
            } else {
                summary
            }
        )
    } else {
        format!(
            "{} completion candidates (Tab again to list)",
            state.palette.suggestions.len()
        )
    };
}

fn palette_completion_suggestions(
    state: &AppState,
    context: &PaletteCompletionContext,
) -> Vec<PaletteSuggestion> {
    if context.active_index == 0 {
        return PALETTE_COMMANDS
            .iter()
            .map(|command| PaletteSuggestion {
                value: command.name.to_string(),
                description: Some(command.description.to_string()),
            })
            .collect();
    }

    let command = context
        .tokens
        .first()
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_default();

    match command.as_str() {
        "config" => config_completion_suggestions(state, context),
        "sync" => sync_completion_suggestions(state, context),
        _ => Vec::new(),
    }
}

#[derive(Debug, Clone)]
struct PaletteCompletionContext {
    tokens: Vec<String>,
    active_index: usize,
    active_token: String,
    prefix: String,
}

fn parse_palette_completion_context(input: &str) -> PaletteCompletionContext {
    let tokens: Vec<String> = input.split_whitespace().map(ToOwned::to_owned).collect();
    let trailing_space = input.chars().last().is_some_and(char::is_whitespace);

    if trailing_space {
        return PaletteCompletionContext {
            active_index: tokens.len(),
            active_token: String::new(),
            prefix: input.to_string(),
            tokens,
        };
    }

    let split_index = input
        .char_indices()
        .rev()
        .find_map(|(index, character)| character.is_whitespace().then_some(index));

    if let Some(index) = split_index {
        return PaletteCompletionContext {
            active_index: tokens.len().saturating_sub(1),
            active_token: input[index + 1..].to_string(),
            prefix: input[..=index].to_string(),
            tokens,
        };
    }

    PaletteCompletionContext {
        active_index: 0,
        active_token: input.to_string(),
        prefix: String::new(),
        tokens,
    }
}

fn config_completion_suggestions(
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
            let keys = if action == "set" {
                CONFIG_SET_KEYS
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
        "b4.path" => vec![PaletteSuggestion {
            value: "\"/usr/bin/b4\"".to_string(),
            description: Some("Path to b4 executable".to_string()),
        }],
        _ => Vec::new(),
    }
}

fn sync_completion_suggestions(
    state: &AppState,
    context: &PaletteCompletionContext,
) -> Vec<PaletteSuggestion> {
    if context.active_index != 1 {
        return Vec::new();
    }

    let mut candidates: Vec<String> = state
        .subscriptions
        .iter()
        .map(|subscription| subscription.mailbox.clone())
        .collect();
    candidates.push(state.active_thread_mailbox.clone());
    candidates.push(state.runtime.imap_mailbox.clone());
    candidates.sort();
    candidates.dedup();
    candidates
        .into_iter()
        .map(|value| PaletteSuggestion {
            value,
            description: Some("Mailbox".to_string()),
        })
        .collect()
}

fn longest_common_prefix(values: &[String]) -> String {
    let Some(first) = values.first() else {
        return String::new();
    };

    let mut prefix = first.clone();
    for value in values.iter().skip(1) {
        let mut matched_bytes = 0usize;
        for (left, right) in prefix.chars().zip(value.chars()) {
            if left == right {
                matched_bytes += left.len_utf8();
            } else {
                break;
            }
        }
        prefix.truncate(matched_bytes);
        if prefix.is_empty() {
            break;
        }
    }
    prefix
}

fn lore_archive_url(mailbox: &str) -> String {
    format!("https://lore.kernel.org/{mailbox}/")
}

fn mailbox_description(mailbox: &str) -> String {
    if let Some(template) = VGER_SUBSCRIPTIONS
        .iter()
        .find(|entry| entry.mailbox == mailbox)
    {
        template.description.to_string()
    } else {
        "Custom mailbox".to_string()
    }
}

fn is_palette_toggle(key: KeyEvent) -> bool {
    matches!(key.code, KeyCode::F(1))
        || (key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(
                key.code,
                KeyCode::Char('`')
                    | KeyCode::Char('~')
                    | KeyCode::Char('/')
                    | KeyCode::Char('?')
                    | KeyCode::Char('_')
                    | KeyCode::Null
            ))
}

fn is_palette_open_shortcut(key: KeyEvent) -> bool {
    is_palette_toggle(key) || is_palette_open_fallback_key(key)
}

fn is_palette_open_fallback_key(key: KeyEvent) -> bool {
    matches!(key.code, KeyCode::Char(':'))
        && !key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER)
}

fn draw(
    frame: &mut Frame<'_>,
    state: &AppState,
    config: &RuntimeConfig,
    bootstrap: &BootstrapState,
) {
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(10),
            Constraint::Length(1),
        ])
        .split(frame.area());

    let uptime = state.started_at.elapsed().as_secs();
    let page_label = match state.ui_page {
        UiPage::Mail => "mail",
        UiPage::CodeBrowser => "code",
    };
    let header = format!(
        "page: {} | mailbox: {} | db schema: {} | db: {} | threads: {} | uptime: {}s",
        page_label,
        state.active_thread_mailbox,
        bootstrap.db.schema_version,
        bootstrap.db.path.display(),
        state.filtered_thread_indices.len(),
        uptime
    );
    let header_widget =
        Paragraph::new(header).style(Style::default().fg(Color::Black).bg(Color::Cyan));
    frame.render_widget(header_widget, areas[0]);

    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(25),
            Constraint::Percentage(35),
            Constraint::Percentage(40),
        ])
        .split(areas[1]); // placeholder layout to keep ratios on mail page

    match state.ui_page {
        UiPage::Mail => {
            draw_subscriptions(frame, panes[0], state);
            draw_threads(frame, panes[1], state);
            draw_preview(frame, panes[2], state, config);
        }
        UiPage::CodeBrowser => {
            draw_code_browser_page(frame, areas[1], state);
        }
    }

    let shortcuts_text = match state.ui_page {
        UiPage::Mail => {
            "/ search | Tab page | : palette | j/l focus | i/k move | y/n enable | Enter open/toggle"
        }
        UiPage::CodeBrowser => {
            "Tab page | : palette | j/l focus | i/k move/scroll | Enter expand/collapse"
        }
    };
    let footer_sections = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(shortcuts_text.chars().count() as u16),
            Constraint::Min(1),
        ])
        .split(areas[2]);

    let shortcuts =
        Paragraph::new(shortcuts_text).style(Style::default().fg(Color::White).bg(Color::DarkGray));
    frame.render_widget(shortcuts, footer_sections[0]);

    let status_line = format!("status: {}", state.status);
    let status = Paragraph::new(status_line)
        .alignment(Alignment::Right)
        .style(Style::default().fg(Color::White).bg(Color::DarkGray));
    frame.render_widget(status, footer_sections[1]);

    if state.palette.open {
        draw_command_palette(frame, state);
    }
    if state.search.active {
        draw_search_overlay(frame, state);
    }
}

fn draw_code_browser_page(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(area);

    draw_kernel_tree(frame, panes[0], state);
    draw_code_source_preview(frame, panes[1], state);
}

fn draw_subscriptions(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let rows = state.subscription_rows();
    let items: Vec<ListItem> = rows
        .iter()
        .map(|row| ListItem::new(row.text.clone()))
        .collect();
    let selected_row = if rows.is_empty() {
        None
    } else {
        Some(
            state
                .subscription_row_index
                .min(rows.len().saturating_sub(1)),
        )
    };

    let mut list_state = ListState::default();
    list_state.select(selected_row);

    let list = List::new(items)
        .block(panel_block(Pane::Subscriptions, state.focus))
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );

    frame.render_stateful_widget(list, area, &mut list_state);
}

fn draw_kernel_tree(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let block = panel_block_with_title("Kernel Tree", state.code_focus == CodePaneFocus::Tree);
    let items: Vec<ListItem> = if state.kernel_tree_rows.is_empty() {
        vec![ListItem::new(
            "<no files found under configured kernel trees>",
        )]
    } else {
        state
            .kernel_tree_rows
            .iter()
            .map(|row| ListItem::new(row.display_text()))
            .collect()
    };

    let selected_row = if state.kernel_tree_rows.is_empty() {
        None
    } else {
        Some(
            state
                .kernel_tree_row_index
                .min(state.kernel_tree_rows.len().saturating_sub(1)),
        )
    };

    let mut list_state = ListState::default();
    list_state.select(selected_row);

    let list = List::new(items).block(block).highlight_style(
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    );

    frame.render_stateful_widget(list, area, &mut list_state);
}

fn draw_code_source_preview(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let block = panel_block_with_title("Source Preview", state.code_focus == CodePaneFocus::Source);
    let inner_area = block.inner(area);
    frame.render_widget(block, area);
    frame.render_widget(Clear, inner_area);

    let paragraph =
        Paragraph::new(load_code_source_preview(state)).scroll((state.code_preview_scroll, 0));
    frame.render_widget(paragraph, inner_area);
}

fn subscription_line(item: &SubscriptionItem) -> String {
    let marker = if item.enabled { "y" } else { "n" };
    format!(
        "[{marker}] {} - {} ({})",
        item.mailbox,
        item.description,
        lore_archive_url(&item.mailbox)
    )
}

fn default_kernel_tree_expanded_paths(root_paths: &[PathBuf]) -> HashSet<PathBuf> {
    root_paths
        .iter()
        .filter(|path| path.exists() && path.is_dir())
        .cloned()
        .collect()
}

fn build_kernel_tree_rows(
    root_paths: &[PathBuf],
    expanded_paths: &HashSet<PathBuf>,
) -> Vec<KernelTreeRow> {
    let mut rows = Vec::new();
    for root in root_paths {
        if rows.len() >= KERNEL_TREE_MAX_ROWS {
            break;
        }

        if !root.exists() {
            rows.push(KernelTreeRow {
                path: root.clone(),
                name: String::new(),
                depth: 0,
                kind: KernelTreeRowKind::MissingPath,
                expandable: false,
                expanded: false,
            });
            continue;
        }

        if !root.is_dir() {
            rows.push(KernelTreeRow {
                path: root.clone(),
                name: root
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_else(|| root.display().to_string()),
                depth: 0,
                kind: KernelTreeRowKind::RootFile,
                expandable: false,
                expanded: false,
            });
            continue;
        }

        let has_children = has_child_entries(root);
        let is_expanded = expanded_paths.contains(root);
        rows.push(KernelTreeRow {
            path: root.clone(),
            name: root
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| root.display().to_string()),
            depth: 0,
            kind: KernelTreeRowKind::RootDirectory,
            expandable: has_children,
            expanded: has_children && is_expanded,
        });

        if has_children && is_expanded {
            append_kernel_tree_rows(root, 1, expanded_paths, &mut rows);
        }
    }
    rows
}

fn append_kernel_tree_rows(
    directory: &Path,
    depth: usize,
    expanded_paths: &HashSet<PathBuf>,
    rows: &mut Vec<KernelTreeRow>,
) {
    if rows.len() >= KERNEL_TREE_MAX_ROWS {
        return;
    }

    let children = child_entries(directory);
    for child in children {
        if rows.len() >= KERNEL_TREE_MAX_ROWS {
            break;
        }

        if child.is_dir() {
            let has_children = has_child_entries(&child);
            let is_expanded = expanded_paths.contains(&child);
            let name = child
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| child.display().to_string());
            rows.push(KernelTreeRow {
                path: child.clone(),
                name,
                depth,
                kind: KernelTreeRowKind::Directory,
                expandable: has_children,
                expanded: has_children && is_expanded,
            });

            if has_children && is_expanded {
                append_kernel_tree_rows(&child, depth + 1, expanded_paths, rows);
            }
            continue;
        }

        let name = child
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| child.display().to_string());
        rows.push(KernelTreeRow {
            path: child,
            name,
            depth,
            kind: KernelTreeRowKind::File,
            expandable: false,
            expanded: false,
        });
    }
}

fn child_entries(path: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let mut files = Vec::new();
    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(_) => return Vec::new(),
    };
    for entry in entries.flatten() {
        let child = entry.path();
        if child.is_dir() {
            dirs.push(child);
        } else if child.is_file() {
            files.push(child);
        }
    }
    dirs.sort_by(|left, right| {
        left.file_name()
            .map(|name| name.to_string_lossy().to_string())
            .cmp(
                &right
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string()),
            )
    });
    files.sort_by(|left, right| {
        left.file_name()
            .map(|name| name.to_string_lossy().to_string())
            .cmp(
                &right
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string()),
            )
    });
    dirs.extend(files);
    dirs
}

fn has_child_entries(path: &Path) -> bool {
    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(_) => return false,
    };
    for entry in entries.flatten() {
        let child = entry.path();
        if child.is_dir() || child.is_file() {
            return true;
        }
    }
    false
}

fn default_subscriptions(
    default_mailbox: &str,
    enabled_mailboxes: &HashSet<String>,
    active_mailbox: Option<&str>,
) -> Vec<SubscriptionItem> {
    let default_enabled = false;
    let mut items: Vec<SubscriptionItem> = VGER_SUBSCRIPTIONS
        .iter()
        .map(|entry| SubscriptionItem {
            mailbox: entry.mailbox.to_string(),
            description: entry.description.to_string(),
            enabled: if default_enabled {
                entry.mailbox == default_mailbox
            } else {
                enabled_mailboxes.contains(entry.mailbox)
            },
        })
        .collect();

    if items.iter().all(|item| item.mailbox != default_mailbox) {
        items.insert(
            0,
            SubscriptionItem {
                mailbox: default_mailbox.to_string(),
                description: mailbox_description(default_mailbox),
                enabled: default_enabled || enabled_mailboxes.contains(default_mailbox),
            },
        );
    }

    for mailbox in enabled_mailboxes {
        if items.iter().any(|item| item.mailbox == *mailbox) {
            continue;
        }
        items.push(SubscriptionItem {
            mailbox: mailbox.clone(),
            description: mailbox_description(mailbox),
            enabled: true,
        });
    }

    if let Some(mailbox) = active_mailbox
        && !mailbox.is_empty()
        && items.iter().all(|item| item.mailbox != mailbox)
    {
        items.push(SubscriptionItem {
            mailbox: mailbox.to_string(),
            description: mailbox_description(mailbox),
            enabled: enabled_mailboxes.contains(mailbox),
        });
    }

    items.sort_by(|left, right| {
        right
            .enabled
            .cmp(&left.enabled)
            .then_with(|| left.mailbox.cmp(&right.mailbox))
    });

    items
}

fn draw_threads(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let mut visible_count_by_thread: HashMap<i64, usize> = HashMap::new();
    for index in &state.filtered_thread_indices {
        if let Some(row) = state.threads.get(*index) {
            *visible_count_by_thread.entry(row.thread_id).or_insert(0) += 1;
        }
    }

    let mut items: Vec<ListItem> = Vec::new();
    let mut selected = None;
    let mut previous_thread_id: Option<i64> = None;
    for (position, index) in state.filtered_thread_indices.iter().enumerate() {
        let Some(row) = state.threads.get(*index) else {
            continue;
        };

        if previous_thread_id != Some(row.thread_id) {
            previous_thread_id = Some(row.thread_id);
            let visible_count = visible_count_by_thread
                .get(&row.thread_id)
                .copied()
                .unwrap_or(1);
            items.push(
                ListItem::new(thread_group_line(row.thread_id, visible_count)).style(
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD),
                ),
            );
        }

        if position == state.thread_index {
            selected = Some(items.len());
        }
        items.push(ListItem::new(thread_line(row)));
    }

    let mut list_state = ListState::default();
    list_state.select(selected);

    let list = List::new(items)
        .block(panel_block(Pane::Threads, state.focus))
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );

    frame.render_stateful_widget(list, area, &mut list_state);
}

fn thread_group_line(thread_id: i64, visible_count: usize) -> String {
    let noun = if visible_count == 1 { "msg" } else { "msgs" };
    format!("Thread {thread_id} ({visible_count} {noun})")
}

fn thread_line(row: &ThreadRow) -> String {
    let indent = "  ".repeat(row.depth as usize);
    let subject = if row.subject.trim().is_empty() {
        "(no subject)"
    } else {
        row.subject.trim()
    };
    format!("{indent}{subject}  [{}]", row.from_addr)
}

fn draw_preview(frame: &mut Frame<'_>, area: Rect, state: &AppState, config: &RuntimeConfig) {
    let preview = if let Some(thread) = state.selected_thread() {
        let subject = if thread.subject.trim().is_empty() {
            "(no subject)"
        } else {
            thread.subject.trim()
        };
        format!(
            "Subject: {}\n\n{}",
            subject,
            load_mail_body_preview(thread.raw_path.as_ref()),
        )
    } else {
        format!(
            "No synced thread data\n\nRun:\n  courier sync --fixture-dir <DIR>\n\nConfig: {}\nDatabase: {}",
            config.config_path.display(),
            config.database_path.display(),
        )
    };

    let block = panel_block(Pane::Preview, state.focus);
    let inner_area = block.inner(area);
    frame.render_widget(block, area);
    frame.render_widget(Clear, inner_area);

    let paragraph = Paragraph::new(preview)
        .scroll((state.preview_scroll, 0))
        .wrap(Wrap { trim: false });

    frame.render_widget(paragraph, inner_area);
}

fn load_code_source_preview(state: &AppState) -> String {
    if !state.supports_code_browser() {
        return "No kernel tree configured.\n\nSet [kernel].tree or [kernel].trees in config."
            .to_string();
    }

    let Some(row) = state.selected_kernel_tree_row() else {
        return "<kernel tree is empty>".to_string();
    };

    match row.kind {
        KernelTreeRowKind::File | KernelTreeRowKind::RootFile => {
            load_source_file_preview(&row.path)
        }
        KernelTreeRowKind::MissingPath => {
            format!("<missing path>\n\n{}", row.path.display())
        }
        KernelTreeRowKind::RootDirectory | KernelTreeRowKind::Directory => format!(
            "Directory: {}\n\nSelect a file in the tree to preview source content.",
            row.path.display()
        ),
    }
}

fn load_source_file_preview(path: &Path) -> String {
    let content = match fs::read(path) {
        Ok(value) => value,
        Err(error) => return format!("<failed to read {}: {}>", path.display(), error),
    };

    let truncated_by_bytes = content.len() > CODE_PREVIEW_MAX_BYTES;
    let content_slice = if truncated_by_bytes {
        &content[..CODE_PREVIEW_MAX_BYTES]
    } else {
        content.as_slice()
    };

    let text = String::from_utf8_lossy(content_slice)
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let sanitized = sanitize_source_preview_text(&text);
    let mut source_lines = sanitized.lines();
    let mut lines = Vec::new();
    for line in source_lines.by_ref().take(CODE_PREVIEW_MAX_LINES) {
        lines.push(line);
    }
    let truncated_by_lines = source_lines.next().is_some();

    let body = if lines.is_empty() {
        "<empty file>".to_string()
    } else {
        lines.join("\n")
    };

    let mut preview = format!("File: {}\n\n{}", path.display(), body);
    if truncated_by_bytes || truncated_by_lines {
        preview.push_str("\n\n<truncated preview>");
    }
    preview
}

fn load_mail_body_preview(path: Option<&PathBuf>) -> String {
    let Some(path) = path else {
        return "<raw mail file unavailable>".to_string();
    };

    let content = match fs::read(path) {
        Ok(value) => value,
        Err(error) => return format!("<failed to read {}: {}>", path.display(), error),
    };

    extract_mail_body_preview(&content)
}

fn extract_mail_body_preview(raw: &[u8]) -> String {
    let body_start = find_subslice(raw, b"\r\n\r\n")
        .map(|index| index + 4)
        .or_else(|| find_subslice(raw, b"\n\n").map(|index| index + 2))
        .unwrap_or(0);

    let body = &raw[body_start..];
    let text = String::from_utf8_lossy(body).replace("\r\n", "\n");
    let stripped = strip_first_mime_part_headers(&text);

    let sanitized = sanitize_preview_text(&stripped);

    let lines: Vec<&str> = sanitized
        .lines()
        .map(str::trim_end)
        .skip_while(|line| line.trim().is_empty())
        .take(80)
        .collect();

    let snippet = lines.join("\n");
    if snippet.trim().is_empty() {
        "<empty mail body>".to_string()
    } else {
        snippet
    }
}

fn sanitize_preview_text(input: &str) -> String {
    let mut sanitized = String::with_capacity(input.len());
    for character in input.chars() {
        match character {
            '\n' => sanitized.push('\n'),
            '\t' => sanitized.push_str(PREVIEW_TAB_SPACES),
            _ if character.is_control() => {}
            _ => sanitized.push(character),
        }
    }
    sanitized
}

fn sanitize_source_preview_text(input: &str) -> String {
    let mut sanitized = String::with_capacity(input.len());
    for character in input.chars() {
        match character {
            '\n' | '\t' => sanitized.push(character),
            _ if character.is_control() => {}
            _ => sanitized.push(character),
        }
    }
    sanitized
}

fn strip_first_mime_part_headers(body: &str) -> String {
    let lines: Vec<&str> = body.lines().collect();
    let Some(first_non_empty_index) = lines.iter().position(|line| !line.trim().is_empty()) else {
        return String::new();
    };

    let boundary = lines[first_non_empty_index].trim();
    if !boundary.starts_with("--") {
        return body.to_string();
    }

    let mut cursor = first_non_empty_index + 1;
    while cursor < lines.len() && !lines[cursor].trim().is_empty() {
        cursor += 1;
    }

    if cursor >= lines.len() {
        return body.to_string();
    }

    let content_start = cursor + 1;
    let mut content = Vec::new();
    let closing_boundary = format!("{boundary}--");
    for line in &lines[content_start..] {
        let trimmed = line.trim();
        if trimmed == boundary || trimmed == closing_boundary {
            break;
        }
        content.push(line.trim_end());
    }

    if content.is_empty() {
        body.to_string()
    } else {
        content.join("\n")
    }
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }

    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn draw_search_overlay(frame: &mut Frame<'_>, state: &AppState) {
    let area = centered_rect(70, 20, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title("Search Threads")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::LightBlue));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
        ])
        .split(inner);

    let input = Paragraph::new(format!("> {}", state.search.input));
    frame.render_widget(input, sections[0]);

    let hint = Paragraph::new("Enter: apply and locate first match  Esc: cancel");
    frame.render_widget(hint, sections[1]);

    let current = if state.search.applied_query.is_empty() {
        "Current filter: <none>".to_string()
    } else {
        format!("Current filter: {}", state.search.applied_query)
    };
    frame.render_widget(Paragraph::new(current), sections[2]);
}

fn draw_command_palette(frame: &mut Frame<'_>, state: &AppState) {
    let area = centered_rect(70, 28, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title("Command Palette")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::LightGreen));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
        ])
        .split(inner);

    let input = Paragraph::new(format!("> {}", state.palette.input));
    frame.render_widget(input, sections[0]);

    let hints = Paragraph::new(
        "Tab: complete (press twice to list args)  Enter: execute  Esc: close  Ctrl+` or F1: toggle  : opens",
    );
    frame.render_widget(hints, sections[1]);

    let suggestions = palette_overlay_suggestions(state);
    let candidate_title = if suggestions.is_empty() {
        "Candidates: <none>"
    } else if state.palette.show_suggestions {
        "Completion Candidates"
    } else {
        "Command Candidates"
    };
    let candidate_header = Paragraph::new(candidate_title);
    frame.render_widget(candidate_header, sections[2]);

    let items: Vec<ListItem> = suggestions
        .iter()
        .take(8)
        .map(|suggestion| match suggestion.description.as_deref() {
            Some(description) if !description.is_empty() => {
                ListItem::new(format!("{} - {}", suggestion.value, description))
            }
            _ => ListItem::new(suggestion.value.clone()),
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Gray)),
    );
    frame.render_widget(list, sections[3]);
}

fn palette_overlay_suggestions(state: &AppState) -> Vec<PaletteSuggestion> {
    if state.palette.show_suggestions && !state.palette.suggestions.is_empty() {
        return state.palette.suggestions.clone();
    }

    matching_commands(&state.palette.input)
        .into_iter()
        .map(|command| PaletteSuggestion {
            value: command.name.to_string(),
            description: Some(command.description.to_string()),
        })
        .collect()
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1]);

    horizontal[1]
}

fn matching_commands(input: &str) -> Vec<&'static PaletteCommand> {
    let query = input.trim().to_ascii_lowercase();
    let mut matched: Vec<(u8, &PaletteCommand)> = Vec::new();

    for command in PALETTE_COMMANDS {
        if query.is_empty() || command.name.starts_with(&query) {
            matched.push((0, command));
            continue;
        }

        let description = command.description.to_ascii_lowercase();
        if command.name.contains(&query) || description.contains(&query) {
            matched.push((1, command));
        }
    }

    matched.sort_by_key(|(score, command)| (*score, command.name));
    matched.into_iter().map(|(_, command)| command).collect()
}

fn panel_block(panel: Pane, focus: Pane) -> Block<'static> {
    let is_focused = panel == focus;
    panel_block_with_title(panel.title(), is_focused)
}

fn panel_block_with_title(title: &str, is_focused: bool) -> Block<'static> {
    let decorated_title = if is_focused {
        format!("{title} *")
    } else {
        title.to_string()
    };

    let border_style = if is_focused {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Gray)
    };

    Block::default()
        .title(decorated_title)
        .borders(Borders::ALL)
        .border_style(border_style)
}

struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let mut stdout = io::stdout();
        let _ = execute!(stdout, LeaveAlternateScreen);
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use crate::infra::bootstrap::BootstrapState;
    use crate::infra::config::RuntimeConfig;
    use crate::infra::db::DatabaseState;
    use crate::infra::mail_store::ThreadRow;

    use super::{
        AppState, LoopAction, Pane, UiPage, draw, extract_mail_body_preview, handle_key_event,
        is_palette_open_shortcut, is_palette_toggle, load_source_file_preview, matching_commands,
    };

    fn temp_dir(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("courier-ui-{label}-{nonce}"));
        fs::create_dir_all(&path).expect("create temp dir");
        path
    }

    fn sample_thread(subject: &str, message_id: &str, depth: u16) -> ThreadRow {
        ThreadRow {
            thread_id: 1,
            mail_id: 1,
            depth,
            subject: subject.to_string(),
            from_addr: "alice@example.com".to_string(),
            message_id: message_id.to_string(),
            in_reply_to: None,
            date: None,
            raw_path: None,
        }
    }

    fn sample_thread_with_raw(
        subject: &str,
        message_id: &str,
        depth: u16,
        raw_path: PathBuf,
    ) -> ThreadRow {
        ThreadRow {
            thread_id: 1,
            mail_id: 1,
            depth,
            subject: subject.to_string(),
            from_addr: "alice@example.com".to_string(),
            message_id: message_id.to_string(),
            in_reply_to: None,
            date: None,
            raw_path: Some(raw_path),
        }
    }

    fn sample_thread_in_thread(
        thread_id: i64,
        mail_id: i64,
        subject: &str,
        message_id: &str,
        depth: u16,
    ) -> ThreadRow {
        ThreadRow {
            thread_id,
            mail_id,
            depth,
            subject: subject.to_string(),
            from_addr: "alice@example.com".to_string(),
            message_id: message_id.to_string(),
            in_reply_to: None,
            date: None,
            raw_path: None,
        }
    }

    fn test_runtime() -> RuntimeConfig {
        let root = PathBuf::from("/tmp/courier-ui-test");
        RuntimeConfig {
            config_path: root.join("config.toml"),
            data_dir: root.join("data"),
            database_path: root.join("data/courier.db"),
            raw_mail_dir: root.join("data/raw"),
            patch_dir: root.join("data/patches"),
            log_dir: root.join("data/logs"),
            b4_path: None,
            log_filter: "info".to_string(),
            imap_mailbox: "inbox".to_string(),
            lore_base_url: "https://lore.kernel.org".to_string(),
            kernel_trees: Vec::new(),
        }
    }

    fn test_runtime_with_kernel_tree(tree: PathBuf) -> RuntimeConfig {
        let mut runtime = test_runtime();
        runtime.kernel_trees = vec![tree];
        runtime
    }

    fn test_bootstrap(runtime: &RuntimeConfig) -> BootstrapState {
        BootstrapState {
            db: DatabaseState {
                path: runtime.database_path.clone(),
                schema_version: 1,
                created: false,
                applied_migrations: vec![],
            },
        }
    }

    #[test]
    fn empty_query_returns_all_palette_commands() {
        let all = matching_commands("");
        assert_eq!(all.len(), 5);
        assert_eq!(all[0].name, "config");
        assert_eq!(all[1].name, "exit");
        assert_eq!(all[2].name, "help");
        assert_eq!(all[3].name, "quit");
        assert_eq!(all[4].name, "sync");
    }

    #[test]
    fn prefix_matches_rank_before_fuzzy_matches() {
        let commands = matching_commands("ex");
        assert_eq!(commands[0].name, "exit");
    }

    #[test]
    fn command_palette_quit_exits_application() {
        let mut state = AppState::new(vec![], test_runtime());
        state.palette.open = true;
        state.palette.input = "quit".to_string();

        let action = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        );
        assert!(matches!(action, LoopAction::Exit));
    }

    #[test]
    fn config_palette_get_and_set_roundtrip() {
        let root = temp_dir("palette-config");
        let config_path = root.join("courier-config.toml");
        fs::write(
            &config_path,
            r#"
[source]
mailbox = "inbox"
"#,
        )
        .expect("write config file");

        let mut runtime = test_runtime();
        runtime.config_path = config_path.clone();
        let mut state = AppState::new(vec![], runtime);

        state.palette.open = true;
        state.palette.input = "config get source.mailbox".to_string();
        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        );
        assert!(state.status.contains("source.mailbox"));
        assert!(state.status.contains("inbox"));

        state.palette.open = true;
        state.palette.input = "config set source.mailbox io-uring".to_string();
        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        );
        assert!(state.status.contains("config updated"));
        assert_eq!(state.runtime.imap_mailbox, "io-uring");

        let persisted = fs::read_to_string(&config_path).expect("read config file");
        assert!(persisted.contains("mailbox = \"io-uring\""));

        state.palette.open = true;
        state.palette.input = "config get source.mailbox".to_string();
        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        );
        assert!(state.status.contains("io-uring"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn ctrl_backtick_toggles_command_palette() {
        let key = KeyEvent::new(KeyCode::Char('`'), KeyModifiers::CONTROL);
        assert!(is_palette_toggle(key));
    }

    #[test]
    fn colon_opens_command_palette() {
        let mut state = AppState::new(vec![], test_runtime());

        let key = KeyEvent::new(KeyCode::Char(':'), KeyModifiers::SHIFT);
        assert!(is_palette_open_shortcut(key));

        let action = handle_key_event(&mut state, key);
        assert!(matches!(action, LoopAction::Continue));
        assert!(state.palette.open);
    }

    #[test]
    fn palette_tab_completes_top_level_command() {
        let mut state = AppState::new(vec![], test_runtime());
        state.palette.open = true;
        state.palette.input = "co".to_string();

        let _ = handle_key_event(&mut state, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));

        assert_eq!(state.palette.input, "config ");
    }

    #[test]
    fn palette_tab_completes_config_subcommand_and_key() {
        let mut state = AppState::new(vec![], test_runtime());
        state.palette.open = true;
        state.palette.input = "config g".to_string();
        let _ = handle_key_event(&mut state, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(state.palette.input, "config get ");

        state.palette.input = "config get source.m".to_string();
        let _ = handle_key_event(&mut state, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(state.palette.input, "config get source.mailbox ");
    }

    #[test]
    fn palette_tab_completes_sync_mailbox() {
        let mut state = AppState::new(vec![], test_runtime());
        state.palette.open = true;
        state.palette.input = "sync bp".to_string();

        let _ = handle_key_event(&mut state, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));

        assert_eq!(state.palette.input, "sync bpf ");
    }

    #[test]
    fn palette_double_tab_lists_config_arguments() {
        let mut state = AppState::new(vec![], test_runtime());
        state.palette.open = true;
        state.palette.input = "config".to_string();

        let _ = handle_key_event(&mut state, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(state.palette.input, "config ");
        assert!(!state.palette.show_suggestions);

        let _ = handle_key_event(&mut state, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert!(state.palette.show_suggestions);
        let values: Vec<String> = state
            .palette
            .suggestions
            .iter()
            .map(|item| item.value.clone())
            .collect();
        assert!(values.contains(&"show".to_string()));
        assert!(values.contains(&"get".to_string()));
        assert!(values.contains(&"set".to_string()));
        assert!(values.contains(&"help".to_string()));
    }

    #[test]
    fn tab_toggles_between_mail_page_and_code_browser_page() {
        let tree_root = temp_dir("kernel-tree-tab");
        fs::create_dir_all(tree_root.join("io_uring")).expect("create kernel dir");
        let runtime = test_runtime_with_kernel_tree(tree_root.clone());
        let mut state = AppState::new(vec![], runtime);

        assert!(matches!(state.ui_page, UiPage::Mail));
        let _ = handle_key_event(&mut state, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert!(matches!(state.ui_page, UiPage::CodeBrowser));

        let _ = handle_key_event(&mut state, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert!(matches!(state.ui_page, UiPage::Mail));

        let _ = fs::remove_dir_all(tree_root);
    }

    #[test]
    fn kernel_tree_enter_expands_and_collapses_selected_directory() {
        let tree_root = temp_dir("kernel-tree-expand");
        let dir_a = tree_root.join("a");
        let dir_b = dir_a.join("b");
        let dir_c = tree_root.join("c");
        fs::create_dir_all(&dir_b).expect("create nested directory");
        fs::create_dir_all(&dir_c).expect("create sibling directory");

        let runtime = test_runtime_with_kernel_tree(tree_root.clone());
        let mut state = AppState::new(vec![], runtime);
        let _ = handle_key_event(&mut state, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert!(matches!(state.ui_page, UiPage::CodeBrowser));

        let index_a = state
            .kernel_tree_rows
            .iter()
            .position(|row| row.path == dir_a)
            .expect("directory a row exists");
        state.kernel_tree_row_index = index_a;
        assert!(state.kernel_tree_rows[index_a].expandable);
        assert!(!state.kernel_tree_rows.iter().any(|row| row.path == dir_b));

        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        );
        assert!(state.kernel_tree_rows.iter().any(|row| row.path == dir_b));

        let index_a_after_expand = state
            .kernel_tree_rows
            .iter()
            .position(|row| row.path == dir_a)
            .expect("directory a row exists after expand");
        state.kernel_tree_row_index = index_a_after_expand;
        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        );
        assert!(!state.kernel_tree_rows.iter().any(|row| row.path == dir_b));

        let _ = fs::remove_dir_all(tree_root);
    }

    #[test]
    fn kernel_tree_lists_files_and_source_preview_preserves_indentation() {
        let tree_root = temp_dir("kernel-tree-files");
        let dir_a = tree_root.join("a");
        let file_path = dir_a.join("demo.c");
        fs::create_dir_all(&dir_a).expect("create directory");
        fs::write(
            &file_path,
            "fn demo() {\n\tif true {\n        return;\n\t}\n}\n",
        )
        .expect("write source file");

        let runtime = test_runtime_with_kernel_tree(tree_root.clone());
        let mut state = AppState::new(vec![], runtime);
        let _ = handle_key_event(&mut state, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert!(matches!(state.ui_page, UiPage::CodeBrowser));

        let index_a = state
            .kernel_tree_rows
            .iter()
            .position(|row| row.path == dir_a)
            .expect("directory a row exists");
        state.kernel_tree_row_index = index_a;
        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        );

        let file_index = state
            .kernel_tree_rows
            .iter()
            .position(|row| row.path == file_path)
            .expect("file row exists");
        state.kernel_tree_row_index = file_index;

        let preview = load_source_file_preview(&file_path);
        assert!(preview.contains("\tif true {"));
        assert!(preview.contains("        return;"));

        let _ = fs::remove_dir_all(tree_root);
    }

    #[test]
    fn enter_on_subscription_opens_threads_without_toggling_enabled_state() {
        let mut state = AppState::new(vec![], test_runtime());
        state.focus = Pane::Subscriptions;
        let initial = state.subscriptions[0].enabled;

        let action = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        );

        assert!(matches!(action, LoopAction::Continue));
        assert_eq!(state.subscriptions[0].enabled, initial);
    }

    #[test]
    fn enter_on_group_header_toggles_expand_and_collapse() {
        let mut state = AppState::new(vec![], test_runtime());
        state.focus = Pane::Subscriptions;
        state.subscription_row_index = 0;

        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        );
        assert!(!state.enabled_group_expanded);
        let rows_after_collapse = state.subscription_rows();
        assert!(rows_after_collapse[0].text.starts_with('▶'));

        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        );
        assert!(state.enabled_group_expanded);
        let rows_after_expand = state.subscription_rows();
        assert!(rows_after_expand[0].text.starts_with('▼'));
    }

    #[test]
    fn first_open_starts_with_all_subscriptions_disabled() {
        let state = AppState::new(vec![], test_runtime());
        assert!(state.subscriptions.iter().all(|item| !item.enabled));
    }

    #[test]
    fn y_and_n_toggle_subscription_and_keep_grouped_sort_order() {
        let mut state = AppState::new(vec![], test_runtime());
        state.focus = Pane::Subscriptions;

        let target_index = state
            .subscriptions
            .iter()
            .position(|item| item.mailbox == "bpf")
            .expect("bpf subscription exists");
        state.subscription_index = target_index;
        state.sync_subscription_row_to_selected_item();

        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE),
        );

        let bpf_after_enable = state
            .subscriptions
            .iter()
            .position(|item| item.mailbox == "bpf")
            .expect("bpf exists after enable");
        assert!(state.subscriptions[bpf_after_enable].enabled);

        let first_disabled = state
            .subscriptions
            .iter()
            .position(|item| !item.enabled)
            .expect("has disabled subscriptions");
        assert!(bpf_after_enable < first_disabled);

        let enabled_group = &state.subscriptions[..first_disabled];
        assert!(
            enabled_group
                .windows(2)
                .all(|pair| pair[0].mailbox <= pair[1].mailbox)
        );

        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE),
        );

        let bpf_after_disable = state
            .subscriptions
            .iter()
            .position(|item| item.mailbox == "bpf")
            .expect("bpf exists after disable");
        assert!(!state.subscriptions[bpf_after_disable].enabled);

        let last_enabled = state.subscriptions.iter().rposition(|item| item.enabled);
        if let Some(last_enabled) = last_enabled {
            assert!(bpf_after_disable > last_enabled);

            let disabled_group = &state.subscriptions[last_enabled + 1..];
            assert!(
                disabled_group
                    .windows(2)
                    .all(|pair| pair[0].mailbox <= pair[1].mailbox)
            );
        } else {
            assert!(state.subscriptions.iter().all(|item| !item.enabled));
            assert!(
                state
                    .subscriptions
                    .windows(2)
                    .all(|pair| pair[0].mailbox <= pair[1].mailbox)
            );
        }
    }

    #[test]
    fn slash_opens_search_and_filters_threads() {
        let mut state = AppState::new(
            vec![
                sample_thread("[PATCH] mm cleanup", "a@example.com", 0),
                sample_thread("[PATCH] net fix", "b@example.com", 0),
            ],
            test_runtime(),
        );

        let action_search = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE),
        );
        assert!(matches!(action_search, LoopAction::Continue));
        assert!(state.search.active);

        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE),
        );
        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE),
        );
        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE),
        );
        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        );

        assert!(!state.search.active);
        assert_eq!(state.filtered_thread_indices.len(), 1);
        let selected = state.selected_thread().expect("selected thread");
        assert_eq!(selected.message_id, "b@example.com");
    }

    #[test]
    fn jl_focus_and_ik_move_selection() {
        let mut state = AppState::new(
            vec![
                sample_thread("t0", "a@example.com", 0),
                sample_thread("t1", "b@example.com", 1),
            ],
            test_runtime(),
        );
        state.subscription_index = 1;

        let action_l = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE),
        );
        assert!(matches!(action_l, LoopAction::Continue));
        assert!(matches!(state.focus, Pane::Threads));

        let action_i = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE),
        );
        assert!(matches!(action_i, LoopAction::Continue));
        assert_eq!(state.thread_index, 0);

        let action_k = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE),
        );
        assert!(matches!(action_k, LoopAction::Continue));
        assert_eq!(state.thread_index, 1);

        let action_j = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
        );
        assert!(matches!(action_j, LoopAction::Continue));
        assert!(matches!(state.focus, Pane::Subscriptions));
    }

    #[test]
    fn preview_hides_rfc_headers_and_keeps_body() {
        let raw = b"Message-ID: <a@example.com>\r\nSubject: test\r\nFrom: a@example.com\r\n\r\nhello\nworld\n";
        let preview = extract_mail_body_preview(raw);
        assert!(!preview.contains("Message-ID:"));
        assert!(!preview.contains("Subject: test"));
        assert!(preview.contains("hello"));
        assert!(preview.contains("world"));
    }

    #[test]
    fn preview_skips_first_mime_part_headers() {
        let raw = b"Content-Type: multipart/alternative; boundary=\"abc\"\r\n\r\n--abc\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: 8bit\r\n\r\nplain body line\r\n--abc--\r\n";
        let preview = extract_mail_body_preview(raw);
        assert!(!preview.contains("Content-Transfer-Encoding"));
        assert!(preview.contains("plain body line"));
    }

    #[test]
    fn preview_strips_control_characters() {
        let raw =
            b"Message-ID: <a@example.com>\r\nSubject: test\r\n\r\nline1\x1b[31m\x07\nline2\tok\r\n";
        let preview = extract_mail_body_preview(raw);
        assert!(!preview.contains('\u{001b}'));
        assert!(!preview.contains('\u{0007}'));
        assert!(!preview.contains('\t'));
        assert!(preview.contains("line1"));
        assert!(preview.contains("line2    ok"));
    }

    #[test]
    fn preview_redraw_clears_stale_characters_after_thread_switch() {
        let root = temp_dir("preview-clear");
        let first_raw = root.join("first.eml");
        let second_raw = root.join("second.eml");

        fs::write(
            &first_raw,
            b"Message-ID: <first@example.com>\r\nSubject: first\r\nFrom: a@example.com\r\n\r\nSTALE_PREVIEW_TOKEN_123456\nlong line that should disappear\n",
        )
        .expect("write first raw mail");
        fs::write(
            &second_raw,
            b"Message-ID: <second@example.com>\r\nSubject: second\r\nFrom: b@example.com\r\n\r\nshort\n",
        )
        .expect("write second raw mail");

        let runtime = test_runtime();
        let bootstrap = test_bootstrap(&runtime);
        let mut state = AppState::new(
            vec![
                sample_thread_with_raw("first", "first@example.com", 0, first_raw.clone()),
                sample_thread_with_raw("second", "second@example.com", 0, second_raw.clone()),
            ],
            runtime.clone(),
        );
        state.focus = Pane::Threads;

        let mut terminal = Terminal::new(TestBackend::new(120, 30)).expect("create test terminal");
        terminal
            .draw(|frame| draw(frame, &state, &runtime, &bootstrap))
            .expect("draw first frame");
        let first_frame = format!("{}", terminal.backend());
        assert!(first_frame.contains("STALE_PREVIEW_TOKEN_123456"));

        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE),
        );
        terminal
            .draw(|frame| draw(frame, &state, &runtime, &bootstrap))
            .expect("draw second frame");
        let second_frame = format!("{}", terminal.backend());
        assert!(!second_frame.contains("STALE_PREVIEW_TOKEN_123456"));
        assert!(second_frame.contains("short"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn preview_render_preserves_code_indentation() {
        let root = temp_dir("preview-indent");
        let raw = root.join("indent.eml");

        fs::write(
            &raw,
            b"Message-ID: <indent@example.com>\r\nSubject: indent\r\nFrom: a@example.com\r\n\r\nfn demo() {\n\tif true {\n        return;\n\t}\n}\n",
        )
        .expect("write raw mail");

        let runtime = test_runtime();
        let bootstrap = test_bootstrap(&runtime);
        let state = AppState::new(
            vec![sample_thread_with_raw(
                "indent",
                "indent@example.com",
                0,
                raw.clone(),
            )],
            runtime.clone(),
        );

        let mut terminal = Terminal::new(TestBackend::new(120, 30)).expect("create test terminal");
        terminal
            .draw(|frame| draw(frame, &state, &runtime, &bootstrap))
            .expect("draw frame");
        let rendered = format!("{}", terminal.backend());
        assert!(rendered.contains("    if true {"));
        assert!(rendered.contains("        return;"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn threads_panel_renders_thread_group_headers() {
        let runtime = test_runtime();
        let bootstrap = test_bootstrap(&runtime);
        let mut state = AppState::new(
            vec![
                sample_thread_in_thread(100, 1, "thread a root", "a-root@example.com", 0),
                sample_thread_in_thread(100, 2, "thread a reply", "a-reply@example.com", 1),
                sample_thread_in_thread(200, 3, "thread b root", "b-root@example.com", 0),
            ],
            runtime.clone(),
        );
        state.focus = Pane::Threads;

        let mut terminal = Terminal::new(TestBackend::new(120, 30)).expect("create test terminal");
        terminal
            .draw(|frame| draw(frame, &state, &runtime, &bootstrap))
            .expect("draw frame");
        let rendered = format!("{}", terminal.backend());
        assert!(rendered.contains("Thread 100 (2 msgs)"));
        assert!(rendered.contains("Thread 200 (1 msg)"));
        assert!(rendered.contains("thread a root"));
        assert!(rendered.contains("thread b root"));
    }
}
