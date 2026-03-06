use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::io::{self, Stdout};
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use crate::app::patch as patch_worker;
use crate::app::sync as sync_worker;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Text;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::{Frame, Terminal};
use toml::Value as TomlValue;
use toml::value::Table as TomlTable;

use crate::domain::subscriptions::VGER_SUBSCRIPTIONS;
use crate::infra::bootstrap::BootstrapState;
use crate::infra::config::{IMAP_INBOX_MAILBOX, RuntimeConfig};
use crate::infra::error::{CourierError, ErrorCode, Result};
use crate::infra::mail_store::{self, ThreadRow};
use crate::infra::ui_state::{self, UiState};

mod preview;

use preview::load_mail_preview;
#[cfg(test)]
use preview::{extract_mail_body_preview, extract_mail_preview};

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

#[derive(Debug, Clone)]
struct LocalCommandResult {
    command: String,
    cwd: PathBuf,
    exit_code: String,
    output: String,
}

#[derive(Debug, Clone)]
struct LastApplySnapshot {
    thread_id: i64,
    before_head: String,
    after_head: String,
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
        name: "restart",
        description: "Restart TUI with startup config",
    },
    PaletteCommand {
        name: "sync",
        description: "Sync mailbox now",
    },
    PaletteCommand {
        name: "config",
        description: "Open visual config editor or update runtime config",
    },
    PaletteCommand {
        name: "vim",
        description: "Open selected source file in external vim",
    },
];

const PALETTE_SYNC_RECONNECT_ATTEMPTS: u8 = 3;
const PREVIEW_TAB_SPACES: &str = "    ";
const PREVIEW_RECIPIENT_PREVIEW_LIMIT: usize = 2;
const PREVIEW_PANE_FIXED_WIDTH: u16 = 80;
const THREAD_LINE_MAX_CHARS: usize = 120;
const KERNEL_TREE_MAX_ROWS: usize = 2048;
const CODE_PREVIEW_MAX_BYTES: usize = 256 * 1024;
const CODE_PREVIEW_MAX_LINES: usize = 800;
const MY_INBOX_LABEL: &str = "My Inbox";
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
    "imap.email",
    "imap.user",
    "imap.pass",
    "imap.server",
    "imap.serverport",
    "imap.encryption",
    "imap.proxy",
    "source.lore_base_url",
    "ui.startup_sync",
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
    "imap.email",
    "imap.user",
    "imap.pass",
    "imap.server",
    "imap.serverport",
    "imap.encryption",
    "imap.proxy",
    "source.lore_base_url",
    "ui.startup_sync",
    "kernel.tree",
    "kernel.trees",
];
const CONFIG_EDITOR_FIELDS: &[ConfigEditorField] = &[
    ConfigEditorField {
        key: "source.mailbox",
        description: "Default lore mailbox used when sync runs without an explicit mailbox.",
    },
    ConfigEditorField {
        key: "source.lore_base_url",
        description: "Base URL used for lore links and message lookups.",
    },
    ConfigEditorField {
        key: "ui.startup_sync",
        description: "Whether enabled subscriptions start syncing automatically after TUI launch.",
    },
    ConfigEditorField {
        key: "logging.filter",
        description: "Tracing/logging filter level for Courier runtime logs.",
    },
    ConfigEditorField {
        key: "logging.dir",
        description: "Directory where Courier writes runtime log files.",
    },
    ConfigEditorField {
        key: "storage.data_dir",
        description: "Runtime data root used for db, mail cache, patches and UI state defaults.",
    },
    ConfigEditorField {
        key: "storage.database",
        description: "SQLite database path used for synced mail and patch metadata.",
    },
    ConfigEditorField {
        key: "storage.raw_mail_dir",
        description: "Directory where raw downloaded .eml files are stored.",
    },
    ConfigEditorField {
        key: "storage.patch_dir",
        description: "Directory where downloaded patch files are written.",
    },
    ConfigEditorField {
        key: "b4.path",
        description: "Optional explicit path to the b4 executable or wrapper script.",
    },
    ConfigEditorField {
        key: "imap.email",
        description: "Self email address for matching your own mail; also used as login when imap.user is omitted.",
    },
    ConfigEditorField {
        key: "imap.user",
        description: "IMAP login account. Gmail usually expects the full email address.",
    },
    ConfigEditorField {
        key: "imap.pass",
        description: "IMAP login password or app password.",
    },
    ConfigEditorField {
        key: "imap.server",
        description: "IMAP server host name.",
    },
    ConfigEditorField {
        key: "imap.serverport",
        description: "IMAP server port number.",
    },
    ConfigEditorField {
        key: "imap.encryption",
        description: "Connection security mode. Gmail 993 uses ssl/tls here.",
    },
    ConfigEditorField {
        key: "imap.proxy",
        description: "Optional proxy URL for IMAP. Supports http://, socks5:// and socks5h://.",
    },
    ConfigEditorField {
        key: "kernel.tree",
        description: "Single kernel tree root shown in the code browser pane.",
    },
    ConfigEditorField {
        key: "kernel.trees",
        description: "Multiple kernel tree roots, written as a TOML array of paths.",
    },
];
const CODE_EDIT_ENTRY_HINT: &str = "select a source file in Source pane, then press e";
const EXTERNAL_EDITOR_ENTRY_HINT: &str = "select a source file in Source pane, then press E";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ExternalEditorProcessResult {
    success: bool,
    exit_code: Option<i32>,
}

type ExternalEditorRunner =
    fn(&str, &Path) -> std::result::Result<ExternalEditorProcessResult, String>;

#[derive(Debug, Clone)]
enum StartupSyncEvent {
    MailboxStarted {
        mailbox: String,
        index: usize,
        total: usize,
    },
    MailboxFinished {
        mailbox: String,
        fetched: usize,
        inserted: usize,
        updated: usize,
    },
    MailboxFailed {
        mailbox: String,
        error: String,
    },
    WorkerCompleted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartupSyncMailboxStatus {
    Pending,
    InFlight,
    Finished,
    Failed,
}

impl StartupSyncMailboxStatus {
    fn ui_suffix(self) -> &'static str {
        match self {
            Self::Pending => " [queued]",
            Self::InFlight => " [sync]",
            Self::Finished => " [done]",
            Self::Failed => " [failed]",
        }
    }

    fn log_label(self) -> &'static str {
        match self {
            Self::Pending => "queued",
            Self::InFlight => "syncing",
            Self::Finished => "done",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug)]
struct StartupSyncState {
    receiver: Receiver<StartupSyncEvent>,
    mailbox_order: Vec<String>,
    mailboxes: HashMap<String, StartupSyncMailboxStatus>,
    total: usize,
    completed: usize,
    succeeded: usize,
    failed: usize,
}

impl StartupSyncState {
    fn pending_count(&self) -> usize {
        self.mailbox_order
            .iter()
            .filter(|mailbox| {
                matches!(
                    self.mailboxes.get(mailbox.as_str()),
                    Some(StartupSyncMailboxStatus::Pending)
                )
            })
            .count()
    }

    fn inflight_mailboxes_display(&self) -> String {
        let running: Vec<&str> = self
            .mailbox_order
            .iter()
            .filter_map(|mailbox| {
                matches!(
                    self.mailboxes.get(mailbox.as_str()),
                    Some(StartupSyncMailboxStatus::InFlight)
                )
                .then_some(mailbox.as_str())
            })
            .collect();
        if running.is_empty() {
            "-".to_string()
        } else {
            running.join(",")
        }
    }

    fn mailbox_states_display(&self) -> String {
        self.mailbox_order
            .iter()
            .map(|mailbox| {
                let status = self
                    .mailboxes
                    .get(mailbox.as_str())
                    .copied()
                    .unwrap_or(StartupSyncMailboxStatus::Pending);
                format!("{mailbox}:{}", status.log_label())
            })
            .collect::<Vec<String>>()
            .join(" ")
    }

    fn progress_summary(&self) -> String {
        format!(
            "{}/{} ok={} fail={} queued={} running={}",
            self.completed,
            self.total,
            self.succeeded,
            self.failed,
            self.pending_count(),
            self.inflight_mailboxes_display()
        )
    }
}

#[derive(Debug, Default)]
struct CommandPaletteState {
    open: bool,
    input: String,
    suggestions: Vec<PaletteSuggestion>,
    show_suggestions: bool,
    last_tab_input: String,
    last_local_result: Option<LocalCommandResult>,
}

impl CommandPaletteState {
    fn clear_completion(&mut self) {
        self.suggestions.clear();
        self.show_suggestions = false;
        self.last_tab_input.clear();
    }

    fn clear_local_result(&mut self) {
        self.last_local_result = None;
    }
}

#[derive(Debug, Default)]
struct SearchState {
    active: bool,
    input: String,
    applied_query: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum ConfigEditorMode {
    #[default]
    Browse,
    Edit,
}

#[derive(Debug, Default)]
struct ConfigEditorState {
    open: bool,
    selected_field: usize,
    mode: ConfigEditorMode,
    input: String,
}

#[derive(Debug, Clone, Copy)]
struct ConfigEditorField {
    key: &'static str,
    description: &'static str,
}

#[derive(Debug, Clone)]
struct SubscriptionItem {
    mailbox: String,
    label: String,
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
enum CodeEditMode {
    Browse,
    VimNormal,
    VimInsert,
    VimCommand,
}

impl CodeEditMode {
    fn is_active(self) -> bool {
        !matches!(self, Self::Browse)
    }

    fn label(self) -> &'static str {
        match self {
            Self::Browse => "BROWSE",
            Self::VimNormal => "NORMAL",
            Self::VimInsert => "INSERT",
            Self::VimCommand => "COMMAND",
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
    imap_defaults_initialized: bool,
    ui_page: UiPage,
    focus: Pane,
    code_focus: CodePaneFocus,
    subscriptions: Vec<SubscriptionItem>,
    enabled_group_expanded: bool,
    disabled_group_expanded: bool,
    threads: Vec<ThreadRow>,
    series_summaries: HashMap<i64, patch_worker::SeriesSummary>,
    filtered_thread_indices: Vec<usize>,
    subscription_index: usize,
    subscription_row_index: usize,
    kernel_tree_rows: Vec<KernelTreeRow>,
    kernel_tree_expanded_paths: HashSet<PathBuf>,
    kernel_tree_row_index: usize,
    code_preview_scroll: u16,
    code_edit_mode: CodeEditMode,
    code_edit_target: Option<PathBuf>,
    code_edit_buffer: Vec<String>,
    code_edit_cursor_row: usize,
    code_edit_cursor_col: usize,
    code_edit_dirty: bool,
    code_edit_command_input: String,
    thread_index: usize,
    preview_scroll: u16,
    started_at: Instant,
    status: String,
    last_apply_snapshot: Option<LastApplySnapshot>,
    palette: CommandPaletteState,
    search: SearchState,
    config_editor: ConfigEditorState,
    external_editor_runner: ExternalEditorRunner,
    needs_terminal_refresh: bool,
    startup_sync: Option<StartupSyncState>,
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
        let persisted_imap_defaults_initialized = persisted
            .as_ref()
            .map(|state| state.imap_defaults_initialized)
            .unwrap_or(false);
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
            .unwrap_or_else(|| runtime.default_active_mailbox().to_string());
        let subscriptions = default_subscriptions(
            &runtime,
            &enabled_mailboxes,
            Some(active_thread_mailbox.as_str()),
            runtime.imap.is_complete() && !persisted_imap_defaults_initialized,
        );
        let kernel_tree_expanded_paths = default_kernel_tree_expanded_paths(&runtime.kernel_trees);
        let kernel_tree_rows =
            build_kernel_tree_rows(&runtime.kernel_trees, &kernel_tree_expanded_paths);
        let mut state = Self {
            active_thread_mailbox,
            runtime,
            ui_state_path,
            imap_defaults_initialized: persisted_imap_defaults_initialized,
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
            series_summaries: HashMap::new(),
            filtered_thread_indices: Vec::new(),
            subscription_index: 0,
            subscription_row_index: 0,
            kernel_tree_rows,
            kernel_tree_expanded_paths,
            kernel_tree_row_index: 0,
            code_preview_scroll: 0,
            code_edit_mode: CodeEditMode::Browse,
            code_edit_target: None,
            code_edit_buffer: Vec::new(),
            code_edit_cursor_row: 0,
            code_edit_cursor_col: 0,
            code_edit_dirty: false,
            code_edit_command_input: String::new(),
            thread_index: 0,
            preview_scroll: 0,
            started_at: Instant::now(),
            status: "ready".to_string(),
            last_apply_snapshot: None,
            palette: CommandPaletteState::default(),
            search: SearchState::default(),
            config_editor: ConfigEditorState::default(),
            external_editor_runner: run_external_editor_session,
            needs_terminal_refresh: false,
            startup_sync: None,
        };
        if state.runtime.imap.is_complete() {
            state.imap_defaults_initialized = true;
        }
        if let Some(index) = state
            .subscriptions
            .iter()
            .position(|item| item.mailbox == state.active_thread_mailbox)
        {
            state.subscription_index = index;
        }
        state.refresh_series_summaries();
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
        self.refresh_series_summaries();
        self.thread_index = 0;
        self.preview_scroll = 0;
        self.apply_thread_filter();
    }

    fn show_mailbox_threads(
        &mut self,
        mailbox: &str,
        threads: Vec<ThreadRow>,
        status: String,
        persist_ui_state: bool,
    ) {
        self.active_thread_mailbox = mailbox.to_string();
        if let Some(index) = self
            .subscriptions
            .iter()
            .position(|item| item.mailbox == mailbox)
        {
            self.subscription_index = index;
            self.sync_subscription_row_to_selected_item();
        }
        self.replace_threads(threads);
        self.status = status;
        if persist_ui_state {
            self.persist_ui_state();
        }
    }

    fn recover_from_empty_active_mailbox(&mut self, reason: &str) -> bool {
        if !self.threads.is_empty() {
            return false;
        }

        let current_mailbox = self.active_thread_mailbox.clone();
        let mut candidates = self.enabled_mailboxes();
        if !candidates
            .iter()
            .any(|mailbox| mailbox == &self.runtime.source_mailbox)
        {
            candidates.push(self.runtime.source_mailbox.clone());
        }
        candidates.retain(|mailbox| mailbox != &current_mailbox);
        candidates.dedup();

        for mailbox in candidates {
            match mail_store::load_thread_rows_by_mailbox(
                &self.runtime.database_path,
                &mailbox,
                500,
            ) {
                Ok(rows) if !rows.is_empty() => {
                    self.show_mailbox_threads(
                        &mailbox,
                        rows,
                        format!("{reason}; showing threads for {mailbox}"),
                        true,
                    );
                    return true;
                }
                Ok(_) => {}
                Err(error) => {
                    tracing::warn!(
                        mailbox = %mailbox,
                        error = %error,
                        "failed to load fallback mailbox thread rows"
                    );
                }
            }
        }

        false
    }

    fn startup_sync_mailbox_status(&self, mailbox: &str) -> Option<StartupSyncMailboxStatus> {
        self.startup_sync
            .as_ref()
            .and_then(|state| state.mailboxes.get(mailbox).copied())
    }

    fn startup_sync_mailbox_pending(&self, mailbox: &str) -> bool {
        matches!(
            self.startup_sync_mailbox_status(mailbox),
            Some(StartupSyncMailboxStatus::Pending | StartupSyncMailboxStatus::InFlight)
        )
    }

    fn startup_sync_progress_text(&self) -> Option<String> {
        self.startup_sync
            .as_ref()
            .map(|state| format!("sync: {}", state.progress_summary()))
    }

    fn refresh_series_summaries(&mut self) {
        self.series_summaries =
            patch_worker::build_series_index(&self.active_thread_mailbox, &self.threads);
        if let Err(error) = patch_worker::hydrate_series_statuses(
            &self.runtime.database_path,
            &self.active_thread_mailbox,
            &mut self.series_summaries,
        ) {
            tracing::warn!(
                mailbox = %self.active_thread_mailbox,
                error = %error,
                "failed to hydrate patch series status from database"
            );
        }
    }

    fn enabled_mailboxes(&self) -> Vec<String> {
        self.subscriptions
            .iter()
            .filter(|item| item.enabled)
            .map(|item| item.mailbox.clone())
            .collect()
    }

    fn start_startup_sync_if_enabled(&mut self) {
        if !self.runtime.startup_sync {
            tracing::info!(
                op = "startup_sync",
                status = "disabled",
                reason = "ui.startup_sync=false"
            );
            return;
        }

        let mailboxes = self.enabled_mailboxes();
        if mailboxes.is_empty() {
            tracing::info!(
                op = "startup_sync",
                status = "skipped",
                reason = "no_enabled_subscriptions"
            );
            return;
        }

        let receiver = spawn_startup_sync_worker(self.runtime.clone(), mailboxes.clone());
        self.startup_sync = Some(StartupSyncState {
            receiver,
            mailbox_order: mailboxes.clone(),
            mailboxes: mailboxes
                .iter()
                .cloned()
                .map(|mailbox| (mailbox, StartupSyncMailboxStatus::Pending))
                .collect(),
            total: mailboxes.len(),
            completed: 0,
            succeeded: 0,
            failed: 0,
        });
        self.status = format!(
            "startup sync queued: {} mailbox(es): {}",
            mailboxes.len(),
            mailboxes.join(", ")
        );
        if let Some(sync_state) = self.startup_sync.as_ref() {
            tracing::info!(
                op = "startup_sync",
                status = "started",
                total = sync_state.total,
                completed = sync_state.completed,
                succeeded = sync_state.succeeded,
                failed = sync_state.failed,
                queued = sync_state.pending_count(),
                running = %sync_state.inflight_mailboxes_display(),
                mailbox_states = %sync_state.mailbox_states_display(),
                mailboxes = %mailboxes.join(",")
            );
        }
    }

    fn pump_startup_sync_events(&mut self) {
        let mut events = Vec::new();
        let mut disconnected = false;
        {
            let Some(sync_state) = self.startup_sync.as_ref() else {
                return;
            };
            loop {
                match sync_state.receiver.try_recv() {
                    Ok(event) => events.push(event),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }
        }

        for event in events {
            self.apply_startup_sync_event(event);
        }

        if disconnected && self.startup_sync.is_some() {
            let (completed, total, succeeded, failed) = self
                .startup_sync
                .as_ref()
                .map(|state| (state.completed, state.total, state.succeeded, state.failed))
                .unwrap_or((0, 0, 0, 0));
            self.startup_sync = None;
            self.status = format!(
                "startup sync worker disconnected (completed={completed}/{total} ok={succeeded} failed={failed})"
            );
            tracing::warn!(
                op = "startup_sync",
                status = "failed",
                completed,
                total,
                succeeded,
                failed,
                "startup sync worker disconnected unexpectedly"
            );
        }
    }

    fn apply_startup_sync_event(&mut self, event: StartupSyncEvent) {
        match event {
            StartupSyncEvent::MailboxStarted {
                mailbox,
                index,
                total,
            } => {
                if let Some(sync_state) = self.startup_sync.as_mut() {
                    sync_state
                        .mailboxes
                        .insert(mailbox.clone(), StartupSyncMailboxStatus::InFlight);
                }
                self.status = format!("startup sync [{index}/{total}] syncing {mailbox}...");
                if let Some(sync_state) = self.startup_sync.as_ref() {
                    tracing::info!(
                        op = "startup_sync",
                        status = "progress",
                        phase = "started",
                        mailbox = %mailbox,
                        index,
                        total,
                        completed = sync_state.completed,
                        succeeded = sync_state.succeeded,
                        failed = sync_state.failed,
                        queued = sync_state.pending_count(),
                        running = %sync_state.inflight_mailboxes_display(),
                        mailbox_states = %sync_state.mailbox_states_display()
                    );
                }
            }
            StartupSyncEvent::MailboxFinished {
                mailbox,
                fetched,
                inserted,
                updated,
            } => {
                if let Some(sync_state) = self.startup_sync.as_mut() {
                    sync_state
                        .mailboxes
                        .insert(mailbox.clone(), StartupSyncMailboxStatus::Finished);
                    sync_state.completed += 1;
                    sync_state.succeeded += 1;
                }

                if mailbox == self.active_thread_mailbox {
                    self.reload_active_mailbox_threads_after_sync();
                }
                if self.threads.is_empty()
                    && !self.startup_sync_mailbox_pending(&self.active_thread_mailbox)
                {
                    let _ = self.recover_from_empty_active_mailbox(&format!(
                        "startup sync ready for {mailbox}"
                    ));
                }

                if let Some(sync_state) = self.startup_sync.as_ref() {
                    tracing::info!(
                        op = "startup_sync",
                        status = "succeeded",
                        phase = "finished",
                        mailbox = %mailbox,
                        fetched,
                        inserted,
                        updated,
                        completed = sync_state.completed,
                        total = sync_state.total,
                        succeeded = sync_state.succeeded,
                        failed = sync_state.failed,
                        queued = sync_state.pending_count(),
                        running = %sync_state.inflight_mailboxes_display(),
                        mailbox_states = %sync_state.mailbox_states_display()
                    );
                }
            }
            StartupSyncEvent::MailboxFailed { mailbox, error } => {
                if let Some(sync_state) = self.startup_sync.as_mut() {
                    sync_state
                        .mailboxes
                        .insert(mailbox.clone(), StartupSyncMailboxStatus::Failed);
                    sync_state.completed += 1;
                    sync_state.failed += 1;
                }
                self.status = format!("startup sync failed for {mailbox}: {error}");
                if mailbox == self.active_thread_mailbox && self.threads.is_empty() {
                    let _ = self.recover_from_empty_active_mailbox(&format!(
                        "startup sync failed for {mailbox}: {error}"
                    ));
                }
                if let Some(sync_state) = self.startup_sync.as_ref() {
                    tracing::error!(
                        op = "startup_sync",
                        status = "failed",
                        phase = "finished",
                        mailbox = %mailbox,
                        error = %error,
                        completed = sync_state.completed,
                        total = sync_state.total,
                        succeeded = sync_state.succeeded,
                        failed = sync_state.failed,
                        queued = sync_state.pending_count(),
                        running = %sync_state.inflight_mailboxes_display(),
                        mailbox_states = %sync_state.mailbox_states_display()
                    );
                }
            }
            StartupSyncEvent::WorkerCompleted => {}
        }

        self.maybe_finish_startup_sync();
    }

    fn maybe_finish_startup_sync(&mut self) {
        let Some(sync_state) = self.startup_sync.as_ref() else {
            return;
        };
        if sync_state.completed < sync_state.total {
            return;
        }

        let succeeded = sync_state.succeeded;
        let failed = sync_state.failed;
        let total = sync_state.total;
        let mailbox_states = sync_state.mailbox_states_display();
        self.startup_sync = None;
        self.status =
            format!("startup sync finished: ok={succeeded} failed={failed} total={total}");
        tracing::info!(
            op = "startup_sync",
            status = if failed == 0 { "succeeded" } else { "partial" },
            succeeded,
            failed,
            total,
            mailbox_states = %mailbox_states
        );
    }

    fn reload_active_mailbox_threads_after_sync(&mut self) {
        match mail_store::load_thread_rows_by_mailbox(
            &self.runtime.database_path,
            &self.active_thread_mailbox,
            500,
        ) {
            Ok(rows) => self.replace_threads(rows),
            Err(error) => {
                tracing::error!(
                    op = "startup_sync",
                    status = "failed",
                    mailbox = %self.active_thread_mailbox,
                    error = %error
                );
                self.status = format!(
                    "startup sync ok but failed to reload threads for {}: {}",
                    self.active_thread_mailbox, error
                );
            }
        }
    }

    fn to_ui_state(&self) -> UiState {
        UiState {
            enabled_mailboxes: self.enabled_mailboxes(),
            enabled_group_expanded: self.enabled_group_expanded,
            disabled_group_expanded: self.disabled_group_expanded,
            imap_defaults_initialized: self.imap_defaults_initialized,
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
                    text: format!(
                        "  {}",
                        subscription_line(item, self.startup_sync_mailbox_status(&item.mailbox))
                    ),
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
                    text: format!(
                        "  {}",
                        subscription_line(item, self.startup_sync_mailbox_status(&item.mailbox))
                    ),
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
        let label = self.subscriptions[selected_index].label.clone();
        if let Some(item) = self.subscriptions.get_mut(selected_index) {
            item.enabled = enabled;
        }

        self.sort_subscriptions_keep_selected(&mailbox);
        let marker = if enabled { "enabled" } else { "disabled" };
        self.status = format!("{marker} subscription {label}");
        self.persist_ui_state();
    }

    fn sort_subscriptions_keep_selected(&mut self, selected_mailbox: &str) {
        self.subscriptions.sort_by(|left, right| {
            right
                .enabled
                .cmp(&left.enabled)
                .then_with(|| left.label.cmp(&right.label))
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
        tracing::debug!(mailbox = %mailbox, enabled, "user opened subscription");

        if !enabled {
            self.status = format!("subscription {} is disabled, press y to enable", mailbox);
            return;
        }

        match mail_store::load_thread_rows_by_mailbox(&self.runtime.database_path, &mailbox, 500) {
            Ok(rows) if !rows.is_empty() => {
                self.show_mailbox_threads(
                    &mailbox,
                    rows,
                    format!("showing threads for {}", mailbox),
                    true,
                );
            }
            Ok(_) => {
                if self.startup_sync_mailbox_pending(&mailbox) {
                    self.show_mailbox_threads(
                        &mailbox,
                        Vec::new(),
                        format!("{mailbox} is syncing in background; page stays responsive"),
                        true,
                    );
                    return;
                }

                tracing::info!(
                    op = "subscription_sync",
                    status = "started",
                    mailbox = %mailbox
                );
                let request = sync_worker::SyncRequest {
                    mailbox: mailbox.clone(),
                    fixture_dir: None,
                    uidvalidity: None,
                    reconnect_attempts: PALETTE_SYNC_RECONNECT_ATTEMPTS,
                };

                match run_sync_request_guarded(&self.runtime, request) {
                    Ok(summary) => match mail_store::load_thread_rows_by_mailbox(
                        &self.runtime.database_path,
                        &mailbox,
                        500,
                    ) {
                        Ok(fresh_rows) => {
                            tracing::info!(
                                op = "subscription_sync",
                                status = "succeeded",
                                mailbox = %mailbox,
                                fetched = summary.fetched,
                                inserted = summary.inserted,
                                updated = summary.updated
                            );
                            self.show_mailbox_threads(
                                &mailbox,
                                fresh_rows,
                                format!(
                                    "synced {}: fetched={} inserted={} updated={}",
                                    mailbox, summary.fetched, summary.inserted, summary.updated
                                ),
                                true,
                            );
                        }
                        Err(error) => {
                            tracing::error!(
                                mailbox = %mailbox,
                                error = %error,
                                "sync succeeded but reload thread rows failed"
                            );
                            self.status = format!(
                                "sync ok but failed to reload threads for {}: {error}",
                                mailbox
                            );
                        }
                    },
                    Err(error) => {
                        tracing::error!(
                            op = "subscription_sync",
                            status = "failed",
                            mailbox = %mailbox,
                            error = %error,
                            "subscription sync failed"
                        );
                        self.status = format!("failed to sync {}: {error}", mailbox);
                    }
                }
            }
            Err(error) => {
                tracing::error!(
                    mailbox = %mailbox,
                    error = %error,
                    "failed to load mailbox thread rows"
                );
                self.status = format!("failed to load threads for {}: {error}", mailbox);
            }
        }
    }

    fn selected_thread(&self) -> Option<&ThreadRow> {
        self.filtered_thread_indices
            .get(self.thread_index)
            .and_then(|index| self.threads.get(*index))
    }

    fn selected_series(&self) -> Option<&patch_worker::SeriesSummary> {
        let thread = self.selected_thread()?;
        self.series_summaries.get(&thread.thread_id)
    }

    fn run_patch_action(&mut self, action: patch_worker::PatchAction) {
        tracing::info!(
            op = "patch.action",
            status = "started",
            action = action.name()
        );
        if !matches!(self.ui_page, UiPage::Mail) {
            self.status = "patch action is only available on mail page".to_string();
            return;
        }

        let Some(series) = self.selected_series().cloned() else {
            self.status = "current thread is not a patch series".to_string();
            return;
        };

        match patch_worker::run_action(&self.runtime, &series, action) {
            Ok(result) => {
                if let Some(series_summary) = self.series_summaries.get_mut(&series.thread_id) {
                    series_summary.status = result.status;
                }
                if matches!(action, patch_worker::PatchAction::Apply)
                    && result.status == crate::domain::models::PatchSeriesStatus::Applied
                    && let (Some(before_head), Some(after_head)) =
                        (result.head_before.as_deref(), result.head_after.as_deref())
                {
                    self.last_apply_snapshot = Some(LastApplySnapshot {
                        thread_id: series.thread_id,
                        before_head: before_head.to_string(),
                        after_head: after_head.to_string(),
                    });
                }
                let exit_code = result
                    .exit_code
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "unknown".to_string());
                let output_dir = result
                    .output_path
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "-".to_string());
                self.status = format!(
                    "{}: {} (status={} exit={} timeout={})",
                    action.name(),
                    result.summary,
                    match result.status {
                        crate::domain::models::PatchSeriesStatus::New => "new",
                        crate::domain::models::PatchSeriesStatus::Reviewing => "reviewing",
                        crate::domain::models::PatchSeriesStatus::Applied => "applied",
                        crate::domain::models::PatchSeriesStatus::Failed => "failed",
                        crate::domain::models::PatchSeriesStatus::Conflict => "conflict",
                    },
                    exit_code,
                    result.timed_out
                );
                tracing::info!(
                    op = "patch.action",
                    status = "succeeded",
                    action = action.name(),
                    command = %result.command_line,
                    output_dir = %output_dir,
                );
            }
            Err(error) => {
                tracing::error!(
                    op = "patch.action",
                    status = "failed",
                    action = action.name(),
                    error = %error
                );
                self.status = format!("{} failed: {}", action.name(), error);
            }
        }
    }

    fn run_patch_undo_action(&mut self) {
        tracing::info!(op = "patch.undo", status = "started");
        if !matches!(self.ui_page, UiPage::Mail) {
            self.status = "undo is only available on mail page".to_string();
            return;
        }

        let Some(snapshot) = self.last_apply_snapshot.clone() else {
            self.status = "no apply action to undo in this session".to_string();
            return;
        };

        match patch_worker::undo_last_apply(
            &self.runtime,
            &snapshot.before_head,
            &snapshot.after_head,
        ) {
            Ok(head_after_reset) => {
                if let Some(series_summary) = self.series_summaries.get_mut(&snapshot.thread_id) {
                    series_summary.status = crate::domain::models::PatchSeriesStatus::New;
                }
                self.last_apply_snapshot = None;
                self.status = format!(
                    "undo apply: reset HEAD to {}",
                    short_commit_id(&head_after_reset)
                );
                tracing::info!(
                    op = "patch.undo",
                    status = "succeeded",
                    thread_id = snapshot.thread_id,
                    head = %head_after_reset
                );
            }
            Err(error) => {
                tracing::error!(op = "patch.undo", status = "failed", error = %error);
                self.status = format!("undo apply failed: {error}");
            }
        }
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
            self.palette.clear_local_result();
            self.status = "command palette opened".to_string();
        } else {
            self.palette.input.clear();
            self.palette.clear_completion();
            self.palette.clear_local_result();
            self.status = "command palette closed".to_string();
        }
    }

    fn close_palette(&mut self) {
        self.palette.open = false;
        self.palette.input.clear();
        self.palette.clear_completion();
        self.palette.clear_local_result();
        self.status = "command palette closed".to_string();
    }

    fn open_config_editor(&mut self, key_hint: Option<&str>) {
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

    fn close_config_editor(&mut self) {
        self.config_editor.open = false;
        self.config_editor.mode = ConfigEditorMode::Browse;
        self.config_editor.input.clear();
        self.status = "config editor closed".to_string();
    }

    fn selected_config_editor_field(&self) -> &'static ConfigEditorField {
        let index = self
            .config_editor
            .selected_field
            .min(CONFIG_EDITOR_FIELDS.len().saturating_sub(1));
        &CONFIG_EDITOR_FIELDS[index]
    }

    fn move_config_editor_up(&mut self) {
        if self.config_editor.selected_field > 0 {
            self.config_editor.selected_field -= 1;
        }
    }

    fn move_config_editor_down(&mut self) {
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

    fn start_config_editor_edit(&mut self) {
        let key = self.selected_config_editor_field().key;
        self.config_editor.mode = ConfigEditorMode::Edit;
        self.config_editor.input = self.config_editor_seed_input(key);
        self.status = format!("editing config {key}");
    }

    fn cycle_config_editor_value(&mut self) {
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
            Ok(rendered_value) => match reload_runtime_from_config(self) {
                Ok(()) => {
                    self.status = format!("config updated: {key} = {rendered_value}");
                    tracing::info!(
                        op = "config.set",
                        status = "succeeded",
                        key = %key,
                        value = %rendered_value,
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
                    self.status = format!("config file updated but runtime reload failed: {error}");
                }
            },
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

    fn save_config_editor_value(&mut self) {
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
            Ok(rendered_value) => match reload_runtime_from_config(self) {
                Ok(()) => {
                    self.config_editor.mode = ConfigEditorMode::Browse;
                    self.config_editor.input.clear();
                    self.status = format!("config updated: {key} = {rendered_value}");
                    tracing::info!(
                        op = "config.set",
                        status = "succeeded",
                        key = %key,
                        value = %rendered_value,
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
                    self.status = format!("config file updated but runtime reload failed: {error}");
                }
            },
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

    fn unset_selected_config_key(&mut self) {
        let key = self.selected_config_editor_field().key;
        tracing::info!(op = "config.unset", status = "started", key = %key);
        match remove_config_key_from_file(&self.runtime.config_path, key) {
            Ok(true) => match reload_runtime_from_config(self) {
                Ok(()) => {
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
                Err(error) => {
                    tracing::error!(
                        op = "config.unset",
                        status = "failed",
                        key = %key,
                        error = %error
                    );
                    self.status = format!("config file updated but runtime reload failed: {error}");
                }
            },
            Ok(false) => {
                self.status = format!("config key already unset: {key}");
            }
            Err(error) => {
                tracing::error!(op = "config.unset", status = "failed", key = %key, error = %error);
                self.status = format!("failed to unset config key {key}: {error}");
            }
        }
    }

    fn is_code_edit_active(&self) -> bool {
        self.code_edit_mode.is_active()
    }

    fn mark_terminal_refresh_needed(&mut self) {
        self.needs_terminal_refresh = true;
    }

    fn take_terminal_refresh_needed(&mut self) -> bool {
        std::mem::take(&mut self.needs_terminal_refresh)
    }

    fn enter_code_edit_mode(&mut self) {
        if !matches!(self.ui_page, UiPage::CodeBrowser)
            || !matches!(self.code_focus, CodePaneFocus::Source)
        {
            self.status = CODE_EDIT_ENTRY_HINT.to_string();
            return;
        }

        let Some(path) = self.selected_kernel_tree_file_path().map(Path::to_path_buf) else {
            self.status = CODE_EDIT_ENTRY_HINT.to_string();
            return;
        };

        let buffer = match load_code_edit_buffer_from_path(&path) {
            Ok(buffer) => buffer,
            Err(error) => {
                self.status = error;
                return;
            }
        };

        self.code_edit_mode = CodeEditMode::VimNormal;
        self.code_edit_target = Some(path.clone());
        self.code_edit_buffer = buffer;
        self.code_edit_cursor_row = 0;
        self.code_edit_cursor_col = 0;
        self.code_edit_dirty = false;
        self.code_edit_command_input.clear();
        self.code_preview_scroll = 0;
        self.status = format!("editing {} (NORMAL)", path.display());
        tracing::info!(
            op = "code.edit",
            status = "started",
            file = %path.display()
        );
    }

    fn exit_code_edit_mode(&mut self, status: String) {
        let target_for_log = self
            .code_edit_target
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "<none>".to_string());
        self.code_edit_mode = CodeEditMode::Browse;
        self.code_edit_target = None;
        self.code_edit_buffer.clear();
        self.code_edit_cursor_row = 0;
        self.code_edit_cursor_col = 0;
        self.code_edit_dirty = false;
        self.code_edit_command_input.clear();
        self.code_preview_scroll = 0;
        self.status = status;
        tracing::info!(
            op = "code.edit",
            status = "succeeded",
            file = %target_for_log,
            detail = %self.status
        );
    }

    fn save_code_edit_buffer(&mut self) -> bool {
        let Some(path) = self.code_edit_target.clone() else {
            self.status = "no file is being edited".to_string();
            return false;
        };

        let content = self.code_edit_buffer.join("\n");
        match fs::write(&path, content) {
            Ok(_) => {
                self.code_edit_dirty = false;
                self.status = format!("saved {}", path.display());
                tracing::info!(
                    op = "code.save",
                    status = "succeeded",
                    file = %path.display(),
                    lines = self.code_edit_buffer.len()
                );
                true
            }
            Err(error) => {
                self.status = format!("failed to save {}: {}", path.display(), error);
                tracing::error!(
                    op = "code.save",
                    status = "failed",
                    file = %path.display(),
                    error = %error
                );
                false
            }
        }
    }

    fn current_external_editor_target(&self) -> Option<PathBuf> {
        if self.is_code_edit_active() {
            self.code_edit_target
                .clone()
                .or_else(|| self.selected_kernel_tree_file_path().map(Path::to_path_buf))
        } else {
            self.selected_kernel_tree_file_path().map(Path::to_path_buf)
        }
    }

    fn reload_code_edit_target_from_disk(
        &mut self,
        path: &Path,
    ) -> std::result::Result<(), String> {
        let Some(target) = self.code_edit_target.as_ref() else {
            return Ok(());
        };
        if target != path {
            return Ok(());
        }

        let buffer = load_code_edit_buffer_from_path(path)?;
        self.code_edit_buffer = buffer;
        self.code_edit_dirty = false;
        self.clamp_code_edit_cursor();
        self.adjust_code_edit_scroll();
        Ok(())
    }

    fn open_external_editor(&mut self) {
        if !matches!(self.ui_page, UiPage::CodeBrowser)
            || !matches!(self.code_focus, CodePaneFocus::Source)
        {
            tracing::info!(
                op = "external_editor",
                status = "blocked",
                ui_page = ?self.ui_page,
                code_focus = ?self.code_focus,
                reason = "invalid_page_or_focus"
            );
            self.status = EXTERNAL_EDITOR_ENTRY_HINT.to_string();
            return;
        }

        if self.code_edit_dirty {
            tracing::info!(
                op = "external_editor",
                status = "blocked",
                reason = "inline_buffer_dirty"
            );
            self.status = "unsaved changes, run :w before external vim".to_string();
            return;
        }

        let Some(path) = self.current_external_editor_target() else {
            tracing::info!(
                op = "external_editor",
                status = "blocked",
                reason = "no_source_file_selected"
            );
            self.status = EXTERNAL_EDITOR_ENTRY_HINT.to_string();
            return;
        };

        let editor = resolve_external_editor();
        let from_inline_edit = self.is_code_edit_active();
        tracing::info!(
            op = "external_editor",
            status = "started",
            editor = %editor,
            file = %path.display(),
            from_inline_edit
        );
        let runner = self.external_editor_runner;
        match runner(&editor, &path) {
            Ok(result) => {
                self.mark_terminal_refresh_needed();
                let reload_status = self.reload_code_edit_target_from_disk(&path);
                self.code_preview_scroll = 0;
                self.status = match (result.success, result.exit_code) {
                    (true, Some(code)) => format!(
                        "external vim exited successfully (editor={} exit={} file={})",
                        editor,
                        code,
                        path.display()
                    ),
                    (true, None) => format!(
                        "external vim exited successfully (editor={} file={})",
                        editor,
                        path.display()
                    ),
                    (false, Some(code)) => format!(
                        "external vim exited with code {} (editor={} file={})",
                        code,
                        editor,
                        path.display()
                    ),
                    (false, None) => format!(
                        "external vim terminated by signal (editor={} file={})",
                        editor,
                        path.display()
                    ),
                };

                if let Err(error) = reload_status {
                    self.status = format!(
                        "{}; failed to reload {}: {}",
                        self.status,
                        path.display(),
                        error
                    );
                }

                tracing::info!(
                    op = "external_editor",
                    status = if result.success { "succeeded" } else { "failed" },
                    editor = %editor,
                    file = %path.display(),
                    success = result.success,
                    exit_code = ?result.exit_code
                );
            }
            Err(error) => {
                self.mark_terminal_refresh_needed();
                self.status = format!("external vim failed for {}: {}", path.display(), error);
                tracing::error!(
                    op = "external_editor",
                    status = "failed",
                    editor = %editor,
                    file = %path.display(),
                    error = %error
                );
            }
        }
    }

    fn code_edit_line_len(&self, row: usize) -> usize {
        self.code_edit_buffer
            .get(row)
            .map(|line| line.chars().count())
            .unwrap_or(0)
    }

    fn clamp_code_edit_cursor(&mut self) {
        if self.code_edit_buffer.is_empty() {
            self.code_edit_buffer.push(String::new());
        }
        if self.code_edit_cursor_row >= self.code_edit_buffer.len() {
            self.code_edit_cursor_row = self.code_edit_buffer.len().saturating_sub(1);
        }
        let line_len = self.code_edit_line_len(self.code_edit_cursor_row);
        if self.code_edit_cursor_col > line_len {
            self.code_edit_cursor_col = line_len;
        }
    }

    fn adjust_code_edit_scroll(&mut self) {
        const EDIT_HEADER_LINES: usize = 4;
        let logical_cursor_line = self.code_edit_cursor_row.saturating_add(EDIT_HEADER_LINES);
        let scroll_target = logical_cursor_line.saturating_sub(3);
        self.code_preview_scroll = scroll_target.min(u16::MAX as usize) as u16;
    }

    fn move_code_edit_cursor_left(&mut self) {
        self.clamp_code_edit_cursor();
        if self.code_edit_cursor_col > 0 {
            self.code_edit_cursor_col -= 1;
        } else if self.code_edit_cursor_row > 0 {
            self.code_edit_cursor_row -= 1;
            self.code_edit_cursor_col = self.code_edit_line_len(self.code_edit_cursor_row);
        }
        self.adjust_code_edit_scroll();
    }

    fn move_code_edit_cursor_right(&mut self) {
        self.clamp_code_edit_cursor();
        let line_len = self.code_edit_line_len(self.code_edit_cursor_row);
        if self.code_edit_cursor_col < line_len {
            self.code_edit_cursor_col += 1;
        } else if self.code_edit_cursor_row + 1 < self.code_edit_buffer.len() {
            self.code_edit_cursor_row += 1;
            self.code_edit_cursor_col = 0;
        }
        self.adjust_code_edit_scroll();
    }

    fn move_code_edit_cursor_up(&mut self) {
        self.clamp_code_edit_cursor();
        if self.code_edit_cursor_row > 0 {
            self.code_edit_cursor_row -= 1;
            let line_len = self.code_edit_line_len(self.code_edit_cursor_row);
            self.code_edit_cursor_col = self.code_edit_cursor_col.min(line_len);
        }
        self.adjust_code_edit_scroll();
    }

    fn move_code_edit_cursor_down(&mut self) {
        self.clamp_code_edit_cursor();
        if self.code_edit_cursor_row + 1 < self.code_edit_buffer.len() {
            self.code_edit_cursor_row += 1;
            let line_len = self.code_edit_line_len(self.code_edit_cursor_row);
            self.code_edit_cursor_col = self.code_edit_cursor_col.min(line_len);
        }
        self.adjust_code_edit_scroll();
    }

    fn insert_code_edit_character(&mut self, character: char) -> bool {
        self.clamp_code_edit_cursor();
        let Some(line) = self.code_edit_buffer.get_mut(self.code_edit_cursor_row) else {
            return false;
        };
        let byte_index = char_to_byte_index(line, self.code_edit_cursor_col);
        line.insert(byte_index, character);
        self.code_edit_cursor_col += 1;
        self.code_edit_dirty = true;
        self.adjust_code_edit_scroll();
        true
    }

    fn backspace_code_edit_character(&mut self) -> bool {
        self.clamp_code_edit_cursor();

        if self.code_edit_cursor_col > 0 {
            let Some(line) = self.code_edit_buffer.get_mut(self.code_edit_cursor_row) else {
                return false;
            };
            let remove_at = self.code_edit_cursor_col - 1;
            let start = char_to_byte_index(line, remove_at);
            let end = char_to_byte_index(line, remove_at + 1);
            line.replace_range(start..end, "");
            self.code_edit_cursor_col -= 1;
            self.code_edit_dirty = true;
            self.adjust_code_edit_scroll();
            return true;
        }

        if self.code_edit_cursor_row == 0 {
            return false;
        }

        let current = self.code_edit_buffer.remove(self.code_edit_cursor_row);
        self.code_edit_cursor_row -= 1;
        let Some(previous_line) = self.code_edit_buffer.get_mut(self.code_edit_cursor_row) else {
            return false;
        };
        let previous_len = previous_line.chars().count();
        previous_line.push_str(&current);
        self.code_edit_cursor_col = previous_len;
        self.code_edit_dirty = true;
        self.adjust_code_edit_scroll();
        true
    }

    fn insert_code_edit_newline(&mut self) -> bool {
        self.clamp_code_edit_cursor();
        let Some(line) = self.code_edit_buffer.get_mut(self.code_edit_cursor_row) else {
            return false;
        };
        let byte_index = char_to_byte_index(line, self.code_edit_cursor_col);
        let tail = line.split_off(byte_index);
        self.code_edit_buffer
            .insert(self.code_edit_cursor_row + 1, tail);
        self.code_edit_cursor_row += 1;
        self.code_edit_cursor_col = 0;
        self.code_edit_dirty = true;
        self.adjust_code_edit_scroll();
        true
    }

    fn delete_code_edit_character(&mut self) -> bool {
        self.clamp_code_edit_cursor();
        let row = self.code_edit_cursor_row;
        if row >= self.code_edit_buffer.len() {
            return false;
        }

        let line_len = self.code_edit_line_len(row);
        if self.code_edit_cursor_col < line_len {
            let Some(line) = self.code_edit_buffer.get_mut(row) else {
                return false;
            };
            let start = char_to_byte_index(line, self.code_edit_cursor_col);
            let end = char_to_byte_index(line, self.code_edit_cursor_col + 1);
            line.replace_range(start..end, "");
            self.clamp_code_edit_cursor();
            self.code_edit_dirty = true;
            self.adjust_code_edit_scroll();
            return true;
        }

        if self.code_edit_cursor_col == line_len && row + 1 < self.code_edit_buffer.len() {
            let next = self.code_edit_buffer.remove(row + 1);
            let Some(line) = self.code_edit_buffer.get_mut(row) else {
                return false;
            };
            line.push_str(&next);
            self.code_edit_dirty = true;
            self.adjust_code_edit_scroll();
            return true;
        }

        false
    }

    fn enter_code_edit_command_mode(&mut self) {
        self.code_edit_mode = CodeEditMode::VimCommand;
        self.code_edit_command_input.clear();
        self.status = "command mode".to_string();
    }

    fn execute_code_edit_command(&mut self) {
        let command = self.code_edit_command_input.trim().to_string();
        self.code_edit_command_input.clear();
        self.code_edit_mode = CodeEditMode::VimNormal;

        if command.is_empty() {
            self.status = "empty command".to_string();
            return;
        }

        let target_for_log = self
            .code_edit_target
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "<none>".to_string());
        tracing::info!(
            op = "code.command",
            status = "started",
            command = %command,
            file = %target_for_log
        );
        match command.as_str() {
            "w" => {
                let _ = self.save_code_edit_buffer();
            }
            "q!" => {
                let target = self
                    .code_edit_target
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "<file>".to_string());
                self.exit_code_edit_mode(format!("discarded unsaved changes for {target}"));
            }
            "q" => {
                if self.code_edit_dirty {
                    self.status = "unsaved changes, run :w, :wq, or :q!".to_string();
                } else {
                    let target = self
                        .code_edit_target
                        .as_ref()
                        .map(|path| path.display().to_string())
                        .unwrap_or_else(|| "<file>".to_string());
                    self.exit_code_edit_mode(format!("exit edit mode for {target}"));
                }
            }
            "wq" => {
                if self.save_code_edit_buffer() {
                    let target = self
                        .code_edit_target
                        .as_ref()
                        .map(|path| path.display().to_string())
                        .unwrap_or_else(|| "<file>".to_string());
                    self.exit_code_edit_mode(format!("saved and exited {target}"));
                }
            }
            "vim" => {
                self.open_external_editor();
            }
            _ => {
                self.status = format!("unsupported command: :{command}");
            }
        }
    }
}

fn spawn_startup_sync_worker(
    runtime: RuntimeConfig,
    mailboxes: Vec<String>,
) -> Receiver<StartupSyncEvent> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let total = mailboxes.len();
        for (index, mailbox) in mailboxes.into_iter().enumerate() {
            if sender
                .send(StartupSyncEvent::MailboxStarted {
                    mailbox: mailbox.clone(),
                    index: index + 1,
                    total,
                })
                .is_err()
            {
                return;
            }

            let request = sync_worker::SyncRequest {
                mailbox: mailbox.clone(),
                fixture_dir: None,
                uidvalidity: None,
                reconnect_attempts: PALETTE_SYNC_RECONNECT_ATTEMPTS,
            };
            match run_sync_request_guarded(&runtime, request) {
                Ok(summary) => {
                    if sender
                        .send(StartupSyncEvent::MailboxFinished {
                            mailbox,
                            fetched: summary.fetched,
                            inserted: summary.inserted,
                            updated: summary.updated,
                        })
                        .is_err()
                    {
                        return;
                    }
                }
                Err(error) => {
                    if sender
                        .send(StartupSyncEvent::MailboxFailed {
                            mailbox,
                            error: error.to_string(),
                        })
                        .is_err()
                    {
                        return;
                    }
                }
            }
        }

        let _ = sender.send(StartupSyncEvent::WorkerCompleted);
    });

    receiver
}

fn run_sync_request_guarded(
    runtime: &RuntimeConfig,
    request: sync_worker::SyncRequest,
) -> Result<sync_worker::SyncSummary> {
    let mailbox = request.mailbox.clone();
    catch_sync_panic(&mailbox, || sync_worker::run(runtime, request))
}

fn catch_sync_panic<T, F>(mailbox: &str, operation: F) -> Result<T>
where
    F: FnOnce() -> Result<T>,
{
    match panic::catch_unwind(AssertUnwindSafe(operation)) {
        Ok(result) => result,
        Err(payload) => {
            let message = if let Some(message) = payload.downcast_ref::<String>() {
                message.clone()
            } else if let Some(message) = payload.downcast_ref::<&str>() {
                (*message).to_string()
            } else {
                "unknown panic payload".to_string()
            };

            Err(CourierError::new(
                ErrorCode::Tui,
                format!("sync panicked for {mailbox}: {message}"),
            ))
        }
    }
}

fn load_code_edit_buffer_from_path(path: &Path) -> std::result::Result<Vec<String>, String> {
    let content =
        fs::read(path).map_err(|error| format!("failed to read {}: {}", path.display(), error))?;
    let text = String::from_utf8_lossy(&content)
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let mut buffer: Vec<String> = text.split('\n').map(ToOwned::to_owned).collect();
    if buffer.is_empty() {
        buffer.push(String::new());
    }
    Ok(buffer)
}

fn pick_external_editor(visual: Option<&str>, editor: Option<&str>) -> String {
    visual
        .and_then(normalized_external_editor_value)
        .or_else(|| editor.and_then(normalized_external_editor_value))
        .unwrap_or_else(|| "vim".to_string())
}

fn normalized_external_editor_value(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn resolve_external_editor() -> String {
    let visual = env::var("VISUAL").ok();
    let editor = env::var("EDITOR").ok();
    pick_external_editor(visual.as_deref(), editor.as_deref())
}

fn run_external_editor_session(
    editor_spec: &str,
    file_path: &Path,
) -> std::result::Result<ExternalEditorProcessResult, String> {
    run_external_editor_session_with(
        editor_spec,
        file_path,
        || disable_raw_mode().map_err(|error| error.to_string()),
        || {
            let mut stdout = io::stdout();
            execute!(stdout, LeaveAlternateScreen).map_err(|error| error.to_string())
        },
        launch_external_editor_process,
        || {
            let mut stdout = io::stdout();
            execute!(stdout, EnterAlternateScreen).map_err(|error| error.to_string())
        },
        || enable_raw_mode().map_err(|error| error.to_string()),
    )
}

fn run_external_editor_session_with<DisableRaw, LeaveAlt, LaunchEditor, EnterAlt, EnableRaw>(
    editor_spec: &str,
    file_path: &Path,
    mut disable_raw: DisableRaw,
    mut leave_alternate_screen: LeaveAlt,
    mut launch_editor: LaunchEditor,
    mut enter_alternate_screen: EnterAlt,
    mut enable_raw: EnableRaw,
) -> std::result::Result<ExternalEditorProcessResult, String>
where
    DisableRaw: FnMut() -> std::result::Result<(), String>,
    LeaveAlt: FnMut() -> std::result::Result<(), String>,
    LaunchEditor: FnMut(&str, &Path) -> std::result::Result<ExternalEditorProcessResult, String>,
    EnterAlt: FnMut() -> std::result::Result<(), String>,
    EnableRaw: FnMut() -> std::result::Result<(), String>,
{
    disable_raw().map_err(|error| format!("failed to disable raw mode: {error}"))?;

    if let Err(error) = leave_alternate_screen() {
        let _ = enable_raw();
        return Err(format!("failed to leave alternate screen: {error}"));
    }

    let launch_result = launch_editor(editor_spec, file_path);
    let enter_result = enter_alternate_screen();
    let enable_result = enable_raw();

    let mut restore_errors = Vec::new();
    if let Err(error) = enter_result {
        restore_errors.push(format!("failed to re-enter alternate screen: {error}"));
    }
    if let Err(error) = enable_result {
        restore_errors.push(format!("failed to re-enable raw mode: {error}"));
    }

    if !restore_errors.is_empty() {
        return match launch_result {
            Ok(result) => Err(format!(
                "terminal restore failed after external editor session (exit={:?}): {}",
                result.exit_code,
                restore_errors.join("; ")
            )),
            Err(error) => Err(format!("{error}; {}", restore_errors.join("; "))),
        };
    }

    launch_result
}

fn launch_external_editor_process(
    editor_spec: &str,
    file_path: &Path,
) -> std::result::Result<ExternalEditorProcessResult, String> {
    let (program, args) = split_external_editor_command(editor_spec)
        .ok_or_else(|| "external editor command is empty".to_string())?;
    let status = ProcessCommand::new(&program)
        .args(args)
        .arg(file_path)
        .status()
        .map_err(|error| format!("failed to launch {program}: {error}"))?;
    Ok(ExternalEditorProcessResult {
        success: status.success(),
        exit_code: status.code(),
    })
}

fn split_external_editor_command(editor_spec: &str) -> Option<(String, Vec<String>)> {
    let mut parts = editor_spec.split_whitespace();
    let program = parts.next()?.to_string();
    let args = parts.map(ToOwned::to_owned).collect();
    Some((program, args))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TuiAction {
    Exit,
    Restart,
}

enum LoopAction {
    Continue,
    Exit,
    Restart,
}

pub fn run(config: &RuntimeConfig, bootstrap: &BootstrapState) -> Result<TuiAction> {
    let ui_state_path = ui_state::path_for_data_dir(&config.data_dir);
    let persisted_ui_state = load_persisted_ui_state(&ui_state_path);
    let should_persist_imap_defaults = config.imap.is_complete()
        && !persisted_ui_state
            .as_ref()
            .map(|state| state.imap_defaults_initialized)
            .unwrap_or(false);
    let initial_mailbox = persisted_ui_state
        .as_ref()
        .and_then(|state| state.active_mailbox.as_ref())
        .map(|mailbox| mailbox.trim())
        .filter(|mailbox| !mailbox.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| config.default_active_mailbox().to_string());
    let threads =
        mail_store::load_thread_rows_by_mailbox(&config.database_path, &initial_mailbox, 500)?;
    let mut terminal = setup_terminal()?;
    let guard = TerminalGuard;
    let mut state = if let Some(persisted) = persisted_ui_state {
        AppState::new_with_ui_state(threads, config.clone(), Some(persisted))
    } else {
        AppState::new(threads, config.clone())
    };
    if should_persist_imap_defaults {
        state.persist_ui_state();
    }
    if state.filtered_thread_indices.is_empty()
        && !state.recover_from_empty_active_mailbox("active mailbox has no local data")
    {
        state.status = "no synced thread data, run `courier sync` first".to_string();
    }
    state.start_startup_sync_if_enabled();

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
) -> Result<TuiAction> {
    loop {
        state.pump_startup_sync_events();

        if state.take_terminal_refresh_needed() {
            terminal.clear().map_err(|error| {
                CourierError::with_source(ErrorCode::Tui, "failed to clear terminal", error)
            })?;
            terminal.hide_cursor().map_err(|error| {
                CourierError::with_source(ErrorCode::Tui, "failed to hide terminal cursor", error)
            })?;
        }

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

                match handle_key_event(state, key) {
                    LoopAction::Continue => {}
                    LoopAction::Exit => return Ok(TuiAction::Exit),
                    LoopAction::Restart => return Ok(TuiAction::Restart),
                }
            }
        }
    }
}

fn handle_key_event(state: &mut AppState, key: KeyEvent) -> LoopAction {
    tracing::debug!(
        key = ?key,
        ui_page = ?state.ui_page,
        focus = ?state.focus,
        code_focus = ?state.code_focus,
        code_edit_mode = ?state.code_edit_mode,
        config_editor_open = state.config_editor.open,
        palette_open = state.palette.open,
        search_active = state.search.active,
        "user key event"
    );

    if state.config_editor.open {
        return handle_config_editor_key_event(state, key);
    }

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

    if state.is_code_edit_active() {
        return handle_code_edit_key_event(state, key);
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
        KeyCode::Char('e') if matches!(state.ui_page, UiPage::CodeBrowser) => {
            state.enter_code_edit_mode();
        }
        KeyCode::Char('E') if matches!(state.ui_page, UiPage::CodeBrowser) => {
            state.open_external_editor();
        }
        KeyCode::Tab => state.toggle_ui_page(),
        KeyCode::Char('j') => state.move_focus_previous(),
        KeyCode::Char('l') => state.move_focus_next(),
        KeyCode::Char('i') => state.move_up(),
        KeyCode::Char('k') => state.move_down(),
        KeyCode::Char(character)
            if matches!(state.ui_page, UiPage::Mail)
                && matches!(state.focus, Pane::Threads)
                && character.eq_ignore_ascii_case(&'a') =>
        {
            state.run_patch_action(patch_worker::PatchAction::Apply);
        }
        KeyCode::Char(character)
            if matches!(state.ui_page, UiPage::Mail)
                && matches!(state.focus, Pane::Threads)
                && character.eq_ignore_ascii_case(&'d') =>
        {
            state.run_patch_action(patch_worker::PatchAction::Download);
        }
        KeyCode::Char(character)
            if matches!(state.ui_page, UiPage::Mail)
                && matches!(state.focus, Pane::Threads)
                && character.eq_ignore_ascii_case(&'u') =>
        {
            state.run_patch_undo_action();
        }
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

fn handle_config_editor_key_event(state: &mut AppState, key: KeyEvent) -> LoopAction {
    match state.config_editor.mode {
        ConfigEditorMode::Browse => match key.code {
            KeyCode::Esc => state.close_config_editor(),
            KeyCode::Up | KeyCode::Char('i') => state.move_config_editor_up(),
            KeyCode::Down | KeyCode::Char('k') => state.move_config_editor_down(),
            KeyCode::Enter | KeyCode::Char('e') => state.start_config_editor_edit(),
            KeyCode::Tab => state.cycle_config_editor_value(),
            KeyCode::Char('x') => state.unset_selected_config_key(),
            _ => {}
        },
        ConfigEditorMode::Edit => match key.code {
            KeyCode::Esc => {
                state.config_editor.mode = ConfigEditorMode::Browse;
                state.config_editor.input.clear();
                state.status = "config edit cancelled".to_string();
            }
            KeyCode::Enter => state.save_config_editor_value(),
            KeyCode::Tab => state.cycle_config_editor_value(),
            KeyCode::Backspace => {
                state.config_editor.input.pop();
            }
            KeyCode::Char(character)
                if !key.modifiers.intersects(
                    KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER,
                ) =>
            {
                state.config_editor.input.push(character);
            }
            _ => {}
        },
    }

    LoopAction::Continue
}

fn handle_code_edit_key_event(state: &mut AppState, key: KeyEvent) -> LoopAction {
    match state.code_edit_mode {
        CodeEditMode::Browse => {}
        CodeEditMode::VimNormal => match key.code {
            KeyCode::Char('h') => state.move_code_edit_cursor_left(),
            KeyCode::Char('j') => state.move_code_edit_cursor_down(),
            KeyCode::Char('k') => state.move_code_edit_cursor_up(),
            KeyCode::Char('l') => state.move_code_edit_cursor_right(),
            KeyCode::Char('i') => {
                state.code_edit_mode = CodeEditMode::VimInsert;
                state.status = "insert mode".to_string();
            }
            KeyCode::Char('x') => {
                if !state.delete_code_edit_character() {
                    state.status = "nothing to delete".to_string();
                }
            }
            KeyCode::Char('s') => {
                let _ = state.save_code_edit_buffer();
            }
            KeyCode::Char('E') => {
                state.open_external_editor();
            }
            KeyCode::Char(':')
                if !key.modifiers.intersects(
                    KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER,
                ) =>
            {
                state.enter_code_edit_command_mode();
            }
            KeyCode::Esc => {
                if state.code_edit_dirty {
                    state.status = "unsaved changes, run :w, :wq, or :q!".to_string();
                } else {
                    let target = state
                        .code_edit_target
                        .as_ref()
                        .map(|path| path.display().to_string())
                        .unwrap_or_else(|| "<file>".to_string());
                    state.exit_code_edit_mode(format!("exit edit mode for {target}"));
                }
            }
            _ => {}
        },
        CodeEditMode::VimInsert => match key.code {
            KeyCode::Esc => {
                state.code_edit_mode = CodeEditMode::VimNormal;
                state.status = "normal mode".to_string();
            }
            KeyCode::Backspace => {
                if !state.backspace_code_edit_character() {
                    state.status = "nothing to delete".to_string();
                }
            }
            KeyCode::Enter => {
                let _ = state.insert_code_edit_newline();
            }
            KeyCode::Tab => {
                for character in PREVIEW_TAB_SPACES.chars() {
                    let _ = state.insert_code_edit_character(character);
                }
            }
            KeyCode::Char(character)
                if !key.modifiers.intersects(
                    KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER,
                ) =>
            {
                let _ = state.insert_code_edit_character(character);
            }
            _ => {}
        },
        CodeEditMode::VimCommand => match key.code {
            KeyCode::Esc => {
                state.code_edit_command_input.clear();
                state.code_edit_mode = CodeEditMode::VimNormal;
                state.status = "command cancelled".to_string();
            }
            KeyCode::Backspace => {
                state.code_edit_command_input.pop();
            }
            KeyCode::Enter => {
                state.execute_code_edit_command();
            }
            KeyCode::Char(character)
                if !key.modifiers.intersects(
                    KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER,
                ) =>
            {
                state.code_edit_command_input.push(character);
            }
            _ => {}
        },
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
            let raw_command = state.palette.input.trim().to_string();
            tracing::debug!(command = %raw_command, "user submitted command palette input");
            state.palette.input.clear();
            state.palette.clear_completion();

            if raw_command.is_empty() {
                state.status = "empty command".to_string();
                return LoopAction::Continue;
            }

            if let Some(local_command) = raw_command.strip_prefix('!') {
                run_palette_local_command(state, local_command);
                return LoopAction::Continue;
            }

            state.palette.clear_local_result();
            let command = raw_command.to_ascii_lowercase();
            match command.as_str() {
                "quit" | "exit" => return LoopAction::Exit,
                "restart" => return LoopAction::Restart,
                "help" => {
                    state.status = "commands: quit, exit, restart, help, sync [mailbox], config ..., vim, !<local shell command> | keys: j/l focus, i/k move, y/n enable, a apply, d download, u undo apply, e inline edit, E external vim".to_string();
                }
                value if value.split_whitespace().next() == Some("sync") => {
                    run_palette_sync(state, value);
                }
                value if value.split_whitespace().next() == Some("config") => {
                    run_palette_config(state, value);
                }
                "vim" => {
                    state.open_external_editor();
                }
                _ => {
                    state.status = format!("unknown command: {command}");
                }
            }
        }
        KeyCode::Backspace => {
            state.palette.input.pop();
            state.palette.clear_completion();
            if !state.palette.input.trim_start().starts_with('!') {
                state.palette.clear_local_result();
            }
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
            if !state.palette.input.trim_start().starts_with('!') {
                state.palette.clear_local_result();
            }
        }
        _ => {}
    }

    LoopAction::Continue
}

fn run_palette_local_command(state: &mut AppState, local_command: &str) {
    let local_command = local_command.trim();
    if local_command.is_empty() {
        state.palette.clear_local_result();
        state.status = "empty local command after !".to_string();
        return;
    }
    tracing::info!(
        op = "local_command",
        status = "started",
        command = %local_command
    );

    let cwd = match resolve_palette_local_workdir(state) {
        Ok(path) => path,
        Err(message) => {
            tracing::error!(
                op = "local_command",
                status = "failed",
                command = %local_command,
                error = %message
            );
            state.palette.clear_local_result();
            state.status = format!("local command setup failed: {message}");
            return;
        }
    };

    let output = ProcessCommand::new("bash")
        .arg("-lc")
        .arg(local_command)
        .current_dir(&cwd)
        .output();

    match output {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let output_text = render_local_command_output(&stdout, &stderr);
            let summary = first_non_empty_line(&stdout)
                .or_else(|| first_non_empty_line(&stderr))
                .unwrap_or_else(|| "<no output>".to_string());
            let exit_code = output
                .status
                .code()
                .map(|value| value.to_string())
                .unwrap_or_else(|| "signal".to_string());
            state.palette.last_local_result = Some(LocalCommandResult {
                command: local_command.to_string(),
                cwd: cwd.clone(),
                exit_code: exit_code.clone(),
                output: output_text,
            });

            if output.status.success() {
                state.status = format!(
                    "local command ok (exit={} cwd={}): {}",
                    exit_code,
                    cwd.display(),
                    summary
                );
                tracing::info!(
                    op = "local_command",
                    status = "succeeded",
                    command = local_command,
                    cwd = %cwd.display(),
                    exit_code = %exit_code
                );
            } else {
                state.status = format!(
                    "local command failed (exit={} cwd={}): {}",
                    exit_code,
                    cwd.display(),
                    summary
                );
                tracing::error!(
                    op = "local_command",
                    status = "failed",
                    command = local_command,
                    cwd = %cwd.display(),
                    exit_code = %exit_code
                );
            }
        }
        Err(error) => {
            tracing::error!(
                op = "local_command",
                status = "failed",
                command = %local_command,
                cwd = %cwd.display(),
                error = %error
            );
            state.palette.last_local_result = Some(LocalCommandResult {
                command: local_command.to_string(),
                cwd: cwd.clone(),
                exit_code: "spawn-error".to_string(),
                output: format!("{error}"),
            });
            state.status = format!(
                "failed to launch local command in {}: {}",
                cwd.display(),
                error
            );
        }
    }
}

fn render_local_command_output(stdout: &str, stderr: &str) -> String {
    let stdout_trimmed = stdout.trim_end();
    let stderr_trimmed = stderr.trim_end();
    match (stdout_trimmed.is_empty(), stderr_trimmed.is_empty()) {
        (true, true) => "<no output>".to_string(),
        (false, true) => stdout_trimmed.to_string(),
        (true, false) => format!("[stderr]\n{stderr_trimmed}"),
        (false, false) => format!("{stdout_trimmed}\n\n[stderr]\n{stderr_trimmed}"),
    }
}

fn short_commit_id(value: &str) -> String {
    value.chars().take(12).collect()
}

fn resolve_palette_local_workdir(state: &AppState) -> std::result::Result<PathBuf, String> {
    if let Some(path) = state.runtime.kernel_trees.first() {
        if !path.exists() {
            return Err(format!("[kernel].tree does not exist: {}", path.display()));
        }
        if !path.is_dir() {
            return Err(format!(
                "[kernel].tree is not a directory: {}",
                path.display()
            ));
        }
        return Ok(path.clone());
    }

    let home = env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "HOME is not set and [kernel].tree is not configured".to_string())?;
    if !home.exists() || !home.is_dir() {
        return Err(format!("home directory is unavailable: {}", home.display()));
    }

    Ok(home)
}

fn first_non_empty_line(value: &str) -> Option<String> {
    value
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(ToOwned::to_owned)
}

fn run_palette_sync(state: &mut AppState, command: &str) {
    tracing::debug!(command = %command, "user executed sync command from palette");
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
            vec![state.runtime.default_active_mailbox().to_string()]
        } else {
            enabled
        }
    };
    tracing::info!(
        op = "sync",
        status = "started",
        command = %command,
        mailboxes = %mailboxes.join(",")
    );

    let mut success = 0usize;
    let mut failed = 0usize;
    let mut total_fetched = 0usize;
    let mut total_inserted = 0usize;
    let mut total_updated = 0usize;
    let mut first_error: Option<String> = None;
    let total = mailboxes.len();

    for (index, mailbox) in mailboxes.into_iter().enumerate() {
        tracing::info!(
            op = "sync",
            status = "progress",
            phase = "started",
            mailbox = %mailbox,
            index = index + 1,
            total,
            completed = success + failed,
            succeeded = success,
            failed
        );
        let request = sync_worker::SyncRequest {
            mailbox: mailbox.clone(),
            fixture_dir: None,
            uidvalidity: None,
            reconnect_attempts: PALETTE_SYNC_RECONNECT_ATTEMPTS,
        };

        match run_sync_request_guarded(&state.runtime, request) {
            Ok(summary) => {
                success += 1;
                total_fetched += summary.fetched;
                total_inserted += summary.inserted;
                total_updated += summary.updated;
                tracing::info!(
                    op = "sync",
                    status = "succeeded",
                    phase = "finished",
                    mailbox = %mailbox,
                    index = index + 1,
                    total,
                    completed = success + failed,
                    succeeded = success,
                    failed,
                    fetched = summary.fetched,
                    inserted = summary.inserted,
                    updated = summary.updated
                );
            }
            Err(error) => {
                failed += 1;
                if first_error.is_none() {
                    first_error = Some(format!("{mailbox}: {error}"));
                }
                tracing::error!(
                    op = "sync",
                    status = "failed",
                    phase = "finished",
                    mailbox = %mailbox,
                    index = index + 1,
                    total,
                    completed = success + failed,
                    succeeded = success,
                    failed,
                    error = %error
                );
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

    let first_error_text = first_error
        .as_deref()
        .unwrap_or("unknown error")
        .to_string();

    state.status = if failed == 0 {
        format!(
            "sync ok: mailboxes={} fetched={} inserted={} updated={}",
            success, total_fetched, total_inserted, total_updated
        )
    } else if success == 0 {
        format!("sync failed: {}", first_error_text)
    } else {
        format!(
            "sync partial: ok={} failed={} fetched={} inserted={} updated={} first_error={}",
            success, failed, total_fetched, total_inserted, total_updated, first_error_text
        )
    };

    tracing::info!(
        op = "sync",
        status = if failed == 0 {
            "succeeded"
        } else if success == 0 {
            "failed"
        } else {
            "partial"
        },
        command = %command,
        success,
        failed,
        fetched = total_fetched,
        inserted = total_inserted,
        updated = total_updated,
        first_error = %first_error_text
    );

    if failed > 0 {
        tracing::error!(
            op = "sync",
            status = "failed",
            command = %command,
            success,
            failed,
            first_error = %first_error_text
        );
    }
}

fn run_palette_config(state: &mut AppState, command: &str) {
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
                Ok(rendered_value) => match reload_runtime_from_config(state) {
                    Ok(()) => {
                        state.status = format!("config updated: {key} = {rendered_value}");
                        tracing::info!(
                            op = "config.set",
                            status = "succeeded",
                            key = %key,
                            value = %rendered_value,
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
                        state.status =
                            format!("config file updated but runtime reload failed: {error}");
                    }
                },
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

fn remove_config_key_from_file(config_path: &Path, key: &str) -> std::result::Result<bool, String> {
    let mut table = read_config_table(config_path)?;
    let removed = remove_config_key(&mut table, key)?;
    if removed {
        write_config_table(config_path, &table)?;
    }
    Ok(removed)
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
        table.remove(key);
    }

    removed
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

fn config_editor_field_index(key: &str) -> Option<usize> {
    CONFIG_EDITOR_FIELDS
        .iter()
        .position(|field| field.key.eq_ignore_ascii_case(key))
}

fn reload_runtime_from_config(state: &mut AppState) -> std::result::Result<(), String> {
    let selected_path_hint = state.selected_kernel_tree_path();
    match crate::infra::config::load(Some(&state.runtime.config_path)) {
        Ok(runtime) => {
            let enabled_mailboxes: HashSet<String> =
                state.enabled_mailboxes().into_iter().collect();
            let active_mailbox = state.active_thread_mailbox.clone();
            state.runtime = runtime;
            state.ui_state_path = ui_state::path_for_data_dir(&state.runtime.data_dir);
            state.subscriptions = default_subscriptions(
                &state.runtime,
                &enabled_mailboxes,
                Some(active_mailbox.as_str()),
                state.runtime.imap.is_complete() && !state.imap_defaults_initialized,
            );
            if state.runtime.imap.is_complete() {
                state.imap_defaults_initialized = true;
            }
            if let Some(index) = state
                .subscriptions
                .iter()
                .position(|item| item.mailbox == state.active_thread_mailbox)
            {
                state.subscription_index = index;
                state.sync_subscription_row_to_selected_item();
            }
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
    if state.palette.input.trim_start().starts_with('!') {
        apply_local_palette_completion(state);
        return;
    }

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

fn apply_local_palette_completion(state: &mut AppState) {
    let Some((local_prefix, local_input)) = split_local_palette_input(&state.palette.input) else {
        state.palette.clear_completion();
        state.status = "invalid local command mode".to_string();
        return;
    };

    let input_before_completion = state.palette.input.clone();
    let context = parse_palette_completion_context(&local_input);
    let mut suggestions = local_completion_suggestions(state, &context);
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
    let completion_prefix = format!("{local_prefix}{}", context.prefix);

    if completion_values.len() == 1 {
        let candidate = completion_values[0].clone();
        state.palette.input = format!(
            "{}{}{}",
            completion_prefix,
            candidate,
            completion_suffix(&candidate)
        );
        state.palette.clear_completion();
        state.status = format!("completed: {candidate}");
        return;
    }

    let common_prefix = longest_common_prefix(&completion_values);
    if common_prefix.len() > context.active_token.len() {
        state.palette.input = format!("{completion_prefix}{common_prefix}");
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

fn completion_suffix(candidate: &str) -> &'static str {
    if candidate.ends_with('/') { "" } else { " " }
}

fn split_local_palette_input(input: &str) -> Option<(String, String)> {
    let trimmed = input.trim_start();
    if !trimmed.starts_with('!') {
        return None;
    }
    let leading_whitespace_len = input.len() - trimmed.len();
    let leading = &input[..leading_whitespace_len];
    let content = trimmed.strip_prefix('!')?.to_string();
    Some((format!("{leading}!"), content))
}

fn local_completion_suggestions(
    state: &AppState,
    context: &PaletteCompletionContext,
) -> Vec<PaletteSuggestion> {
    let token = context.active_token.as_str();
    let token_looks_like_path =
        token.contains('/') || token.starts_with('.') || token.starts_with('~');
    let Ok(workdir) = resolve_palette_local_workdir(state) else {
        return Vec::new();
    };

    if context.active_index == 0 && !token_looks_like_path {
        return local_command_completion_suggestions();
    }

    local_path_completion_suggestions(&workdir, token)
}

fn local_command_completion_suggestions() -> Vec<PaletteSuggestion> {
    let mut seen = HashSet::new();
    let mut suggestions = Vec::new();

    for builtin in ["cd", "echo", "pwd", "true", "false", "test"] {
        if seen.insert(builtin.to_string()) {
            suggestions.push(PaletteSuggestion {
                value: builtin.to_string(),
                description: Some("Shell builtin".to_string()),
            });
        }
    }

    if let Some(path_os) = env::var_os("PATH") {
        for directory in env::split_paths(&path_os) {
            let Ok(entries) = fs::read_dir(directory) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if !is_executable_path(&path) {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().to_string();
                if name.is_empty() || !seen.insert(name.clone()) {
                    continue;
                }
                suggestions.push(PaletteSuggestion {
                    value: name,
                    description: Some("Executable in PATH".to_string()),
                });
            }
        }
    }

    suggestions
}

fn local_path_completion_suggestions(base_dir: &Path, token: &str) -> Vec<PaletteSuggestion> {
    if token == "~" {
        return vec![PaletteSuggestion {
            value: "~/".to_string(),
            description: Some("Home directory".to_string()),
        }];
    }

    let (dir_part, entry_prefix) = token
        .rsplit_once('/')
        .map(|(left, right)| (Some(left), right))
        .unwrap_or((None, token));

    let (search_dir, display_prefix) = match dir_part {
        Some(part) if token.starts_with('/') && part.is_empty() => {
            (PathBuf::from("/"), "/".to_string())
        }
        Some("~") => match env::var("HOME") {
            Ok(home) => (PathBuf::from(home), "~/".to_string()),
            Err(_) => return Vec::new(),
        },
        Some(part) if part.starts_with("~/") => match env::var("HOME") {
            Ok(home) => {
                let suffix = part.strip_prefix("~/").unwrap_or_default();
                (PathBuf::from(home).join(suffix), format!("{part}/"))
            }
            Err(_) => return Vec::new(),
        },
        Some(part) => {
            let search = if Path::new(part).is_absolute() {
                PathBuf::from(part)
            } else {
                base_dir.join(part)
            };
            (search, format!("{part}/"))
        }
        None => (base_dir.to_path_buf(), String::new()),
    };

    let Ok(entries) = fs::read_dir(search_dir) else {
        return Vec::new();
    };

    let mut suggestions = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with(entry_prefix) {
            continue;
        }
        let is_dir = entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false);
        let mut value = format!("{display_prefix}{name}");
        if is_dir {
            value.push('/');
        }
        suggestions.push(PaletteSuggestion {
            value,
            description: Some(if is_dir {
                "Directory".to_string()
            } else {
                "Path".to_string()
            }),
        });
    }
    suggestions
}

fn is_executable_path(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        fs::metadata(path)
            .map(|metadata| metadata.is_file() && (metadata.permissions().mode() & 0o111 != 0))
            .unwrap_or(false)
    }

    #[cfg(not(unix))]
    {
        path.is_file()
    }
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
    candidates.push(state.runtime.source_mailbox.clone());
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
    let header = if let Some(progress) = state.startup_sync_progress_text() {
        format!("{header} | {progress}")
    } else {
        header
    };
    let header = sanitize_inline_ui_text(&header);
    let header_widget =
        Paragraph::new(header).style(Style::default().fg(Color::Black).bg(Color::Cyan));
    frame.render_widget(header_widget, areas[0]);

    match state.ui_page {
        UiPage::Mail => {
            let panes = mail_page_panes(areas[1]);
            draw_subscriptions(frame, panes[0], state);
            draw_threads(frame, panes[1], state);
            draw_preview(frame, panes[2], state, config);
        }
        UiPage::CodeBrowser => {
            draw_code_browser_page(frame, areas[1], state);
        }
    }

    let shortcuts_text = match state.ui_page {
        UiPage::Mail => "/ search | Tab page | : palette | Enter open/toggle",
        UiPage::CodeBrowser if state.is_code_edit_active() => {
            "Esc normal/exit | h/j/k/l move | i insert | x delete | s save | E external vim | :w :q :q! :wq :vim"
        }
        UiPage::CodeBrowser => {
            "Tab page | : palette | Enter expand/collapse | e inline edit | E external vim"
        }
    };
    let sync_progress_text = state
        .startup_sync_progress_text()
        .map(|value| sanitize_inline_ui_text(&value));
    let footer_sections = if let Some(progress_text) = sync_progress_text.as_ref() {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(shortcuts_text.chars().count() as u16),
                Constraint::Length(progress_text.chars().count().min(48) as u16),
                Constraint::Min(1),
            ])
            .split(areas[2])
    } else {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(shortcuts_text.chars().count() as u16),
                Constraint::Min(1),
            ])
            .split(areas[2])
    };

    let shortcuts =
        Paragraph::new(shortcuts_text).style(Style::default().fg(Color::White).bg(Color::DarkGray));
    frame.render_widget(shortcuts, footer_sections[0]);

    let status_area = if let Some(progress_text) = sync_progress_text {
        let progress = Paragraph::new(progress_text)
            .style(Style::default().fg(Color::Yellow).bg(Color::DarkGray));
        frame.render_widget(progress, footer_sections[1]);
        footer_sections[2]
    } else {
        footer_sections[1]
    };

    let status_line = format!("status: {}", sanitize_inline_ui_text(&state.status));
    let status = Paragraph::new(status_line)
        .alignment(Alignment::Right)
        .style(Style::default().fg(Color::White).bg(Color::DarkGray));
    frame.render_widget(status, status_area);

    if state.palette.open {
        draw_command_palette(frame, state);
    }
    if state.search.active {
        draw_search_overlay(frame, state);
    }
    if state.config_editor.open {
        draw_config_editor(frame, state);
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

fn mail_page_panes(area: Rect) -> [Rect; 3] {
    if area.width == 0 {
        return [area, area, area];
    }

    let preview_width = area.width.min(PREVIEW_PANE_FIXED_WIDTH);
    let left_width = area.width.saturating_sub(preview_width);
    let preview = Rect {
        x: area.x + left_width,
        y: area.y,
        width: preview_width,
        height: area.height,
    };

    if left_width == 0 {
        let empty = Rect {
            x: area.x,
            y: area.y,
            width: 0,
            height: area.height,
        };
        return [empty, empty, preview];
    }

    let left = Rect {
        x: area.x,
        y: area.y,
        width: left_width,
        height: area.height,
    };
    let left_panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Ratio(1, 4), Constraint::Ratio(3, 4)])
        .split(left);

    [left_panes[0], left_panes[1], preview]
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
    frame.render_widget(Clear, area);
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
    frame.render_widget(Clear, area);
    let title = if state.is_code_edit_active() {
        let dirty = if state.code_edit_dirty { "*" } else { "-" };
        format!(
            "Source Preview [{} dirty:{}]",
            state.code_edit_mode.label(),
            dirty
        )
    } else {
        "Source Preview".to_string()
    };
    let block = panel_block_with_title(&title, state.code_focus == CodePaneFocus::Source);
    let inner_area = block.inner(area);
    frame.render_widget(block, area);
    frame.render_widget(Clear, inner_area);

    let paragraph =
        Paragraph::new(load_code_source_preview(state)).scroll((state.code_preview_scroll, 0));
    frame.render_widget(paragraph, inner_area);

    if let Some(cursor_position) = code_edit_cursor_position(state, inner_area) {
        frame.set_cursor_position(cursor_position);
    }
}

fn subscription_line(
    item: &SubscriptionItem,
    startup_sync_status: Option<StartupSyncMailboxStatus>,
) -> String {
    let marker = if item.enabled { "y" } else { "n" };
    let suffix = startup_sync_status
        .map(StartupSyncMailboxStatus::ui_suffix)
        .unwrap_or("");
    format!("[{marker}] {}{suffix}", item.label)
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
    runtime: &RuntimeConfig,
    enabled_mailboxes: &HashSet<String>,
    active_mailbox: Option<&str>,
    allow_default_my_inbox: bool,
) -> Vec<SubscriptionItem> {
    let mut items: Vec<SubscriptionItem> = VGER_SUBSCRIPTIONS
        .iter()
        .map(|entry| SubscriptionItem {
            mailbox: entry.mailbox.to_string(),
            label: entry.mailbox.to_string(),
            enabled: enabled_mailboxes.contains(entry.mailbox),
        })
        .collect();

    if runtime.imap.is_complete() {
        let enable_my_inbox = enabled_mailboxes.contains(IMAP_INBOX_MAILBOX)
            || (allow_default_my_inbox && !enabled_mailboxes.contains(IMAP_INBOX_MAILBOX));
        items.insert(
            0,
            SubscriptionItem {
                mailbox: IMAP_INBOX_MAILBOX.to_string(),
                label: MY_INBOX_LABEL.to_string(),
                enabled: enable_my_inbox,
            },
        );
    }

    if items
        .iter()
        .all(|item| item.mailbox != runtime.source_mailbox)
    {
        items.insert(
            0,
            SubscriptionItem {
                mailbox: runtime.source_mailbox.clone(),
                label: runtime.source_mailbox.clone(),
                enabled: enabled_mailboxes.contains(runtime.source_mailbox.as_str()),
            },
        );
    }

    for mailbox in enabled_mailboxes {
        if items.iter().any(|item| item.mailbox == *mailbox) {
            continue;
        }
        items.push(SubscriptionItem {
            mailbox: mailbox.clone(),
            label: if mailbox.eq_ignore_ascii_case(IMAP_INBOX_MAILBOX) {
                MY_INBOX_LABEL.to_string()
            } else {
                mailbox.clone()
            },
            enabled: true,
        });
    }

    if let Some(mailbox) = active_mailbox
        && !mailbox.is_empty()
        && items.iter().all(|item| item.mailbox != mailbox)
    {
        items.push(SubscriptionItem {
            mailbox: mailbox.to_string(),
            label: if mailbox.eq_ignore_ascii_case(IMAP_INBOX_MAILBOX) {
                MY_INBOX_LABEL.to_string()
            } else {
                mailbox.to_string()
            },
            enabled: enabled_mailboxes.contains(mailbox),
        });
    }

    items.sort_by(|left, right| {
        right
            .enabled
            .cmp(&left.enabled)
            .then_with(|| left.label.cmp(&right.label))
            .then_with(|| left.mailbox.cmp(&right.mailbox))
    });

    items
}

fn draw_threads(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let max_thread_line_chars = thread_line_max_chars(area);
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
                ListItem::new(thread_group_line(
                    row.thread_id,
                    visible_count,
                    state.series_summaries.get(&row.thread_id),
                ))
                .style(
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD),
                ),
            );
        }

        if position == state.thread_index {
            selected = Some(items.len());
        }
        items.push(ListItem::new(thread_line(row, max_thread_line_chars)));
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

fn thread_group_line(
    thread_id: i64,
    visible_count: usize,
    series: Option<&patch_worker::SeriesSummary>,
) -> String {
    let noun = if visible_count == 1 { "msg" } else { "msgs" };
    let mut line = format!("Thread {thread_id} ({visible_count} {noun})");
    if let Some(series) = series {
        line.push_str(&format!(
            " | v{} {}/{} | integrity={} | status={}",
            series.version,
            series.present_count(),
            series.expected_total,
            series.integrity.short_label(),
            series.status_label()
        ));
    }
    line
}

fn thread_line_max_chars(area: Rect) -> usize {
    let available = area.width.saturating_sub(2) as usize;
    available.min(THREAD_LINE_MAX_CHARS)
}

fn thread_line(row: &ThreadRow, max_chars: usize) -> String {
    let max_chars = max_chars.min(THREAD_LINE_MAX_CHARS);
    let indent = "  ".repeat(row.depth as usize);
    let subject = if row.subject.trim().is_empty() {
        "(no subject)"
    } else {
        row.subject.trim()
    };
    truncate_with_ellipsis(&format!("{indent}{subject}"), max_chars)
}

fn truncate_with_ellipsis(value: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }

    let value_len = value.chars().count();
    if value_len <= max_chars {
        return value.to_string();
    }

    if max_chars <= 3 {
        return ".".repeat(max_chars);
    }

    let mut truncated = String::new();
    for ch in value.chars().take(max_chars - 3) {
        truncated.push(ch);
    }
    truncated.push_str("...");
    truncated
}

fn sanitize_inline_ui_text(value: &str) -> String {
    let mut sanitized = String::with_capacity(value.len());
    let mut pending_space = false;

    for character in value.chars() {
        if character.is_control() || character.is_whitespace() {
            pending_space = !sanitized.is_empty();
            continue;
        }

        if pending_space {
            sanitized.push(' ');
            pending_space = false;
        }
        sanitized.push(character);
    }

    sanitized.trim().to_string()
}

fn char_to_byte_index(value: &str, char_index: usize) -> usize {
    if char_index == 0 {
        return 0;
    }

    value
        .char_indices()
        .nth(char_index)
        .map(|(byte_index, _)| byte_index)
        .unwrap_or(value.len())
}

fn draw_preview(frame: &mut Frame<'_>, area: Rect, state: &AppState, config: &RuntimeConfig) {
    let (warning, preview) = if let Some(thread) = state.selected_thread() {
        let mut sections = Vec::new();
        if let Some(series_details) = load_series_preview(state, config, thread.thread_id) {
            sections.push(series_details);
        }
        let mail_preview = load_mail_preview(thread);
        sections.push(mail_preview.content);
        (mail_preview.warning, sections.join("\n\n"))
    } else {
        (
            None,
            format!(
                "No synced thread data\n\nRun:\n  courier sync --fixture-dir <DIR>\n\nConfig: {}\nDatabase: {}",
                config.config_path.display(),
                config.database_path.display(),
            ),
        )
    };

    let block = panel_block(Pane::Preview, state.focus);
    let inner_area = block.inner(area);
    frame.render_widget(block, area);
    frame.render_widget(Clear, inner_area);

    let content_area = if let Some(warning_text) = warning {
        let warning_height = warning_text
            .lines()
            .count()
            .min(inner_area.height.saturating_sub(1) as usize) as u16;
        let sections = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(warning_height), Constraint::Min(1)])
            .split(inner_area);
        let warning = Paragraph::new(Text::from(warning_text))
            .style(
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
            .wrap(Wrap { trim: false });
        frame.render_widget(warning, sections[0]);
        sections[1]
    } else {
        inner_area
    };

    let paragraph = Paragraph::new(preview)
        .scroll((state.preview_scroll, 0))
        .wrap(Wrap { trim: false });

    frame.render_widget(paragraph, content_area);
}

fn load_series_preview(state: &AppState, config: &RuntimeConfig, thread_id: i64) -> Option<String> {
    let series = state.series_summaries.get(&thread_id)?;
    let mut lines = vec![
        format!(
            "Series: v{} {}/{} | integrity={} | status={}",
            series.version,
            series.present_count(),
            series.expected_total,
            series.integrity.short_label(),
            series.status_label()
        ),
        format!("Anchor: <{}>", series.anchor_message_id),
    ];

    if !series.missing_seq.is_empty() {
        lines.push(format!(
            "Missing: {}",
            series
                .missing_seq
                .iter()
                .map(|value| value.to_string())
                .collect::<Vec<_>>()
                .join(",")
        ));
    }
    if !series.duplicate_seq.is_empty() {
        lines.push(format!(
            "Duplicate: {}",
            series
                .duplicate_seq
                .iter()
                .map(|value| value.to_string())
                .collect::<Vec<_>>()
                .join(",")
        ));
    }

    match patch_worker::load_latest_report(
        &config.database_path,
        &state.active_thread_mailbox,
        thread_id,
    ) {
        Ok(Some(report)) => {
            if let Some(summary) = report.last_summary.as_deref() {
                lines.push(format!("Last run: {summary}"));
            }
            if let Some(exit_code) = report.last_exit_code {
                lines.push(format!("Exit code: {exit_code}"));
            }
            if let Some(command) = report.last_command.as_deref() {
                lines.push(format!("Command: {command}"));
            }
            if let Some(error) = report.last_error.as_deref() {
                lines.push(format!("Error: {error}"));
            }
        }
        Ok(None) => {}
        Err(error) => {
            lines.push(format!("Series report load failed: {error}"));
        }
    }

    Some(lines.join("\n"))
}

fn load_code_source_preview(state: &AppState) -> String {
    if state.is_code_edit_active() {
        return render_code_edit_preview(state);
    }

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

fn render_code_edit_preview(state: &AppState) -> String {
    let target = state
        .code_edit_target
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "<unknown file>".to_string());
    let dirty = if state.code_edit_dirty { "yes" } else { "no" };
    let mut lines = vec![
        format!("File: {target}"),
        format!(
            "Mode: {} | dirty: {} | cursor: {}:{}",
            state.code_edit_mode.label(),
            dirty,
            state.code_edit_cursor_row + 1,
            state.code_edit_cursor_col + 1
        ),
        "Use h/j/k/l move, i insert, x delete, s save, E external vim, : command".to_string(),
        String::new(),
    ];

    if state.code_edit_buffer.is_empty() {
        lines.push("<empty file>".to_string());
    } else {
        for (index, line) in state.code_edit_buffer.iter().enumerate() {
            let marker = if index == state.code_edit_cursor_row {
                ">"
            } else {
                " "
            };
            let rendered_line = sanitize_source_preview_text(line);
            lines.push(format!("{:>4}{marker} {}", index + 1, rendered_line));
        }
    }

    if matches!(state.code_edit_mode, CodeEditMode::VimCommand) {
        lines.push(String::new());
        lines.push(format!(":{}", state.code_edit_command_input));
    }

    lines.join("\n")
}

fn code_edit_cursor_position(state: &AppState, inner_area: Rect) -> Option<(u16, u16)> {
    if !state.is_code_edit_active() || inner_area.width == 0 || inner_area.height == 0 {
        return None;
    }

    let (logical_row, logical_col) = if matches!(state.code_edit_mode, CodeEditMode::VimCommand) {
        (
            code_edit_command_line_logical_row(state),
            1 + state.code_edit_command_input.chars().count(),
        )
    } else {
        let row = state
            .code_edit_cursor_row
            .min(state.code_edit_buffer.len().saturating_sub(1));
        let line = state
            .code_edit_buffer
            .get(row)
            .map(String::as_str)
            .unwrap_or_default();
        let column = state.code_edit_cursor_col.min(line.chars().count());
        (
            code_edit_source_line_logical_row(row),
            code_edit_source_line_prefix_width(row) + display_column(line, column),
        )
    };

    let scroll = state.code_preview_scroll as usize;
    if logical_row < scroll {
        return None;
    }
    let visible_row = logical_row - scroll;
    if visible_row >= inner_area.height as usize {
        return None;
    }

    let clamped_col = logical_col.min(inner_area.width.saturating_sub(1) as usize);
    Some((
        inner_area.x.saturating_add(clamped_col as u16),
        inner_area.y.saturating_add(visible_row as u16),
    ))
}

fn code_edit_source_line_logical_row(buffer_row: usize) -> usize {
    4 + buffer_row
}

fn code_edit_source_line_prefix_width(buffer_row: usize) -> usize {
    let number_width = ((buffer_row + 1).to_string().chars().count()).max(4);
    number_width + 2
}

fn code_edit_command_line_logical_row(state: &AppState) -> usize {
    4 + state.code_edit_buffer.len() + 1
}

fn display_column(line: &str, char_col: usize) -> usize {
    let mut display_col = 0usize;
    for character in line.chars().take(char_col) {
        match character {
            '\t' => {
                display_col += PREVIEW_TAB_SPACES.chars().count();
            }
            '\n' | '\r' => {}
            _ if character.is_control() => {}
            _ => {
                display_col += 1;
            }
        }
    }
    display_col
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

fn sanitize_source_preview_text(input: &str) -> String {
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
        "Tab: complete (built-in + !local)  Enter: execute  !<cmd>: local shell in [kernel].tree (or ~)  Esc: close",
    );
    frame.render_widget(hints, sections[1]);

    let show_local_result = state.palette.last_local_result.is_some()
        && (state.palette.input.trim().is_empty()
            || state.palette.input.trim_start().starts_with('!'));

    if show_local_result {
        if let Some(result) = state.palette.last_local_result.as_ref() {
            let header = Paragraph::new(format!(
                "Local Result: !{} | exit={} | cwd={}",
                result.command,
                result.exit_code,
                result.cwd.display()
            ));
            frame.render_widget(header, sections[2]);

            let output = Paragraph::new(result.output.clone())
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Gray)),
                )
                .wrap(Wrap { trim: false });
            frame.render_widget(output, sections[3]);
        }
    } else {
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
}

fn draw_config_editor(frame: &mut Frame<'_>, state: &AppState) {
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
    if query.starts_with('!') {
        return Vec::new();
    }
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
    use std::path::{Path, PathBuf};
    use std::sync::mpsc;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::layout::{Constraint, Direction, Layout, Position, Rect};

    use crate::infra::bootstrap::BootstrapState;
    use crate::infra::config::{IMAP_INBOX_MAILBOX, RuntimeConfig};
    use crate::infra::db;
    use crate::infra::db::DatabaseState;
    use crate::infra::mail_parser;
    use crate::infra::mail_store::{self, IncomingMail, SyncBatch, ThreadRow};
    use crate::infra::ui_state::UiState;

    use super::preview::preview_warning_message;
    use super::{
        AppState, CodeEditMode, CodePaneFocus, ExternalEditorProcessResult, LoopAction,
        MY_INBOX_LABEL, Pane, StartupSyncEvent, StartupSyncMailboxStatus, StartupSyncState,
        SubscriptionItem, UiPage, catch_sync_panic, code_edit_cursor_position, draw,
        extract_mail_body_preview, extract_mail_preview, handle_key_event,
        is_palette_open_shortcut, is_palette_toggle, load_source_file_preview, mail_page_panes,
        matching_commands, pick_external_editor, resolve_palette_local_workdir,
        run_external_editor_session_with, sanitize_inline_ui_text, subscription_line, thread_line,
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

    fn test_runtime_in(root: PathBuf) -> RuntimeConfig {
        RuntimeConfig {
            config_path: root.join("config.toml"),
            data_dir: root.join("data"),
            database_path: root.join("data/courier.db"),
            raw_mail_dir: root.join("data/raw"),
            patch_dir: root.join("data/patches"),
            log_dir: root.join("data/logs"),
            b4_path: None,
            log_filter: "info".to_string(),
            source_mailbox: "inbox".to_string(),
            imap: crate::infra::config::ImapConfig::default(),
            lore_base_url: "https://lore.kernel.org".to_string(),
            startup_sync: true,
            kernel_trees: Vec::new(),
        }
    }

    fn test_runtime() -> RuntimeConfig {
        test_runtime_in(PathBuf::from("/tmp/courier-ui-test"))
    }

    fn test_runtime_with_kernel_tree(tree: PathBuf) -> RuntimeConfig {
        let mut runtime = test_runtime();
        runtime.kernel_trees = vec![tree];
        runtime
    }

    fn test_runtime_with_imap_in(root: PathBuf) -> RuntimeConfig {
        let mut runtime = test_runtime_in(root);
        runtime.imap = crate::infra::config::ImapConfig {
            email: Some("me@example.com".to_string()),
            user: Some("imap-user".to_string()),
            pass: Some("imap-pass".to_string()),
            server: Some("imap.example.com".to_string()),
            server_port: Some(993),
            encryption: Some(crate::infra::config::ImapEncryption::Tls),
            proxy: None,
        };
        runtime
    }

    fn test_runtime_with_imap() -> RuntimeConfig {
        test_runtime_with_imap_in(PathBuf::from("/tmp/courier-ui-test"))
    }

    fn seed_mailbox_thread(
        db_path: &Path,
        mailbox: &str,
        uid: u32,
        message_id: &str,
        subject: &str,
    ) {
        fs::create_dir_all(db_path.parent().expect("db parent")).expect("create db parent");
        db::initialize(db_path).expect("initialize db");
        let batch = SyncBatch {
            mailbox: mailbox.to_string(),
            uidvalidity: 1,
            highest_uid: uid,
            highest_modseq: Some(uid as u64),
            mails: vec![IncomingMail {
                mailbox: mailbox.to_string(),
                uid,
                modseq: Some(uid as u64),
                flags: vec!["Seen".to_string()],
                raw_path: PathBuf::from(format!("/tmp/{mailbox}-{uid}.eml")),
                parsed: mail_parser::parse_headers(
                    format!(
                        "Message-ID: <{message_id}>\nSubject: {subject}\nFrom: Alice <alice@example.com>\n\nbody\n"
                    )
                    .as_bytes(),
                    format!("synthetic-{mailbox}-{uid}@local"),
                ),
            }],
        };

        mail_store::apply_sync_batch(db_path, batch).expect("apply mailbox sync batch");
    }

    fn startup_sync_state(mailboxes: &[(&str, StartupSyncMailboxStatus)]) -> StartupSyncState {
        let (_sender, receiver) = mpsc::channel();
        StartupSyncState {
            receiver,
            mailbox_order: mailboxes
                .iter()
                .map(|(mailbox, _)| (*mailbox).to_string())
                .collect(),
            mailboxes: mailboxes
                .iter()
                .map(|(mailbox, status)| ((*mailbox).to_string(), *status))
                .collect(),
            total: mailboxes.len(),
            completed: mailboxes
                .iter()
                .filter(|(_, status)| {
                    matches!(
                        status,
                        StartupSyncMailboxStatus::Finished | StartupSyncMailboxStatus::Failed
                    )
                })
                .count(),
            succeeded: mailboxes
                .iter()
                .filter(|(_, status)| matches!(status, StartupSyncMailboxStatus::Finished))
                .count(),
            failed: mailboxes
                .iter()
                .filter(|(_, status)| matches!(status, StartupSyncMailboxStatus::Failed))
                .count(),
        }
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
    fn startup_sync_is_not_started_when_disabled_in_config() {
        let mut runtime = test_runtime();
        runtime.startup_sync = false;
        let mut state = AppState::new(vec![], runtime);

        state.start_startup_sync_if_enabled();

        assert!(state.startup_sync.is_none());
    }

    #[test]
    fn startup_sync_progress_summary_renders_counts_and_running_mailbox() {
        let sync_state = startup_sync_state(&[
            ("INBOX", StartupSyncMailboxStatus::InFlight),
            ("io-uring", StartupSyncMailboxStatus::Pending),
            ("kvm", StartupSyncMailboxStatus::Finished),
        ]);

        assert_eq!(
            sync_state.progress_summary(),
            "1/3 ok=1 fail=0 queued=1 running=INBOX"
        );
        assert_eq!(
            sync_state.mailbox_states_display(),
            "INBOX:syncing io-uring:queued kvm:done"
        );
    }

    fn external_editor_mock_success(
        _editor: &str,
        file_path: &Path,
    ) -> std::result::Result<ExternalEditorProcessResult, String> {
        fs::write(file_path, "externally edited\n")
            .map_err(|error| format!("failed to write fixture: {error}"))?;
        Ok(ExternalEditorProcessResult {
            success: true,
            exit_code: Some(0),
        })
    }

    fn external_editor_mock_failure(
        _editor: &str,
        _file_path: &Path,
    ) -> std::result::Result<ExternalEditorProcessResult, String> {
        Err("mock launch failure".to_string())
    }

    #[test]
    fn empty_query_returns_all_palette_commands() {
        let all = matching_commands("");
        assert_eq!(all.len(), 7);
        assert_eq!(all[0].name, "config");
        assert_eq!(all[1].name, "exit");
        assert_eq!(all[2].name, "help");
        assert_eq!(all[3].name, "quit");
        assert_eq!(all[4].name, "restart");
        assert_eq!(all[5].name, "sync");
        assert_eq!(all[6].name, "vim");
    }

    #[test]
    fn prefix_matches_rank_before_fuzzy_matches() {
        let commands = matching_commands("ex");
        assert_eq!(commands[0].name, "exit");
    }

    #[test]
    fn bang_mode_is_not_matched_as_builtin_command() {
        let commands = matching_commands("!pwd");
        assert!(commands.is_empty());
    }

    #[test]
    fn external_editor_selection_prefers_visual_then_editor_then_vim() {
        assert_eq!(
            pick_external_editor(Some("nvim"), Some("vim")),
            "nvim".to_string()
        );
        assert_eq!(
            pick_external_editor(Some("  "), Some("hx")),
            "hx".to_string()
        );
        assert_eq!(pick_external_editor(None, Some("nano")), "nano".to_string());
        assert_eq!(pick_external_editor(None, None), "vim".to_string());
    }

    #[test]
    fn external_editor_session_restores_terminal_after_editor_exit() {
        use std::cell::RefCell;
        use std::rc::Rc;

        let steps: Rc<RefCell<Vec<&'static str>>> = Rc::new(RefCell::new(Vec::new()));
        let result = run_external_editor_session_with(
            "vim",
            Path::new("/tmp/demo.rs"),
            {
                let steps = steps.clone();
                move || {
                    steps.borrow_mut().push("disable_raw");
                    Ok(())
                }
            },
            {
                let steps = steps.clone();
                move || {
                    steps.borrow_mut().push("leave_alt");
                    Ok(())
                }
            },
            {
                let steps = steps.clone();
                move |_, _| {
                    steps.borrow_mut().push("launch");
                    Ok(ExternalEditorProcessResult {
                        success: true,
                        exit_code: Some(0),
                    })
                }
            },
            {
                let steps = steps.clone();
                move || {
                    steps.borrow_mut().push("enter_alt");
                    Ok(())
                }
            },
            {
                let steps = steps.clone();
                move || {
                    steps.borrow_mut().push("enable_raw");
                    Ok(())
                }
            },
        )
        .expect("external editor session should succeed");

        assert!(result.success);
        assert_eq!(
            *steps.borrow(),
            vec![
                "disable_raw",
                "leave_alt",
                "launch",
                "enter_alt",
                "enable_raw"
            ]
        );
    }

    #[test]
    fn mail_page_layout_keeps_preview_at_fixed_80_columns() {
        let panes = mail_page_panes(Rect::new(0, 0, 180, 20));

        assert_eq!(panes[2].width, 80);
        assert_eq!(panes[2].x, 100);
        assert_eq!(panes[0].width, 25);
        assert_eq!(panes[1].width, 75);
        assert_eq!(panes[0].width + panes[1].width + panes[2].width, 180);
    }

    #[test]
    fn mail_page_layout_falls_back_to_available_width_when_terminal_is_narrow() {
        let panes = mail_page_panes(Rect::new(0, 0, 60, 20));

        assert_eq!(panes[2].width, 60);
        assert_eq!(panes[0].width, 0);
        assert_eq!(panes[1].width, 0);
    }

    #[test]
    fn subscription_line_shows_marker_and_mailbox_name_only() {
        let enabled = SubscriptionItem {
            mailbox: "io-uring".to_string(),
            label: "io-uring".to_string(),
            enabled: true,
        };
        let disabled = SubscriptionItem {
            mailbox: "linux-mm".to_string(),
            label: "linux-mm".to_string(),
            enabled: false,
        };

        assert_eq!(subscription_line(&enabled, None), "[y] io-uring");
        assert_eq!(subscription_line(&disabled, None), "[n] linux-mm");
    }

    #[test]
    fn subscription_line_shows_sync_suffix_when_progress_is_active() {
        let enabled = SubscriptionItem {
            mailbox: "INBOX".to_string(),
            label: "My Inbox".to_string(),
            enabled: true,
        };

        assert_eq!(
            subscription_line(&enabled, Some(StartupSyncMailboxStatus::Pending)),
            "[y] My Inbox [queued]"
        );
        assert_eq!(
            subscription_line(&enabled, Some(StartupSyncMailboxStatus::InFlight)),
            "[y] My Inbox [sync]"
        );
    }

    #[test]
    fn thread_line_hides_sender() {
        let row = sample_thread("thread subject", "x@example.com", 0);
        let line = thread_line(&row, 120);

        assert_eq!(line, "thread subject");
        assert!(!line.contains("alice@example.com"));
    }

    #[test]
    fn thread_line_truncates_by_max_chars_and_available_width() {
        let long_subject = "x".repeat(240);
        let row = sample_thread(&long_subject, "x@example.com", 0);

        let line_capped_at_120 = thread_line(&row, 200);
        assert_eq!(
            line_capped_at_120.chars().count(),
            super::THREAD_LINE_MAX_CHARS
        );
        assert!(line_capped_at_120.ends_with("..."));

        let line_capped_by_width = thread_line(&row, 30);
        assert_eq!(line_capped_by_width.chars().count(), 30);
        assert!(line_capped_by_width.ends_with("..."));
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
    fn command_palette_restart_requests_tui_restart() {
        let mut state = AppState::new(vec![], test_runtime());
        state.palette.open = true;
        state.palette.input = "restart".to_string();

        let action = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        );
        assert!(matches!(action, LoopAction::Restart));
    }

    #[test]
    fn command_palette_help_includes_keyboard_shortcuts() {
        let mut state = AppState::new(vec![], test_runtime());
        state.palette.open = true;
        state.palette.input = "help".to_string();

        let action = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        );
        assert!(matches!(action, LoopAction::Continue));
        assert!(state.status.contains("keys:"));
        assert!(state.status.contains("j/l focus"));
        assert!(state.status.contains("i/k move"));
        assert!(state.status.contains("y/n enable"));
        assert!(state.status.contains("a apply"));
        assert!(state.status.contains("d download"));
        assert!(state.status.contains("u undo apply"));
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
        assert_eq!(state.runtime.source_mailbox, "io-uring");

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
    fn config_command_opens_visual_editor() {
        let mut state = AppState::new(vec![], test_runtime());
        state.palette.open = true;
        state.palette.input = "config".to_string();

        let action = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        );

        assert!(matches!(action, LoopAction::Continue));
        assert!(state.config_editor.open);
        assert!(!state.palette.open);
        assert_eq!(state.selected_config_editor_field().key, "source.mailbox");
    }

    #[test]
    fn config_editor_saves_selected_value() {
        let root = temp_dir("config-editor-save");
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

        state.open_config_editor(Some("source.mailbox"));
        state.start_config_editor_edit();
        state.config_editor.input = "io-uring".to_string();

        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        );

        assert_eq!(state.runtime.source_mailbox, "io-uring");
        assert!(!state.config_editor.open || state.config_editor.input.is_empty());
        let persisted = fs::read_to_string(&config_path).expect("read config file");
        assert!(persisted.contains("mailbox = \"io-uring\""));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn config_editor_tab_cycles_boolean_presets() {
        let root = temp_dir("config-editor-toggle");
        let config_path = root.join("courier-config.toml");
        fs::write(
            &config_path,
            r#"
[ui]
startup_sync = true
"#,
        )
        .expect("write config file");

        let mut runtime = test_runtime();
        runtime.config_path = config_path.clone();
        let mut state = AppState::new(vec![], runtime);

        state.open_config_editor(Some("ui.startup_sync"));
        let _ = handle_key_event(&mut state, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));

        assert!(!state.runtime.startup_sync);
        let persisted = fs::read_to_string(&config_path).expect("read config file");
        assert!(persisted.contains("startup_sync = false"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn config_editor_can_unset_optional_key() {
        let root = temp_dir("config-editor-unset");
        let config_path = root.join("courier-config.toml");
        fs::write(
            &config_path,
            r#"
[b4]
path = "/usr/bin/b4"
"#,
        )
        .expect("write config file");

        let mut runtime = test_runtime();
        runtime.config_path = config_path.clone();
        let mut state = AppState::new(vec![], runtime);

        state.open_config_editor(Some("b4.path"));
        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
        );

        assert!(state.runtime.b4_path.is_none());
        let persisted = fs::read_to_string(&config_path).expect("read config file");
        assert!(!persisted.contains("path = "));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn config_editor_overlay_is_rendered() {
        let runtime = test_runtime();
        let bootstrap = test_bootstrap(&runtime);
        let mut state = AppState::new(vec![], runtime.clone());
        state.open_config_editor(Some("source.mailbox"));

        let mut terminal = Terminal::new(TestBackend::new(140, 40)).expect("create test terminal");
        terminal
            .draw(|frame| draw(frame, &state, &runtime, &bootstrap))
            .expect("draw config editor");
        let rendered = format!("{}", terminal.backend());

        assert!(rendered.contains("Runtime Config"));
        assert!(rendered.contains("source.mailbox"));
        assert!(rendered.contains("Selected Field"));
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
    fn palette_tab_completes_local_command_path() {
        let tree_root = temp_dir("palette-bang-complete");
        fs::write(tree_root.join("echo-local"), "#!/bin/sh\n").expect("write executable");
        let runtime = test_runtime_with_kernel_tree(tree_root.clone());
        let mut state = AppState::new(vec![], runtime);
        state.palette.open = true;
        state.palette.input = "!./ec".to_string();

        let _ = handle_key_event(&mut state, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));

        assert_eq!(state.palette.input, "!./echo-local ");

        let _ = fs::remove_dir_all(tree_root);
    }

    #[test]
    fn local_command_mode_uses_kernel_tree_as_workdir() {
        let tree_root = temp_dir("palette-bang-kernel-tree");
        let runtime = test_runtime_with_kernel_tree(tree_root.clone());
        let state = AppState::new(vec![], runtime);

        let workdir = resolve_palette_local_workdir(&state).expect("resolve local workdir");
        assert_eq!(workdir, tree_root);

        let _ = fs::remove_dir_all(workdir);
    }

    #[test]
    fn local_command_mode_falls_back_to_home_workdir() {
        let state = AppState::new(vec![], test_runtime());
        let resolved = resolve_palette_local_workdir(&state);
        match std::env::var("HOME") {
            Ok(home) => assert_eq!(resolved.expect("resolve home"), PathBuf::from(home)),
            Err(_) => assert!(resolved.is_err()),
        }
    }

    #[test]
    fn palette_bang_executes_local_command() {
        let tree_root = temp_dir("palette-bang-exec");
        let runtime = test_runtime_with_kernel_tree(tree_root.clone());
        let mut state = AppState::new(vec![], runtime);
        state.palette.open = true;
        state.palette.input = "!pwd".to_string();

        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        );

        assert!(state.status.contains("local command ok"));
        assert!(state.status.contains(&tree_root.display().to_string()));
        let local_result = state
            .palette
            .last_local_result
            .as_ref()
            .expect("local result should exist");
        assert_eq!(local_result.command, "pwd");
        assert!(
            local_result
                .output
                .contains(&tree_root.display().to_string())
        );

        let _ = fs::remove_dir_all(tree_root);
    }

    #[test]
    fn command_palette_renders_local_command_result() {
        let tree_root = temp_dir("palette-bang-render");
        let runtime = test_runtime_with_kernel_tree(tree_root.clone());
        let bootstrap = test_bootstrap(&runtime);
        let mut state = AppState::new(vec![], runtime.clone());
        state.palette.open = true;
        state.palette.input = "!pwd".to_string();

        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        );

        let mut terminal = Terminal::new(TestBackend::new(120, 30)).expect("create test terminal");
        terminal
            .draw(|frame| draw(frame, &state, &runtime, &bootstrap))
            .expect("draw frame");
        let rendered = format!("{}", terminal.backend());
        assert!(rendered.contains("Local Result"));
        assert!(rendered.contains("!pwd"));

        let _ = fs::remove_dir_all(tree_root);
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
        assert!(preview.contains("    if true {"));
        assert!(preview.contains("        return;"));
        assert!(!preview.contains('\t'));

        let _ = fs::remove_dir_all(tree_root);
    }

    #[test]
    fn code_edit_mode_enters_only_on_source_file_focus() {
        let tree_root = temp_dir("code-edit-enter");
        let file_path = tree_root.join("demo.rs");
        fs::write(&file_path, "fn demo() {}\n").expect("write demo file");

        let runtime = test_runtime_with_kernel_tree(tree_root.clone());
        let mut state = AppState::new(vec![], runtime);
        let _ = handle_key_event(&mut state, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert!(matches!(state.ui_page, UiPage::CodeBrowser));

        state.code_focus = CodePaneFocus::Tree;
        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE),
        );
        assert!(matches!(state.code_edit_mode, CodeEditMode::Browse));
        assert!(state.status.contains("select a source file"));

        state.code_focus = CodePaneFocus::Source;
        state.kernel_tree_row_index = 0;
        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE),
        );
        assert!(matches!(state.code_edit_mode, CodeEditMode::Browse));
        assert!(state.status.contains("select a source file"));

        let file_index = state
            .kernel_tree_rows
            .iter()
            .position(|row| row.path == file_path)
            .expect("find source file");
        state.kernel_tree_row_index = file_index;
        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE),
        );
        assert!(matches!(state.code_edit_mode, CodeEditMode::VimNormal));
        assert_eq!(state.code_edit_target.as_ref(), Some(&file_path));

        let _ = fs::remove_dir_all(tree_root);
    }

    #[test]
    fn code_edit_insert_save_and_escape_exit_updates_file() {
        let tree_root = temp_dir("code-edit-save-esc");
        let file_path = tree_root.join("demo.rs");
        fs::write(&file_path, "alpha\nbeta\n").expect("write demo file");

        let runtime = test_runtime_with_kernel_tree(tree_root.clone());
        let mut state = AppState::new(vec![], runtime);
        let _ = handle_key_event(&mut state, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        state.code_focus = CodePaneFocus::Source;
        let file_index = state
            .kernel_tree_rows
            .iter()
            .position(|row| row.path == file_path)
            .expect("find source file");
        state.kernel_tree_row_index = file_index;

        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE),
        );
        assert!(matches!(state.code_edit_mode, CodeEditMode::VimNormal));

        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE),
        );
        assert!(matches!(state.code_edit_mode, CodeEditMode::VimInsert));
        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('!'), KeyModifiers::SHIFT),
        );
        let _ = handle_key_event(&mut state, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(matches!(state.code_edit_mode, CodeEditMode::VimNormal));

        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE),
        );
        let saved = fs::read_to_string(&file_path).expect("read saved file");
        assert!(saved.starts_with("!alpha"));

        let _ = handle_key_event(&mut state, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(matches!(state.code_edit_mode, CodeEditMode::Browse));
        let preview = load_source_file_preview(&file_path);
        assert!(preview.contains("!alpha"));

        let _ = fs::remove_dir_all(tree_root);
    }

    #[test]
    fn code_edit_command_mode_handles_dirty_q_w_and_wq() {
        let tree_root = temp_dir("code-edit-command");
        let file_path = tree_root.join("demo.rs");
        fs::write(&file_path, "hello\n").expect("write demo file");

        let runtime = test_runtime_with_kernel_tree(tree_root.clone());
        let mut state = AppState::new(vec![], runtime);
        let _ = handle_key_event(&mut state, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        state.code_focus = CodePaneFocus::Source;
        let file_index = state
            .kernel_tree_rows
            .iter()
            .position(|row| row.path == file_path)
            .expect("find source file");
        state.kernel_tree_row_index = file_index;

        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE),
        );
        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE),
        );
        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
        );
        let _ = handle_key_event(&mut state, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char(':'), KeyModifiers::SHIFT),
        );
        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
        );
        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        );
        assert!(matches!(state.code_edit_mode, CodeEditMode::VimNormal));
        assert!(state.code_edit_dirty);
        assert!(state.status.contains("unsaved changes"));

        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char(':'), KeyModifiers::SHIFT),
        );
        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE),
        );
        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        );
        assert!(!state.code_edit_dirty);
        let saved_once = fs::read_to_string(&file_path).expect("read saved file");
        assert!(saved_once.starts_with("xhello"));

        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE),
        );
        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE),
        );
        let _ = handle_key_event(&mut state, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(state.code_edit_dirty);

        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char(':'), KeyModifiers::SHIFT),
        );
        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE),
        );
        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
        );
        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        );
        assert!(matches!(state.code_edit_mode, CodeEditMode::Browse));
        let saved_twice = fs::read_to_string(&file_path).expect("read saved file");
        assert!(saved_twice.starts_with("xyhello"));

        let _ = fs::remove_dir_all(tree_root);
    }

    #[test]
    fn code_edit_command_mode_rejects_unsupported_command() {
        let tree_root = temp_dir("code-edit-unsupported-command");
        let file_path = tree_root.join("demo.rs");
        fs::write(&file_path, "hello\n").expect("write demo file");

        let runtime = test_runtime_with_kernel_tree(tree_root.clone());
        let mut state = AppState::new(vec![], runtime);
        let _ = handle_key_event(&mut state, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        state.code_focus = CodePaneFocus::Source;
        let file_index = state
            .kernel_tree_rows
            .iter()
            .position(|row| row.path == file_path)
            .expect("find source file");
        state.kernel_tree_row_index = file_index;
        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE),
        );
        assert!(matches!(state.code_edit_mode, CodeEditMode::VimNormal));

        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char(':'), KeyModifiers::SHIFT),
        );
        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE),
        );
        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE),
        );
        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        );

        assert!(matches!(state.code_edit_mode, CodeEditMode::VimNormal));
        assert!(state.status.contains("unsupported command"));

        let _ = fs::remove_dir_all(tree_root);
    }

    #[test]
    fn code_edit_command_mode_supports_force_quit_without_saving() {
        let tree_root = temp_dir("code-edit-force-quit");
        let file_path = tree_root.join("demo.rs");
        fs::write(&file_path, "hello\n").expect("write demo file");

        let runtime = test_runtime_with_kernel_tree(tree_root.clone());
        let mut state = AppState::new(vec![], runtime);
        let _ = handle_key_event(&mut state, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        state.code_focus = CodePaneFocus::Source;
        let file_index = state
            .kernel_tree_rows
            .iter()
            .position(|row| row.path == file_path)
            .expect("find source file");
        state.kernel_tree_row_index = file_index;

        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE),
        );
        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE),
        );
        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
        );
        let _ = handle_key_event(&mut state, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(state.code_edit_dirty);

        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char(':'), KeyModifiers::SHIFT),
        );
        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
        );
        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('!'), KeyModifiers::SHIFT),
        );
        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        );

        assert!(matches!(state.code_edit_mode, CodeEditMode::Browse));
        assert!(state.status.contains("discarded unsaved changes"));
        let disk = fs::read_to_string(&file_path).expect("read file");
        assert_eq!(disk, "hello\n");

        let _ = fs::remove_dir_all(tree_root);
    }

    #[test]
    fn code_browser_external_vim_key_updates_selected_file_preview() {
        let tree_root = temp_dir("external-vim-key");
        let file_path = tree_root.join("demo.rs");
        fs::write(&file_path, "before\n").expect("write demo file");

        let runtime = test_runtime_with_kernel_tree(tree_root.clone());
        let mut state = AppState::new(vec![], runtime);
        let _ = handle_key_event(&mut state, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        state.code_focus = CodePaneFocus::Source;
        let file_index = state
            .kernel_tree_rows
            .iter()
            .position(|row| row.path == file_path)
            .expect("find source file");
        state.kernel_tree_row_index = file_index;
        state.external_editor_runner = external_editor_mock_success;

        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('E'), KeyModifiers::SHIFT),
        );

        assert!(state.status.contains("external vim exited successfully"));
        let preview = load_source_file_preview(&file_path);
        assert!(preview.contains("externally edited"));

        let _ = fs::remove_dir_all(tree_root);
    }

    #[test]
    fn code_edit_external_vim_rejects_dirty_buffer() {
        let tree_root = temp_dir("external-vim-dirty");
        let file_path = tree_root.join("demo.rs");
        fs::write(&file_path, "hello\n").expect("write demo file");

        let runtime = test_runtime_with_kernel_tree(tree_root.clone());
        let mut state = AppState::new(vec![], runtime);
        let _ = handle_key_event(&mut state, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        state.code_focus = CodePaneFocus::Source;
        let file_index = state
            .kernel_tree_rows
            .iter()
            .position(|row| row.path == file_path)
            .expect("find source file");
        state.kernel_tree_row_index = file_index;
        state.external_editor_runner = external_editor_mock_success;

        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE),
        );
        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE),
        );
        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
        );
        let _ = handle_key_event(&mut state, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(state.code_edit_dirty);

        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('E'), KeyModifiers::SHIFT),
        );

        assert!(
            state
                .status
                .contains("unsaved changes, run :w before external vim")
        );
        assert!(matches!(state.code_edit_mode, CodeEditMode::VimNormal));
        assert!(state.code_edit_dirty);
        let disk = fs::read_to_string(&file_path).expect("read file");
        assert_eq!(disk, "hello\n");

        let _ = fs::remove_dir_all(tree_root);
    }

    #[test]
    fn code_edit_command_mode_vim_reloads_buffer_after_external_edit() {
        let tree_root = temp_dir("external-vim-command");
        let file_path = tree_root.join("demo.rs");
        fs::write(&file_path, "hello\n").expect("write demo file");

        let runtime = test_runtime_with_kernel_tree(tree_root.clone());
        let mut state = AppState::new(vec![], runtime);
        let _ = handle_key_event(&mut state, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        state.code_focus = CodePaneFocus::Source;
        let file_index = state
            .kernel_tree_rows
            .iter()
            .position(|row| row.path == file_path)
            .expect("find source file");
        state.kernel_tree_row_index = file_index;
        state.external_editor_runner = external_editor_mock_success;

        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE),
        );
        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char(':'), KeyModifiers::SHIFT),
        );
        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE),
        );
        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE),
        );
        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE),
        );
        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        );

        assert!(matches!(state.code_edit_mode, CodeEditMode::VimNormal));
        assert!(!state.code_edit_dirty);
        assert_eq!(
            state.code_edit_buffer.first().map(String::as_str),
            Some("externally edited")
        );
        assert!(state.status.contains("external vim exited successfully"));

        let _ = fs::remove_dir_all(tree_root);
    }

    #[test]
    fn command_palette_vim_runs_external_editor() {
        let tree_root = temp_dir("external-vim-palette");
        let file_path = tree_root.join("demo.rs");
        fs::write(&file_path, "before\n").expect("write demo file");

        let runtime = test_runtime_with_kernel_tree(tree_root.clone());
        let mut state = AppState::new(vec![], runtime);
        let _ = handle_key_event(&mut state, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        state.code_focus = CodePaneFocus::Source;
        let file_index = state
            .kernel_tree_rows
            .iter()
            .position(|row| row.path == file_path)
            .expect("find source file");
        state.kernel_tree_row_index = file_index;
        state.external_editor_runner = external_editor_mock_success;
        state.palette.open = true;
        state.palette.input = "vim".to_string();

        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        );

        assert!(state.status.contains("external vim exited successfully"));
        let preview = load_source_file_preview(&file_path);
        assert!(preview.contains("externally edited"));

        let _ = fs::remove_dir_all(tree_root);
    }

    #[test]
    fn external_vim_launch_failure_keeps_tui_interactive() {
        let tree_root = temp_dir("external-vim-failure");
        let file_path = tree_root.join("demo.rs");
        fs::write(&file_path, "before\n").expect("write demo file");

        let runtime = test_runtime_with_kernel_tree(tree_root.clone());
        let mut state = AppState::new(vec![], runtime);
        let _ = handle_key_event(&mut state, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        state.code_focus = CodePaneFocus::Source;
        let file_index = state
            .kernel_tree_rows
            .iter()
            .position(|row| row.path == file_path)
            .expect("find source file");
        state.kernel_tree_row_index = file_index;
        state.external_editor_runner = external_editor_mock_failure;

        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('E'), KeyModifiers::SHIFT),
        );
        assert!(state.status.contains("external vim failed"));

        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
        );
        assert!(matches!(state.code_focus, CodePaneFocus::Tree));

        let _ = fs::remove_dir_all(tree_root);
    }

    #[test]
    fn external_vim_marks_terminal_refresh_needed_after_return() {
        let tree_root = temp_dir("external-vim-refresh");
        let file_path = tree_root.join("demo.rs");
        fs::write(&file_path, "before\n").expect("write demo file");

        let runtime = test_runtime_with_kernel_tree(tree_root.clone());
        let mut state = AppState::new(vec![], runtime);
        let _ = handle_key_event(&mut state, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        state.code_focus = CodePaneFocus::Source;
        let file_index = state
            .kernel_tree_rows
            .iter()
            .position(|row| row.path == file_path)
            .expect("find source file");
        state.kernel_tree_row_index = file_index;
        state.external_editor_runner = external_editor_mock_success;

        assert!(!state.needs_terminal_refresh);
        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('E'), KeyModifiers::SHIFT),
        );
        assert!(state.needs_terminal_refresh);
        assert!(state.take_terminal_refresh_needed());
        assert!(!state.needs_terminal_refresh);

        let _ = fs::remove_dir_all(tree_root);
    }

    #[test]
    fn code_edit_draw_sets_terminal_cursor_position() {
        let tree_root = temp_dir("code-edit-cursor");
        let file_path = tree_root.join("demo.rs");
        fs::write(&file_path, "hello\nworld\n").expect("write demo file");

        let runtime = test_runtime_with_kernel_tree(tree_root.clone());
        let bootstrap = test_bootstrap(&runtime);
        let mut state = AppState::new(vec![], runtime.clone());
        let _ = handle_key_event(&mut state, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        state.code_focus = CodePaneFocus::Source;
        let file_index = state
            .kernel_tree_rows
            .iter()
            .position(|row| row.path == file_path)
            .expect("find source file");
        state.kernel_tree_row_index = file_index;
        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE),
        );
        assert!(matches!(state.code_edit_mode, CodeEditMode::VimNormal));

        let mut terminal = Terminal::new(TestBackend::new(120, 30)).expect("create test terminal");
        let mut expected_cursor: Option<(u16, u16)> = None;
        terminal
            .draw(|frame| {
                let areas = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(1),
                        Constraint::Min(10),
                        Constraint::Length(1),
                    ])
                    .split(frame.area());
                let panes = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
                    .split(areas[1]);
                let inner_area = Rect::new(
                    panes[1].x + 1,
                    panes[1].y + 1,
                    panes[1].width.saturating_sub(2),
                    panes[1].height.saturating_sub(2),
                );
                expected_cursor = code_edit_cursor_position(&state, inner_area);
                draw(frame, &state, &runtime, &bootstrap);
            })
            .expect("draw frame");

        let expected = expected_cursor.expect("cursor position should be visible");
        terminal
            .backend_mut()
            .assert_cursor_position(Position::new(expected.0, expected.1));

        let _ = fs::remove_dir_all(tree_root);
    }

    #[test]
    fn code_browser_navigation_keys_unchanged_when_not_editing() {
        let tree_root = temp_dir("code-edit-regression");
        let file_path = tree_root.join("demo.rs");
        fs::write(&file_path, "line1\nline2\n").expect("write demo file");

        let runtime = test_runtime_with_kernel_tree(tree_root.clone());
        let mut state = AppState::new(vec![], runtime);
        let _ = handle_key_event(&mut state, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert!(matches!(state.ui_page, UiPage::CodeBrowser));
        assert!(matches!(state.code_focus, CodePaneFocus::Tree));

        state.code_focus = CodePaneFocus::Source;
        state.code_preview_scroll = 2;
        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE),
        );
        assert_eq!(state.code_preview_scroll, 1);
        assert!(matches!(state.code_edit_mode, CodeEditMode::Browse));

        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE),
        );
        assert_eq!(state.code_preview_scroll, 2);

        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
        );
        assert!(matches!(state.code_focus, CodePaneFocus::Tree));

        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE),
        );
        assert!(matches!(state.code_focus, CodePaneFocus::Source));

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
    fn first_open_with_complete_imap_enables_my_inbox() {
        let state = AppState::new(vec![], test_runtime_with_imap());
        let my_inbox = state
            .subscriptions
            .iter()
            .find(|item| item.mailbox == IMAP_INBOX_MAILBOX)
            .expect("my inbox subscription exists");

        assert!(my_inbox.enabled);
        assert_eq!(my_inbox.label, MY_INBOX_LABEL);
        assert_eq!(state.active_thread_mailbox, IMAP_INBOX_MAILBOX);
    }

    #[test]
    fn legacy_ui_state_with_complete_imap_enables_my_inbox_once() {
        let state = AppState::new_with_ui_state(
            vec![],
            test_runtime_with_imap(),
            Some(UiState {
                enabled_mailboxes: vec!["io-uring".to_string()],
                enabled_group_expanded: true,
                disabled_group_expanded: true,
                imap_defaults_initialized: false,
                active_mailbox: Some("io-uring".to_string()),
            }),
        );

        let my_inbox = state
            .subscriptions
            .iter()
            .find(|item| item.mailbox == IMAP_INBOX_MAILBOX)
            .expect("my inbox subscription exists");

        assert!(my_inbox.enabled);
        assert!(state.imap_defaults_initialized);
    }

    #[test]
    fn initialized_ui_state_keeps_my_inbox_disabled_when_user_opted_out() {
        let state = AppState::new_with_ui_state(
            vec![],
            test_runtime_with_imap(),
            Some(UiState {
                enabled_mailboxes: vec!["io-uring".to_string()],
                enabled_group_expanded: true,
                disabled_group_expanded: true,
                imap_defaults_initialized: true,
                active_mailbox: Some("io-uring".to_string()),
            }),
        );

        let my_inbox = state
            .subscriptions
            .iter()
            .find(|item| item.mailbox == IMAP_INBOX_MAILBOX)
            .expect("my inbox subscription exists");

        assert!(!my_inbox.enabled);
        assert!(state.imap_defaults_initialized);
    }

    #[test]
    fn catch_sync_panic_converts_panics_into_errors() {
        let error = catch_sync_panic("INBOX", || -> crate::infra::error::Result<()> {
            panic!("boom");
        })
        .expect_err("panic should become courier error");

        assert!(error.to_string().contains("sync panicked for INBOX: boom"));
    }

    #[test]
    fn empty_active_inbox_recovers_to_cached_enabled_mailbox() {
        let root = temp_dir("imap-fallback-cache");
        let runtime = test_runtime_with_imap_in(root.clone());
        seed_mailbox_thread(
            &runtime.database_path,
            "kvm",
            1,
            "kvm@example.com",
            "kvm thread",
        );

        let mut state = AppState::new_with_ui_state(
            vec![],
            runtime,
            Some(UiState {
                enabled_mailboxes: vec![IMAP_INBOX_MAILBOX.to_string(), "kvm".to_string()],
                enabled_group_expanded: true,
                disabled_group_expanded: true,
                imap_defaults_initialized: true,
                active_mailbox: Some(IMAP_INBOX_MAILBOX.to_string()),
            }),
        );

        assert!(state.recover_from_empty_active_mailbox("inbox unavailable"));
        assert_eq!(state.active_thread_mailbox, "kvm");
        assert_eq!(state.threads.len(), 1);
        assert!(state.status.contains("showing threads for kvm"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn startup_sync_failure_for_empty_inbox_falls_back_to_cached_mailbox() {
        let root = temp_dir("imap-fallback-startup");
        let runtime = test_runtime_with_imap_in(root.clone());
        seed_mailbox_thread(
            &runtime.database_path,
            "io-uring",
            1,
            "io-uring@example.com",
            "io_uring thread",
        );

        let mut state = AppState::new_with_ui_state(
            vec![],
            runtime,
            Some(UiState {
                enabled_mailboxes: vec![IMAP_INBOX_MAILBOX.to_string(), "io-uring".to_string()],
                enabled_group_expanded: true,
                disabled_group_expanded: true,
                imap_defaults_initialized: true,
                active_mailbox: Some(IMAP_INBOX_MAILBOX.to_string()),
            }),
        );

        state.apply_startup_sync_event(StartupSyncEvent::MailboxFailed {
            mailbox: IMAP_INBOX_MAILBOX.to_string(),
            error: "imap unavailable".to_string(),
        });

        assert_eq!(state.active_thread_mailbox, "io-uring");
        assert_eq!(state.threads.len(), 1);
        assert!(state.status.contains("showing threads for io-uring"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn enter_on_mailbox_pending_startup_sync_stays_non_blocking() {
        let root = temp_dir("imap-pending-enter");
        let runtime = test_runtime_with_imap_in(root.clone());
        fs::create_dir_all(runtime.database_path.parent().expect("db parent"))
            .expect("create db parent");
        db::initialize(&runtime.database_path).expect("initialize db");

        let mut state = AppState::new_with_ui_state(
            vec![],
            runtime,
            Some(UiState {
                enabled_mailboxes: vec![IMAP_INBOX_MAILBOX.to_string()],
                enabled_group_expanded: true,
                disabled_group_expanded: true,
                imap_defaults_initialized: true,
                active_mailbox: Some(IMAP_INBOX_MAILBOX.to_string()),
            }),
        );
        state.focus = Pane::Subscriptions;
        state.startup_sync = Some(startup_sync_state(&[(
            IMAP_INBOX_MAILBOX,
            StartupSyncMailboxStatus::InFlight,
        )]));

        state.open_threads_for_selected_subscription();

        assert_eq!(state.active_thread_mailbox, IMAP_INBOX_MAILBOX);
        assert!(state.threads.is_empty());
        assert!(state.status.contains("syncing in background"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn background_success_does_not_steal_focus_from_pending_inbox() {
        let root = temp_dir("imap-pending-focus");
        let runtime = test_runtime_with_imap_in(root.clone());
        seed_mailbox_thread(
            &runtime.database_path,
            "kvm",
            1,
            "kvm@example.com",
            "kvm thread",
        );

        let mut state = AppState::new_with_ui_state(
            vec![],
            runtime,
            Some(UiState {
                enabled_mailboxes: vec![IMAP_INBOX_MAILBOX.to_string(), "kvm".to_string()],
                enabled_group_expanded: true,
                disabled_group_expanded: true,
                imap_defaults_initialized: true,
                active_mailbox: Some(IMAP_INBOX_MAILBOX.to_string()),
            }),
        );
        state.startup_sync = Some(startup_sync_state(&[
            (IMAP_INBOX_MAILBOX, StartupSyncMailboxStatus::InFlight),
            ("kvm", StartupSyncMailboxStatus::Pending),
        ]));

        state.apply_startup_sync_event(StartupSyncEvent::MailboxFinished {
            mailbox: "kvm".to_string(),
            fetched: 1,
            inserted: 1,
            updated: 0,
        });

        assert_eq!(state.active_thread_mailbox, IMAP_INBOX_MAILBOX);
        assert!(state.threads.is_empty());

        let _ = fs::remove_dir_all(root);
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
    fn a_d_and_u_require_patch_series_or_apply_snapshot_on_thread_focus() {
        let mut state = AppState::new(
            vec![sample_thread("normal mail", "plain@example.com", 0)],
            test_runtime(),
        );
        state.focus = Pane::Threads;

        let action_apply = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE),
        );
        assert!(matches!(action_apply, LoopAction::Continue));
        assert!(state.status.contains("not a patch series"));

        let action_download = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
        );
        assert!(matches!(action_download, LoopAction::Continue));
        assert!(state.status.contains("not a patch series"));

        let action_undo = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('u'), KeyModifiers::NONE),
        );
        assert!(matches!(action_undo, LoopAction::Continue));
        assert!(state.status.contains("no apply action to undo"));
    }

    #[test]
    fn inline_ui_text_collapses_multiline_errors() {
        let sanitized = sanitize_inline_ui_text(
            "sync failed:\nCould not automatically determine provider\r\n\tline2",
        );

        assert_eq!(
            sanitized,
            "sync failed: Could not automatically determine provider line2"
        );
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
    fn preview_shows_from_sent_to_cc_headers() {
        let raw = b"From: Chen Miao <chenmiao.ku@gmail.com>\r\nDate: Monday, March 2, 2026 5:29 PM\r\nTo: Daniel Baluta <daniel.baluta@nxp.com>; Simona Toaca <simona.toaca@nxp.com>\r\nCc: Team One <team1@example.com>\r\nSubject: [PATCH] demo\r\n\r\nmail body line\n";
        let preview = extract_mail_preview(raw, "(no subject)", "<unknown sender>", None);

        assert!(preview.contains("From: Chen Miao <chenmiao.ku@gmail.com>"));
        assert!(preview.contains("Sent: Monday, March 2, 2026 5:29 PM"));
        assert!(preview.contains(
            "To: Daniel Baluta <daniel.baluta@nxp.com>; Simona Toaca <simona.toaca@nxp.com>"
        ));
        assert!(preview.contains("Cc: Team One <team1@example.com>"));
        assert!(preview.contains("Subject: [PATCH] demo"));
        assert!(preview.contains("mail body line"));
    }

    #[test]
    fn preview_truncates_to_and_cc_recipient_lists() {
        let raw = b"From: sender@example.com\r\nDate: Tue, 3 Mar 2026 12:00:00 +0000\r\nTo: A <a@example.com>, B <b@example.com>, C <c@example.com>\r\nCc: X <x@example.com>; Y <y@example.com>; Z <z@example.com>\r\nSubject: test\r\n\r\nbody\n";
        let preview = extract_mail_preview(raw, "(no subject)", "<unknown sender>", None);

        assert!(preview.contains("To: A <a@example.com>; B <b@example.com>; ..."));
        assert!(preview.contains("Cc: X <x@example.com>; Y <y@example.com>; ..."));
        assert!(!preview.contains("C <c@example.com>"));
        assert!(!preview.contains("Z <z@example.com>"));
    }

    #[test]
    fn preview_warns_for_multipart_mail() {
        let raw = b"Content-Type: multipart/alternative; boundary=\"abc\"\r\n\r\n--abc\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nplain body line\r\n--abc--\r\n";
        let warning = preview_warning_message(raw).expect("warning expected");

        assert!(warning.contains("NON-PLAIN-TEXT MAIL"));
        assert!(warning.contains("Parse artifacts/errors are normal"));
        assert!(warning.contains("Content-Type: multipart/alternative; boundary=\"abc\""));
    }

    #[test]
    fn preview_warns_for_encoded_html_mail() {
        let raw = b"Content-Type: text/html; charset=utf-8\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\n<html><body>hello</body></html>\r\n";
        let warning = preview_warning_message(raw).expect("warning expected");

        assert!(warning.contains("NON-PLAIN-TEXT MAIL"));
        assert!(warning.contains("Content-Type: text/html; charset=utf-8"));
        assert!(warning.contains("Transfer-Encoding: quoted-printable"));
    }

    #[test]
    fn multiline_sync_error_does_not_break_footer_or_palette_render() {
        let runtime = test_runtime();
        let bootstrap = test_bootstrap(&runtime);
        let mut state = AppState::new(vec![], runtime.clone());
        state.status =
            "sync failed: E1007:\nCould not automatically determine provider".to_string();
        state.palette.open = true;

        let mut terminal = Terminal::new(TestBackend::new(140, 35)).expect("create test terminal");
        terminal
            .draw(|frame| draw(frame, &state, &runtime, &bootstrap))
            .expect("draw multiline status");
        let rendered = format!("{}", terminal.backend());

        assert!(
            rendered
                .contains("status: sync failed: E1007: Could not automatically determine provider")
        );
        assert!(rendered.contains("Command Palette"));
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
    fn code_source_preview_redraw_clears_stale_characters_after_file_switch() {
        let root = temp_dir("code-preview-clear");
        let file_a = root.join("a-long.rs");
        let file_b = root.join("b-short.rs");
        fs::write(
            &file_a,
            "fn demo() {\n    let _x = \"STALE_SOURCE_TOKEN_987654\";\n}\n",
        )
        .expect("write file a");
        fs::write(&file_b, "fn demo() {}\n").expect("write file b");

        let runtime = test_runtime_with_kernel_tree(root.clone());
        let bootstrap = test_bootstrap(&runtime);
        let mut state = AppState::new(vec![], runtime.clone());
        state.ui_page = UiPage::CodeBrowser;
        state.code_focus = CodePaneFocus::Tree;

        let index_a = state
            .kernel_tree_rows
            .iter()
            .position(|row| row.path == file_a)
            .expect("find file a");
        state.kernel_tree_row_index = index_a;

        let mut terminal = Terminal::new(TestBackend::new(120, 30)).expect("create test terminal");
        terminal
            .draw(|frame| draw(frame, &state, &runtime, &bootstrap))
            .expect("draw first frame");
        let first_frame = format!("{}", terminal.backend());
        assert!(first_frame.contains("STALE_SOURCE_TOKEN_987654"));

        let _ = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE),
        );
        terminal
            .draw(|frame| draw(frame, &state, &runtime, &bootstrap))
            .expect("draw second frame");
        let second_frame = format!("{}", terminal.backend());
        assert!(!second_frame.contains("STALE_SOURCE_TOKEN_987654"));
        assert!(second_frame.contains("fn demo() {}"));

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
