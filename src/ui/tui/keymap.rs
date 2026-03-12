//! Main-page keymap resolution, matching, and editor UI.
//!
//! CRIEW keeps top-level navigation configurable while preserving the
//! hard-coded rescue keys that operators rely on to recover from a bad custom
//! layout. This module resolves the active scheme from runtime config, matches
//! multi-key chords, and exposes the `:keymap` editor modal.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};

use crate::infra::config::{RuntimeConfig, UiCustomKeymapConfig, UiKeymap, UiKeymapBase};

use super::config::{apply_runtime_update, remove_config_key_from_file, update_config_key_in_file};
use super::input::LoopAction;
use super::input::pending_main_page_move_count;
use super::render::{centered_rect, truncate_with_ellipsis};
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MainPageAction {
    FocusPrevious,
    FocusNext,
    MoveUp,
    MoveDown,
    JumpTop,
    JumpBottom,
    QuickQuit,
}

impl MainPageAction {
    const ALL: [Self; 7] = [
        Self::FocusPrevious,
        Self::FocusNext,
        Self::MoveUp,
        Self::MoveDown,
        Self::JumpTop,
        Self::JumpBottom,
        Self::QuickQuit,
    ];

    fn key(self) -> &'static str {
        match self {
            Self::FocusPrevious => "focus_prev",
            Self::FocusNext => "focus_next",
            Self::MoveUp => "move_up",
            Self::MoveDown => "move_down",
            Self::JumpTop => "jump_top",
            Self::JumpBottom => "jump_bottom",
            Self::QuickQuit => "quick_quit",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::FocusPrevious => "Focus Previous",
            Self::FocusNext => "Focus Next",
            Self::MoveUp => "Move Up",
            Self::MoveDown => "Move Down",
            Self::JumpTop => "Jump Top",
            Self::JumpBottom => "Jump Bottom",
            Self::QuickQuit => "Quick Quit",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::FocusPrevious => "Move focus to the previous top-level pane.",
            Self::FocusNext => "Move focus to the next top-level pane.",
            Self::MoveUp => "Move up inside the currently focused pane.",
            Self::MoveDown => "Move down inside the currently focused pane.",
            Self::JumpTop => "Jump to the top of the active pane or preview.",
            Self::JumpBottom => "Jump to the bottom of the active pane or preview.",
            Self::QuickQuit => "Exit CRIEW without opening the command palette.",
        }
    }

    fn max_tokens(self) -> usize {
        match self {
            Self::FocusPrevious | Self::FocusNext | Self::MoveUp | Self::MoveDown => 1,
            Self::JumpTop | Self::JumpBottom | Self::QuickQuit => 2,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct KeySequence {
    tokens: Vec<char>,
}

impl KeySequence {
    fn from_chars(tokens: Vec<char>) -> Self {
        Self { tokens }
    }

    fn from_config_tokens(tokens: &[String]) -> Self {
        Self {
            tokens: tokens
                .iter()
                .filter_map(|token| token.chars().next())
                .collect(),
        }
    }

    fn display(&self) -> String {
        self.tokens.iter().collect()
    }

    fn as_chars(&self) -> &[char] {
        &self.tokens
    }

    fn starts_with(&self, input: &[char]) -> bool {
        self.tokens.starts_with(input)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ResolvedMainPageKeymap {
    focus_previous: KeySequence,
    focus_next: KeySequence,
    move_up: KeySequence,
    move_down: KeySequence,
    move_shortcut_order: MoveShortcutOrder,
    jump_top: Option<KeySequence>,
    jump_bottom: Option<KeySequence>,
    quick_quit: Option<KeySequence>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MoveShortcutOrder {
    UpThenDown,
    DownThenUp,
}

impl ResolvedMainPageKeymap {
    fn binding(&self, action: MainPageAction) -> Option<&KeySequence> {
        match action {
            MainPageAction::FocusPrevious => Some(&self.focus_previous),
            MainPageAction::FocusNext => Some(&self.focus_next),
            MainPageAction::MoveUp => Some(&self.move_up),
            MainPageAction::MoveDown => Some(&self.move_down),
            MainPageAction::JumpTop => self.jump_top.as_ref(),
            MainPageAction::JumpBottom => self.jump_bottom.as_ref(),
            MainPageAction::QuickQuit => self.quick_quit.as_ref(),
        }
    }

    fn action_for_sequence(&self, input: &[char]) -> Option<MainPageAction> {
        MainPageAction::ALL.into_iter().find(|action| {
            self.binding(*action)
                .is_some_and(|binding| binding.as_chars() == input)
        })
    }

    fn has_pending_prefix(&self, input: &[char]) -> bool {
        MainPageAction::ALL.into_iter().any(|action| {
            self.binding(action)
                .is_some_and(|binding| binding.as_chars() != input && binding.starts_with(input))
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum KeymapEditorMode {
    #[default]
    Browse,
    Capture,
}

#[derive(Debug, Default)]
pub(super) struct KeymapEditorState {
    pub(super) open: bool,
    pub(super) selected_field: usize,
    pub(super) mode: KeymapEditorMode,
    pub(super) capture_tokens: Vec<char>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeymapEditorField {
    ActiveScheme,
    CustomBase,
    Action(MainPageAction),
}

impl KeymapEditorField {
    fn label(self) -> &'static str {
        match self {
            Self::ActiveScheme => "Active Scheme",
            Self::CustomBase => "Custom Base",
            Self::Action(action) => action.label(),
        }
    }
}

const KEYMAP_EDITOR_FIELDS: &[KeymapEditorField] = &[
    KeymapEditorField::ActiveScheme,
    KeymapEditorField::CustomBase,
    KeymapEditorField::Action(MainPageAction::FocusPrevious),
    KeymapEditorField::Action(MainPageAction::FocusNext),
    KeymapEditorField::Action(MainPageAction::MoveUp),
    KeymapEditorField::Action(MainPageAction::MoveDown),
    KeymapEditorField::Action(MainPageAction::JumpTop),
    KeymapEditorField::Action(MainPageAction::JumpBottom),
    KeymapEditorField::Action(MainPageAction::QuickQuit),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PendingMainPageSequenceState {
    tokens: Vec<char>,
    ui_page: UiPage,
    focus: Pane,
    code_focus: CodePaneFocus,
}

impl AppState {
    pub(super) fn open_keymap_editor(&mut self) {
        self.palette.open = false;
        self.palette.input.clear();
        self.palette.clear_completion();
        self.palette.clear_local_result();
        self.keymap_editor.open = true;
        self.keymap_editor.mode = KeymapEditorMode::Browse;
        self.keymap_editor.capture_tokens.clear();
        self.status = "keymap editor opened".to_string();
    }

    pub(super) fn close_keymap_editor(&mut self) {
        self.keymap_editor.open = false;
        self.keymap_editor.mode = KeymapEditorMode::Browse;
        self.keymap_editor.capture_tokens.clear();
        self.status = "keymap editor closed".to_string();
    }

    fn selected_keymap_editor_field(&self) -> KeymapEditorField {
        let index = self
            .keymap_editor
            .selected_field
            .min(KEYMAP_EDITOR_FIELDS.len().saturating_sub(1));
        KEYMAP_EDITOR_FIELDS[index]
    }

    fn move_keymap_editor_up(&mut self) {
        if self.keymap_editor.selected_field > 0 {
            self.keymap_editor.selected_field -= 1;
        }
    }

    fn move_keymap_editor_down(&mut self) {
        if self.keymap_editor.selected_field + 1 < KEYMAP_EDITOR_FIELDS.len() {
            self.keymap_editor.selected_field += 1;
        }
    }

    fn pending_main_page_sequence_state(&self, tokens: Vec<char>) -> PendingMainPageSequenceState {
        PendingMainPageSequenceState {
            tokens,
            ui_page: self.ui_page,
            focus: self.focus,
            code_focus: self.code_focus,
        }
    }

    fn take_pending_main_page_sequence(&mut self) -> Option<Vec<char>> {
        let pending = self.pending_main_page_sequence.take()?;
        let same_scope = pending.ui_page == self.ui_page
            && pending.focus == self.focus
            && pending.code_focus == self.code_focus;
        same_scope.then_some(pending.tokens)
    }

    pub(super) fn rebuild_main_page_keymap(&mut self) {
        self.main_page_keymap = resolve_active_main_page_keymap(&self.runtime);
        self.pending_main_page_sequence = None;
    }

    fn start_keymap_binding_capture(&mut self) {
        let KeymapEditorField::Action(action) = self.selected_keymap_editor_field() else {
            self.status = "select a key binding row to record".to_string();
            return;
        };

        self.keymap_editor.mode = KeymapEditorMode::Capture;
        self.keymap_editor.capture_tokens.clear();
        self.status = format!("recording {}", action.key());
    }

    fn cancel_keymap_binding_capture(&mut self) {
        self.keymap_editor.mode = KeymapEditorMode::Browse;
        self.keymap_editor.capture_tokens.clear();
        self.status = "key binding capture cancelled".to_string();
    }

    fn capture_keymap_binding_token(&mut self, action: MainPageAction, key: KeyEvent) {
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER)
        {
            self.status = "modifiers are not supported in main-page keymap bindings".to_string();
            return;
        }

        let KeyCode::Char(character) = key.code else {
            self.status = "only printable character keys are supported".to_string();
            return;
        };
        if !is_bindable_keymap_character(character) {
            self.status = format!("unsupported binding key: {character}");
            return;
        }
        if is_reserved_keymap_character(character) {
            self.status = format!("reserved binding key: {character}");
            return;
        }
        if self.keymap_editor.capture_tokens.len() >= action.max_tokens() {
            self.status = format!(
                "{} accepts at most {} key(s)",
                action.key(),
                action.max_tokens()
            );
            return;
        }

        self.keymap_editor.capture_tokens.push(character);
        self.status = format!(
            "recording {} = {}",
            action.key(),
            self.keymap_editor.capture_tokens.iter().collect::<String>()
        );
    }

    fn save_keymap_binding_capture(&mut self) {
        let KeymapEditorField::Action(action) = self.selected_keymap_editor_field() else {
            self.cancel_keymap_binding_capture();
            return;
        };
        if self.keymap_editor.capture_tokens.is_empty() {
            self.status = format!(
                "empty binding for {}; press x to reset instead",
                action.key()
            );
            return;
        }

        let literal = format_toml_key_sequence_literal(&self.keymap_editor.capture_tokens);
        match update_config_key_in_file(
            &self.runtime.config_path,
            &format!("ui.custom_keymap.{}", action.key()),
            &literal,
        ) {
            Ok(update) => {
                apply_runtime_update(self, update.runtime);
                self.keymap_editor.mode = KeymapEditorMode::Browse;
                self.keymap_editor.capture_tokens.clear();
                self.status = format!(
                    "custom binding updated: {} = {}",
                    action.key(),
                    update.rendered_value
                );
            }
            Err(error) => {
                self.status = format!("failed to set custom binding {}: {error}", action.key());
            }
        }
    }

    fn cycle_keymap_editor_value(&mut self, forward: bool) {
        match self.selected_keymap_editor_field() {
            KeymapEditorField::ActiveScheme => {
                let next = cycle_keymap_scheme(self.runtime.ui_keymap, forward);
                match update_config_key_in_file(
                    &self.runtime.config_path,
                    "ui.keymap",
                    next.as_str(),
                ) {
                    Ok(update) => {
                        apply_runtime_update(self, update.runtime);
                        self.status = format!("active keymap set to {}", next.as_str());
                    }
                    Err(error) => {
                        self.status = format!("failed to set ui.keymap: {error}");
                    }
                }
            }
            KeymapEditorField::CustomBase => {
                let next = cycle_keymap_base(self.runtime.ui_keymap_base, forward);
                match update_config_key_in_file(
                    &self.runtime.config_path,
                    "ui.keymap_base",
                    next.as_str(),
                ) {
                    Ok(update) => {
                        apply_runtime_update(self, update.runtime);
                        self.status = format!("custom keymap base set to {}", next.as_str());
                    }
                    Err(error) => {
                        self.status = format!("failed to set ui.keymap_base: {error}");
                    }
                }
            }
            KeymapEditorField::Action(_) => {
                self.status = "press Enter or e to record a custom binding".to_string();
            }
        }
    }

    fn reset_selected_keymap_field(&mut self) {
        match self.selected_keymap_editor_field() {
            KeymapEditorField::ActiveScheme => {
                match remove_config_key_from_file(&self.runtime.config_path, "ui.keymap") {
                    Ok(Some(runtime)) => {
                        apply_runtime_update(self, runtime);
                        self.status = "ui.keymap reset to default".to_string();
                    }
                    Ok(None) => {
                        self.status = "ui.keymap already uses the default".to_string();
                    }
                    Err(error) => {
                        self.status = format!("failed to reset ui.keymap: {error}");
                    }
                }
            }
            KeymapEditorField::CustomBase => {
                match remove_config_key_from_file(&self.runtime.config_path, "ui.keymap_base") {
                    Ok(Some(runtime)) => {
                        apply_runtime_update(self, runtime);
                        self.status = "ui.keymap_base reset to inferred default".to_string();
                    }
                    Ok(None) => {
                        self.status =
                            "ui.keymap_base already uses the inferred default".to_string();
                    }
                    Err(error) => {
                        self.status = format!("failed to reset ui.keymap_base: {error}");
                    }
                }
            }
            KeymapEditorField::Action(action) => {
                match remove_config_key_from_file(
                    &self.runtime.config_path,
                    &format!("ui.custom_keymap.{}", action.key()),
                ) {
                    Ok(Some(runtime)) => {
                        apply_runtime_update(self, runtime);
                        self.status = format!("custom binding reset: {}", action.key());
                    }
                    Ok(None) => {
                        self.status = format!("custom binding already inherited: {}", action.key());
                    }
                    Err(error) => {
                        self.status =
                            format!("failed to reset custom binding {}: {error}", action.key());
                    }
                }
            }
        }
    }

    fn reset_all_custom_key_bindings(&mut self) {
        match remove_config_key_from_file(&self.runtime.config_path, "ui.custom_keymap") {
            Ok(Some(runtime)) => {
                apply_runtime_update(self, runtime);
                self.status = "all custom key bindings reset".to_string();
            }
            Ok(None) => {
                self.status = "no custom key bindings to reset".to_string();
            }
            Err(error) => {
                self.status = format!("failed to reset custom key bindings: {error}");
            }
        }
    }
}

pub(super) fn resolve_active_main_page_keymap(runtime: &RuntimeConfig) -> ResolvedMainPageKeymap {
    match runtime.ui_keymap {
        UiKeymap::Default => preset_main_page_keymap(UiKeymapBase::Default),
        UiKeymap::Vim => preset_main_page_keymap(UiKeymapBase::Vim),
        UiKeymap::Custom => resolve_custom_main_page_keymap(runtime),
    }
}

pub(super) fn resolve_custom_main_page_keymap(runtime: &RuntimeConfig) -> ResolvedMainPageKeymap {
    let base = preset_main_page_keymap(runtime.ui_keymap_base);
    overlay_custom_keymap(base, &runtime.ui_custom_keymap)
}

pub(super) fn main_page_focus_shortcuts(keymap: &ResolvedMainPageKeymap) -> String {
    format!(
        "{}/{}",
        keymap.focus_previous.display(),
        keymap.focus_next.display()
    )
}

pub(super) fn main_page_move_shortcuts(keymap: &ResolvedMainPageKeymap) -> String {
    match keymap.move_shortcut_order {
        MoveShortcutOrder::UpThenDown => {
            format!(
                "{}/{}",
                keymap.move_up.display(),
                keymap.move_down.display()
            )
        }
        MoveShortcutOrder::DownThenUp => {
            format!(
                "{}/{}",
                keymap.move_down.display(),
                keymap.move_up.display()
            )
        }
    }
}

pub(super) fn main_page_navigation_shortcuts(keymap: &ResolvedMainPageKeymap) -> String {
    format!(
        "{} focus | {} move",
        main_page_focus_shortcuts(keymap),
        main_page_move_shortcuts(keymap)
    )
}

pub(super) fn handle_main_page_key_event(
    state: &mut AppState,
    key: KeyEvent,
) -> Option<LoopAction> {
    if key
        .modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER)
    {
        state.pending_main_page_sequence = None;
        return None;
    }

    let KeyCode::Char(character) = key.code else {
        state.pending_main_page_sequence = None;
        return None;
    };

    let pending_prefix = state.take_pending_main_page_sequence().unwrap_or_default();
    let combined = extend_sequence(&pending_prefix, character);
    if let Some(action) = state.main_page_keymap.action_for_sequence(&combined) {
        state.pending_main_page_sequence = None;
        return Some(execute_main_page_action(state, action));
    }
    if state.main_page_keymap.has_pending_prefix(&combined) {
        state.clear_pending_main_page_count();
        set_pending_main_page_status(state, &combined);
        state.pending_main_page_sequence = Some(state.pending_main_page_sequence_state(combined));
        return Some(LoopAction::Continue);
    }

    let single = vec![character];
    if let Some(action) = state.main_page_keymap.action_for_sequence(&single) {
        state.pending_main_page_sequence = None;
        return Some(execute_main_page_action(state, action));
    }
    if state.main_page_keymap.has_pending_prefix(&single) {
        state.clear_pending_main_page_count();
        set_pending_main_page_status(state, &single);
        state.pending_main_page_sequence = Some(state.pending_main_page_sequence_state(single));
        return Some(LoopAction::Continue);
    }

    state.pending_main_page_sequence = None;
    None
}

pub(super) fn handle_keymap_editor_key_event(state: &mut AppState, key: KeyEvent) -> LoopAction {
    match state.keymap_editor.mode {
        KeymapEditorMode::Browse => match key.code {
            KeyCode::Esc => state.close_keymap_editor(),
            KeyCode::Up | KeyCode::Char('i') => state.move_keymap_editor_up(),
            KeyCode::Down | KeyCode::Char('k') => state.move_keymap_editor_down(),
            KeyCode::Left => state.cycle_keymap_editor_value(false),
            KeyCode::Right | KeyCode::Tab => state.cycle_keymap_editor_value(true),
            KeyCode::Enter | KeyCode::Char('e') => match state.selected_keymap_editor_field() {
                KeymapEditorField::Action(_) => state.start_keymap_binding_capture(),
                _ => state.cycle_keymap_editor_value(true),
            },
            KeyCode::Char('x') => state.reset_selected_keymap_field(),
            KeyCode::Char('R') => state.reset_all_custom_key_bindings(),
            _ => {}
        },
        KeymapEditorMode::Capture => match key.code {
            KeyCode::Esc => state.cancel_keymap_binding_capture(),
            KeyCode::Enter => state.save_keymap_binding_capture(),
            KeyCode::Backspace => {
                state.keymap_editor.capture_tokens.pop();
            }
            _ => {
                let KeymapEditorField::Action(action) = state.selected_keymap_editor_field() else {
                    return LoopAction::Continue;
                };
                state.capture_keymap_binding_token(action, key);
            }
        },
    }

    LoopAction::Continue
}

pub(super) fn draw_keymap_editor(frame: &mut Frame<'_>, state: &AppState) {
    let custom_keymap = resolve_custom_main_page_keymap(&state.runtime);
    let area = centered_rect(88, 78, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title("Keymap Editor")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(10),
            Constraint::Length(4),
        ])
        .split(inner);

    let header = format!(
        "active={} | custom base={} | command=:keymap",
        state.runtime.ui_keymap.as_str(),
        state.runtime.ui_keymap_base.as_str()
    );
    frame.render_widget(
        Paragraph::new(header).wrap(Wrap { trim: false }),
        sections[0],
    );

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(42), Constraint::Percentage(58)])
        .split(sections[1]);

    let selected_index = state
        .keymap_editor
        .selected_field
        .min(KEYMAP_EDITOR_FIELDS.len().saturating_sub(1));
    let mut list_state = ListState::default();
    list_state.select(Some(selected_index));

    let items: Vec<ListItem> = KEYMAP_EDITOR_FIELDS
        .iter()
        .map(|field| {
            let value = keymap_editor_field_value(*field, state, &custom_keymap);
            let text = truncate_with_ellipsis(
                &format!("{:<14} {}", field.label(), value),
                body[0].width.saturating_sub(3) as usize,
            );
            ListItem::new(text)
        })
        .collect();
    let list = List::new(items)
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");
    frame.render_stateful_widget(list, body[0], &mut list_state);

    let details =
        Paragraph::new(keymap_editor_details(state, &custom_keymap)).wrap(Wrap { trim: false });
    frame.render_widget(details, body[1]);

    let footer = match state.keymap_editor.mode {
        KeymapEditorMode::Browse => {
            "Up/Down move | Left/Right/Tab cycle | Enter/e record | x reset | R reset custom | Esc close"
        }
        KeymapEditorMode::Capture => {
            "Type 1-2 printable keys | Backspace delete | Enter save | Esc cancel"
        }
    };
    frame.render_widget(
        Paragraph::new(footer).wrap(Wrap { trim: false }),
        sections[2],
    );
}

fn preset_main_page_keymap(base: UiKeymapBase) -> ResolvedMainPageKeymap {
    match base {
        UiKeymapBase::Default => ResolvedMainPageKeymap {
            focus_previous: KeySequence::from_chars(vec!['j']),
            focus_next: KeySequence::from_chars(vec!['l']),
            move_up: KeySequence::from_chars(vec!['i']),
            move_down: KeySequence::from_chars(vec!['k']),
            move_shortcut_order: MoveShortcutOrder::UpThenDown,
            jump_top: None,
            jump_bottom: None,
            quick_quit: None,
        },
        UiKeymapBase::Vim => ResolvedMainPageKeymap {
            focus_previous: KeySequence::from_chars(vec!['h']),
            focus_next: KeySequence::from_chars(vec!['l']),
            move_up: KeySequence::from_chars(vec!['k']),
            move_down: KeySequence::from_chars(vec!['j']),
            move_shortcut_order: MoveShortcutOrder::DownThenUp,
            jump_top: Some(KeySequence::from_chars(vec!['g', 'g'])),
            jump_bottom: Some(KeySequence::from_chars(vec!['G'])),
            quick_quit: Some(KeySequence::from_chars(vec!['q', 'q'])),
        },
    }
}

fn overlay_custom_keymap(
    base: ResolvedMainPageKeymap,
    custom: &UiCustomKeymapConfig,
) -> ResolvedMainPageKeymap {
    let ResolvedMainPageKeymap {
        focus_previous,
        focus_next,
        move_up,
        move_down,
        move_shortcut_order,
        jump_top,
        jump_bottom,
        quick_quit,
    } = base;
    ResolvedMainPageKeymap {
        focus_previous: custom
            .focus_prev
            .as_deref()
            .map(KeySequence::from_config_tokens)
            .unwrap_or(focus_previous),
        focus_next: custom
            .focus_next
            .as_deref()
            .map(KeySequence::from_config_tokens)
            .unwrap_or(focus_next),
        move_up: custom
            .move_up
            .as_deref()
            .map(KeySequence::from_config_tokens)
            .unwrap_or(move_up),
        move_down: custom
            .move_down
            .as_deref()
            .map(KeySequence::from_config_tokens)
            .unwrap_or(move_down),
        move_shortcut_order,
        jump_top: custom
            .jump_top
            .as_deref()
            .map(KeySequence::from_config_tokens)
            .or(jump_top),
        jump_bottom: custom
            .jump_bottom
            .as_deref()
            .map(KeySequence::from_config_tokens)
            .or(jump_bottom),
        quick_quit: custom
            .quick_quit
            .as_deref()
            .map(KeySequence::from_config_tokens)
            .or(quick_quit),
    }
}

fn execute_main_page_action(state: &mut AppState, action: MainPageAction) -> LoopAction {
    match action {
        MainPageAction::FocusPrevious => {
            state.clear_pending_main_page_count();
            state.move_focus_previous();
            LoopAction::Continue
        }
        MainPageAction::FocusNext => {
            state.clear_pending_main_page_count();
            state.move_focus_next();
            LoopAction::Continue
        }
        MainPageAction::MoveUp => {
            for _ in 0..pending_main_page_move_count(state) {
                state.move_up();
            }
            LoopAction::Continue
        }
        MainPageAction::MoveDown => {
            for _ in 0..pending_main_page_move_count(state) {
                state.move_down();
            }
            LoopAction::Continue
        }
        MainPageAction::JumpTop => {
            state.clear_pending_main_page_count();
            state.jump_current_pane_to_start();
            LoopAction::Continue
        }
        MainPageAction::JumpBottom => {
            state.clear_pending_main_page_count();
            state.jump_current_pane_to_end();
            LoopAction::Continue
        }
        MainPageAction::QuickQuit => {
            state.clear_pending_main_page_count();
            LoopAction::Exit
        }
    }
}

fn set_pending_main_page_status(state: &mut AppState, input: &[char]) {
    let Some(quick_quit) = state.main_page_keymap.binding(MainPageAction::QuickQuit) else {
        return;
    };
    if !quick_quit.starts_with(input) || quick_quit.as_chars() == input {
        return;
    }

    state.status = format!(
        "press {} to quit or use command palette quit/exit",
        quick_quit.display()
    );
}

fn extend_sequence(prefix: &[char], next: char) -> Vec<char> {
    let mut sequence = prefix.to_vec();
    sequence.push(next);
    sequence
}

fn cycle_keymap_scheme(current: UiKeymap, forward: bool) -> UiKeymap {
    const ORDER: [UiKeymap; 3] = [UiKeymap::Default, UiKeymap::Vim, UiKeymap::Custom];
    cycle_enum_value(&ORDER, current, forward)
}

fn cycle_keymap_base(current: UiKeymapBase, forward: bool) -> UiKeymapBase {
    const ORDER: [UiKeymapBase; 2] = [UiKeymapBase::Default, UiKeymapBase::Vim];
    cycle_enum_value(&ORDER, current, forward)
}

fn cycle_enum_value<T: Copy + PartialEq>(values: &[T], current: T, forward: bool) -> T {
    let index = values
        .iter()
        .position(|value| *value == current)
        .unwrap_or(0);
    if forward {
        values[(index + 1) % values.len()]
    } else if index == 0 {
        values[values.len().saturating_sub(1)]
    } else {
        values[index - 1]
    }
}

fn keymap_editor_field_value(
    field: KeymapEditorField,
    state: &AppState,
    custom_keymap: &ResolvedMainPageKeymap,
) -> String {
    match field {
        KeymapEditorField::ActiveScheme => state.runtime.ui_keymap.as_str().to_string(),
        KeymapEditorField::CustomBase => state.runtime.ui_keymap_base.as_str().to_string(),
        KeymapEditorField::Action(action) => format!(
            "{} [{}]",
            render_binding_text(state.main_page_keymap.binding(action)),
            active_keymap_binding_source_label(state, action, custom_keymap)
        ),
    }
}

fn keymap_editor_details(state: &AppState, custom_keymap: &ResolvedMainPageKeymap) -> String {
    match state.selected_keymap_editor_field() {
        KeymapEditorField::ActiveScheme => format!(
            "Select which main-page keymap is active now.\n\n\
             default: j/l focus, i/k move\n\
             vim: h/l focus, j/k move, gg/G jump, qq quit\n\
             custom: use ui.keymap_base plus ui.custom_keymap overrides\n\n\
             Current active shortcuts: {}\n",
            main_page_navigation_shortcuts(&state.main_page_keymap)
        ),
        KeymapEditorField::CustomBase => format!(
            "Choose the preset that seeds the custom scheme.\n\n\
             The custom scheme inherits any action without an override.\n\
             Current base: {}\n\
             Current custom shortcuts: {}\n",
            state.runtime.ui_keymap_base.as_str(),
            main_page_navigation_shortcuts(custom_keymap)
        ),
        KeymapEditorField::Action(action) => {
            let custom_override = action_custom_binding(&state.runtime, action);
            let active_binding = render_binding_text(state.main_page_keymap.binding(action));
            let custom_binding = render_binding_text(custom_keymap.binding(action));
            let source = if custom_override.is_some() {
                "custom override".to_string()
            } else {
                format!("base {}", state.runtime.ui_keymap_base.as_str())
            };
            let capture = if matches!(state.keymap_editor.mode, KeymapEditorMode::Capture) {
                format!(
                    "\nCapture buffer: {}\n",
                    state
                        .keymap_editor
                        .capture_tokens
                        .iter()
                        .collect::<String>()
                )
            } else {
                String::new()
            };
            format!(
                "{}\n\n\
                 {}\n\n\
                 Active scheme: {}\n\
                 Active binding now: {}\n\
                 Custom binding: {}\n\
                 Custom source: {}\n\
                 Config key: ui.custom_keymap.{}\n\
                 Override in file: {}\n{}",
                action.label(),
                action.description(),
                state.runtime.ui_keymap.as_str(),
                active_binding,
                custom_binding,
                source,
                action.key(),
                custom_override
                    .map(|binding| format_toml_key_sequence_literal(&binding))
                    .unwrap_or_else(|| "<inherit>".to_string()),
                capture
            )
        }
    }
}

fn action_custom_binding(runtime: &RuntimeConfig, action: MainPageAction) -> Option<Vec<char>> {
    let binding = match action {
        MainPageAction::FocusPrevious => runtime.ui_custom_keymap.focus_prev.as_ref(),
        MainPageAction::FocusNext => runtime.ui_custom_keymap.focus_next.as_ref(),
        MainPageAction::MoveUp => runtime.ui_custom_keymap.move_up.as_ref(),
        MainPageAction::MoveDown => runtime.ui_custom_keymap.move_down.as_ref(),
        MainPageAction::JumpTop => runtime.ui_custom_keymap.jump_top.as_ref(),
        MainPageAction::JumpBottom => runtime.ui_custom_keymap.jump_bottom.as_ref(),
        MainPageAction::QuickQuit => runtime.ui_custom_keymap.quick_quit.as_ref(),
    }?;
    Some(
        binding
            .iter()
            .filter_map(|token| token.chars().next())
            .collect(),
    )
}

fn active_keymap_binding_source_label(
    state: &AppState,
    action: MainPageAction,
    custom_keymap: &ResolvedMainPageKeymap,
) -> String {
    match state.runtime.ui_keymap {
        UiKeymap::Default => "default".to_string(),
        UiKeymap::Vim => "vim".to_string(),
        UiKeymap::Custom => {
            if action_custom_binding(&state.runtime, action).is_some() {
                "custom".to_string()
            } else if custom_keymap.binding(action).is_some() {
                format!("base {}", state.runtime.ui_keymap_base.as_str())
            } else {
                "none".to_string()
            }
        }
    }
}

fn render_binding_text(binding: Option<&KeySequence>) -> String {
    binding
        .map(KeySequence::display)
        .unwrap_or_else(|| "<none>".to_string())
}

fn format_toml_key_sequence_literal(tokens: &[char]) -> String {
    format!(
        "[{}]",
        tokens
            .iter()
            .map(|token| format!("\"{}\"", escape_toml_char(*token)))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn escape_toml_char(character: char) -> String {
    match character {
        '"' => "\\\"".to_string(),
        '\\' => "\\\\".to_string(),
        _ => character.to_string(),
    }
}

fn is_bindable_keymap_character(character: char) -> bool {
    !character.is_ascii_control() && !character.is_ascii_whitespace() && !character.is_ascii_digit()
}

fn is_reserved_keymap_character(character: char) -> bool {
    matches!(
        character,
        ':' | '/'
            | 'e'
            | 'r'
            | 'a'
            | 'd'
            | 'u'
            | 'y'
            | 'n'
            | '['
            | ']'
            | '{'
            | '}'
            | 'E'
            | '-'
            | '='
            | '+'
    )
}
