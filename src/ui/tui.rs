use std::io::{self, Stdout};
use std::time::{Duration, Instant};

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

use crate::infra::bootstrap::BootstrapState;
use crate::infra::config::RuntimeConfig;
use crate::infra::error::{CourierError, ErrorCode, Result};

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
];

#[derive(Debug, Default)]
struct CommandPaletteState {
    open: bool,
    input: String,
}

#[derive(Debug)]
struct AppState {
    focus: Pane,
    subscriptions: Vec<&'static str>,
    threads: Vec<&'static str>,
    subscription_index: usize,
    thread_index: usize,
    preview_scroll: u16,
    started_at: Instant,
    status: String,
    palette: CommandPaletteState,
}

impl AppState {
    fn new() -> Self {
        Self {
            focus: Pane::Subscriptions,
            subscriptions: vec![
                "[x] linux-kernel",
                "[x] linux-mm",
                "[ ] linux-fsdevel",
                "[ ] netdev",
                "[ ] linux-arm-kernel",
            ],
            threads: vec![
                "[PATCH v3 0/7] folio cleanups",
                "[PATCH v1 0/5] io_uring fixes",
                "[PATCH v2 0/4] net: xdp updates",
                "[RFC 0/3] mm: compact strategy",
                "[PATCH v4 0/2] tracing docs",
            ],
            subscription_index: 0,
            thread_index: 0,
            preview_scroll: 0,
            started_at: Instant::now(),
            status: "ready".to_string(),
            palette: CommandPaletteState::default(),
        }
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
                if self.subscription_index > 0 {
                    self.subscription_index -= 1;
                }
            }
            Pane::Threads => {
                if self.thread_index > 0 {
                    self.thread_index -= 1;
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
                if self.subscription_index + 1 < self.subscriptions.len() {
                    self.subscription_index += 1;
                }
            }
            Pane::Threads => {
                if self.thread_index + 1 < self.threads.len() {
                    self.thread_index += 1;
                }
            }
            Pane::Preview => {
                self.preview_scroll = self.preview_scroll.saturating_add(1);
            }
        }
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
    let mut terminal = setup_terminal()?;
    let guard = TerminalGuard;
    let mut state = AppState::new();

    let result = tui_loop(&mut terminal, &mut state, config, bootstrap);

    drop(guard);
    result
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

    if is_palette_open_shortcut(key) {
        state.toggle_palette();
        return LoopAction::Continue;
    }

    match key.code {
        KeyCode::Char('j') => state.move_focus_previous(),
        KeyCode::Char('l') => state.move_focus_next(),
        KeyCode::Char('i') => state.move_up(),
        KeyCode::Char('k') => state.move_down(),
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
                    state.status = "commands: quit, exit, help".to_string();
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
        "mailbox: inbox | db schema: {} | db: {} | uptime: {}s",
        bootstrap.db.schema_version,
        bootstrap.db.path.display(),
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

    let shortcuts_text = ": palette | j/l focus | i/k move";
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
}

fn draw_subscriptions(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let items: Vec<ListItem> = state
        .subscriptions
        .iter()
        .map(|entry| ListItem::new(*entry))
        .collect();

    let mut list_state = ListState::default();
    list_state.select(Some(state.subscription_index));

    let list = List::new(items)
        .block(panel_block(Pane::Subscriptions, state.focus))
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );

    frame.render_stateful_widget(list, area, &mut list_state);
}

fn draw_threads(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let items: Vec<ListItem> = state
        .threads
        .iter()
        .map(|entry| ListItem::new(*entry))
        .collect();

    let mut list_state = ListState::default();
    list_state.select(Some(state.thread_index));

    let list = List::new(items)
        .block(panel_block(Pane::Threads, state.focus))
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );

    frame.render_stateful_widget(list, area, &mut list_state);
}

fn draw_preview(frame: &mut Frame<'_>, area: Rect, state: &AppState, config: &RuntimeConfig) {
    let preview = format!(
        "Courier M1 TUI skeleton\n\nFocused pane: {}\nConfig: {}\nDatabase: {}\n\nPatch preview placeholder\n---\n1. Read thread mail\n2. Validate series\n3. Apply with b4 am\n",
        state.focus.title(),
        config.config_path.display(),
        config.database_path.display(),
    );

    let paragraph = Paragraph::new(preview)
        .block(panel_block(Pane::Preview, state.focus))
        .scroll((state.preview_scroll, 0))
        .wrap(Wrap { trim: true });

    frame.render_widget(paragraph, area);
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
    use std::time::Instant;

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::{
        AppState, CommandPaletteState, LoopAction, Pane, handle_key_event,
        is_palette_open_shortcut, is_palette_toggle, matching_commands,
    };

    #[test]
    fn empty_query_returns_all_palette_commands() {
        let all = matching_commands("");
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].name, "exit");
        assert_eq!(all[1].name, "help");
        assert_eq!(all[2].name, "quit");
    }

    #[test]
    fn prefix_matches_rank_before_fuzzy_matches() {
        let commands = matching_commands("ex");
        assert_eq!(commands[0].name, "exit");
    }

    #[test]
    fn quit_key_is_ignored_outside_command_palette() {
        let mut state = AppState {
            focus: Pane::Subscriptions,
            subscriptions: vec![],
            threads: vec![],
            subscription_index: 0,
            thread_index: 0,
            preview_scroll: 0,
            started_at: Instant::now(),
            status: String::new(),
            palette: CommandPaletteState::default(),
        };

        let action = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
        );
        assert!(matches!(action, LoopAction::Continue));
    }

    #[test]
    fn command_palette_quit_exits_application() {
        let mut state = AppState {
            focus: Pane::Subscriptions,
            subscriptions: vec![],
            threads: vec![],
            subscription_index: 0,
            thread_index: 0,
            preview_scroll: 0,
            started_at: Instant::now(),
            status: String::new(),
            palette: CommandPaletteState {
                open: true,
                input: "quit".to_string(),
            },
        };

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
        let mut state = AppState {
            focus: Pane::Subscriptions,
            subscriptions: vec![],
            threads: vec![],
            subscription_index: 0,
            thread_index: 0,
            preview_scroll: 0,
            started_at: Instant::now(),
            status: String::new(),
            palette: CommandPaletteState::default(),
        };

        let key = KeyEvent::new(KeyCode::Char(':'), KeyModifiers::SHIFT);
        assert!(is_palette_open_shortcut(key));

        let action = handle_key_event(&mut state, key);
        assert!(matches!(action, LoopAction::Continue));
        assert!(state.palette.open);
    }

    #[test]
    fn jl_focus_and_ik_move_selection() {
        let mut state = AppState {
            focus: Pane::Subscriptions,
            subscriptions: vec!["a", "b", "c"],
            threads: vec!["t0", "t1"],
            subscription_index: 1,
            thread_index: 0,
            preview_scroll: 0,
            started_at: Instant::now(),
            status: String::new(),
            palette: CommandPaletteState::default(),
        };

        let action_l = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE),
        );
        assert!(matches!(action_l, LoopAction::Continue));
        assert!(matches!(state.focus, Pane::Threads));

        let action_j = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
        );
        assert!(matches!(action_j, LoopAction::Continue));
        assert!(matches!(state.focus, Pane::Subscriptions));

        let action_i = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE),
        );
        assert!(matches!(action_i, LoopAction::Continue));
        assert_eq!(state.subscription_index, 0);

        let action_k = handle_key_event(
            &mut state,
            KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE),
        );
        assert!(matches!(action_k, LoopAction::Continue));
        assert_eq!(state.subscription_index, 1);
    }
}
