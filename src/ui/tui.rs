use std::collections::HashSet;
use std::fs;
use std::io::{self, Stdout};
use std::path::PathBuf;
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
];

const PALETTE_SYNC_RECONNECT_ATTEMPTS: u8 = 3;
const PREVIEW_TAB_SPACES: &str = "    ";

#[derive(Debug, Default)]
struct CommandPaletteState {
    open: bool,
    input: String,
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

#[derive(Debug)]
struct AppState {
    runtime: RuntimeConfig,
    ui_state_path: PathBuf,
    active_thread_mailbox: String,
    focus: Pane,
    subscriptions: Vec<SubscriptionItem>,
    enabled_group_expanded: bool,
    disabled_group_expanded: bool,
    threads: Vec<ThreadRow>,
    filtered_thread_indices: Vec<usize>,
    subscription_index: usize,
    subscription_row_index: usize,
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
        let mut state = Self {
            active_thread_mailbox,
            runtime,
            ui_state_path,
            focus: Pane::Subscriptions,
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

    fn move_focus_next(&mut self) {
        self.focus = self.focus.next();
    }

    fn move_focus_previous(&mut self) {
        self.focus = self.focus.previous();
    }

    fn move_up(&mut self) {
        match self.focus {
            Pane::Subscriptions => {
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
            Pane::Threads => {
                if self.thread_index > 0 {
                    self.thread_index -= 1;
                    self.preview_scroll = 0;
                }
            }
            Pane::Preview => {
                self.preview_scroll = self.preview_scroll.saturating_sub(1);
            }
        }
    }

    fn move_down(&mut self) {
        match self.focus {
            Pane::Subscriptions => {
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
            Pane::Threads => {
                if self.thread_index + 1 < self.filtered_thread_indices.len() {
                    self.thread_index += 1;
                    self.preview_scroll = 0;
                }
            }
            Pane::Preview => {
                self.preview_scroll = self.preview_scroll.saturating_add(1);
            }
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
            self.status = "command palette opened".to_string();
        } else {
            self.palette.input.clear();
            self.status = "command palette closed".to_string();
        }
    }

    fn close_palette(&mut self) {
        self.palette.open = false;
        self.palette.input.clear();
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
            state.open_search();
        }
        KeyCode::Char(character)
            if matches!(state.focus, Pane::Subscriptions)
                && character.eq_ignore_ascii_case(&'y') =>
        {
            state.set_current_subscription_enabled(true);
        }
        KeyCode::Char(character)
            if matches!(state.focus, Pane::Subscriptions)
                && character.eq_ignore_ascii_case(&'n') =>
        {
            state.set_current_subscription_enabled(false);
        }
        KeyCode::Char('j') => state.move_focus_previous(),
        KeyCode::Char('l') => state.move_focus_next(),
        KeyCode::Char('i') => state.move_up(),
        KeyCode::Char('k') => state.move_down(),
        KeyCode::Enter => match state.focus {
            Pane::Subscriptions => state.handle_subscription_enter(),
            Pane::Threads => {
                if let Some(thread) = state.selected_thread() {
                    state.status = format!("selected {}", thread.message_id);
                }
            }
            Pane::Preview => {}
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

            if command.is_empty() {
                state.status = "empty command".to_string();
                return LoopAction::Continue;
            }

            match command.as_str() {
                "quit" | "exit" => return LoopAction::Exit,
                "help" => {
                    state.status = "commands: quit, exit, help, sync [mailbox]".to_string();
                }
                value if value.split_whitespace().next() == Some("sync") => {
                    run_palette_sync(state, value);
                }
                _ => {
                    state.status = format!("unknown command: {command}");
                }
            }
        }
        KeyCode::Backspace => {
            state.palette.input.pop();
        }
        KeyCode::Char(character)
            if !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER) =>
        {
            state.palette.input.push(character);
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
    let header = format!(
        "mailbox: {} | db schema: {} | db: {} | threads: {} | uptime: {}s",
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
        .split(areas[1]);

    draw_subscriptions(frame, panes[0], state);
    draw_threads(frame, panes[1], state);
    draw_preview(frame, panes[2], state, config);

    let shortcuts_text =
        "/ search | : palette | j/l focus | i/k move | y/n enable | Enter open/toggle";
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

fn subscription_line(item: &SubscriptionItem) -> String {
    let marker = if item.enabled { "y" } else { "n" };
    format!(
        "[{marker}] {} - {} ({})",
        item.mailbox,
        item.description,
        lore_archive_url(&item.mailbox)
    )
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
    let items: Vec<ListItem> = state
        .filtered_thread_indices
        .iter()
        .filter_map(|index| state.threads.get(*index))
        .map(thread_line)
        .map(ListItem::new)
        .collect();

    let selected = if items.is_empty() {
        None
    } else {
        Some(state.thread_index)
    };

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

    let hints = Paragraph::new("Enter: execute  Esc: close  Ctrl+` or F1: toggle  : opens");
    frame.render_widget(hints, sections[1]);

    let candidates = matching_commands(&state.palette.input);
    let candidate_title = if candidates.is_empty() {
        "Candidates: <none>"
    } else {
        "Candidates"
    };
    let candidate_header = Paragraph::new(candidate_title);
    frame.render_widget(candidate_header, sections[2]);

    let items: Vec<ListItem> = candidates
        .iter()
        .take(4)
        .map(|command| ListItem::new(format!("{} - {}", command.name, command.description)))
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Gray)),
    );
    frame.render_widget(list, sections[3]);
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
    let title = if is_focused {
        format!("{} *", panel.title())
    } else {
        panel.title().to_string()
    };

    let border_style = if is_focused {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Gray)
    };

    Block::default()
        .title(title)
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
        AppState, LoopAction, Pane, draw, extract_mail_body_preview, handle_key_event,
        is_palette_open_shortcut, is_palette_toggle, matching_commands,
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
    fn empty_query_returns_all_palette_commands() {
        let all = matching_commands("");
        assert_eq!(all.len(), 4);
        assert_eq!(all[0].name, "exit");
        assert_eq!(all[1].name, "help");
        assert_eq!(all[2].name, "quit");
        assert_eq!(all[3].name, "sync");
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
}
