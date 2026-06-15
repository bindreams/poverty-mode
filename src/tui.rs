//! Interactive proxy-selection TUI (ratatui + crossterm).
//!
//! The render loop here is intentionally thin: it owns terminal lifecycle and
//! key mapping only. Every decision is delegated to [`reducer::TuiState`]. The
//! render walks `state.rows()` into a collapsible tree of proxy headers and
//! their expanded settings, then the `Start`/`Cancel` buttons; all value text
//! comes from [`reducer::settings`] (`render_value`/`describe`) so formatting
//! is never duplicated here.
//!
//! Third-party API pinned + verified: ratatui 0.30 (`init`/`restore`,
//! `DefaultTerminal`, `Frame::area`, `Layout::vertical(..).areas`,
//! `render_widget`, `Paragraph`, `Style`/`Modifier`, `Line`/`Span`); crossterm
//! 0.29 (`event::read`, `Event::Key`, `KeyEvent`/`KeyCode`/`KeyModifiers`/
//! `KeyEventKind::Press`).

pub mod reducer;

use std::io::IsTerminal;

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::{DefaultTerminal, Frame};

use crate::config::Config;
use crate::config::ResolvedProxy;
use crate::proxy::ProxyName;
use reducer::focus::Focus;
use reducer::settings::{self, settings_of};
use reducer::{TuiAction, TuiOutcome, TuiState};

#[cfg(test)]
#[path = "tui_tests.rs"]
mod tui_tests;

/// Errors specific to the interactive picker.
#[derive(Debug, thiserror::Error)]
pub enum TuiError {
    /// `run --interactive` was requested but stdio is not a terminal (e.g. CI,
    /// a pipe, or a redirect). The picker needs a TTY; fail loudly rather than
    /// hang on `event::read`.
    #[error("interactive picker requires a terminal (stdin/stdout is not a TTY)")]
    NotATerminal,
}

/// Run the interactive proxy picker, returning the user's terminal choice.
///
/// Seeds a [`TuiState`] from the RESOLVED chain (spec §5.10: "Seeded from the
/// resolved chain"), overlaid onto `config` so every known proxy stays togglable.
/// `resolved` is the caller's cli>env>file resolution — so `--proxies` and
/// `POVERTY_PROXY_CHAIN` feed the picker's initial selection/order rather than
/// being silently dropped. Runs the ratatui event loop and returns the reducer's
/// terminal [`TuiOutcome`] (`Run(..)` on Start, `Cancel` on Esc/Ctrl-C/Cancel).
///
/// Terminal lifecycle: `ratatui::init()` enters raw mode + the alternate screen
/// and installs a panic hook that restores the terminal if the loop panics; the
/// explicit `ratatui::restore()` below runs on the normal and `?`-error return
/// paths (a panic is covered by that installed hook instead). If stdio is not a
/// TTY this returns [`TuiError::NotATerminal`] without touching the terminal.
///
/// Keyboard enhancement: when the host terminal supports the kitty keyboard
/// protocol we push `DISAMBIGUATE_ESCAPE_CODES` so modified whitespace keys
/// (Shift+Enter / Shift+Space) arrive with their SHIFT bit set — otherwise legacy
/// terminals send them identically to plain Enter/Space and the Expand binding is
/// unreachable. The flags are popped before `restore()` on the normal and error
/// paths. (Tab is the protocol-independent expand fallback, so the picker is fully
/// usable even when enhancement is unsupported.)
pub fn run_picker(config: &Config, resolved: &[ResolvedProxy]) -> anyhow::Result<TuiOutcome> {
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        return Err(TuiError::NotATerminal.into());
    }
    let mut state = TuiState::from_config_and_resolved(config, resolved);
    let mut terminal = ratatui::init();
    let enhanced = push_keyboard_enhancement();
    let result = event_loop(&mut terminal, &mut state);
    if enhanced {
        pop_keyboard_enhancement();
    }
    ratatui::restore();
    result
}

/// Enable the kitty keyboard protocol's escape-code disambiguation when the
/// terminal advertises support, so Shift+Enter / Shift+Space are reported with
/// their modifier. Returns whether the flags were pushed (so the caller knows to
/// pop them). Best-effort: any failure leaves the picker in legacy mode, where Tab
/// still drives expand/collapse.
fn push_keyboard_enhancement() -> bool {
    use crossterm::event::{KeyboardEnhancementFlags, PushKeyboardEnhancementFlags};
    if matches!(crossterm::terminal::supports_keyboard_enhancement(), Ok(true)) {
        crossterm::execute!(
            std::io::stdout(),
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        )
        .is_ok()
    } else {
        false
    }
}

/// Pop the keyboard-enhancement flags pushed by [`push_keyboard_enhancement`],
/// restoring the terminal's prior keyboard mode. Best-effort.
fn pop_keyboard_enhancement() {
    use crossterm::event::PopKeyboardEnhancementFlags;
    let _ = crossterm::execute!(std::io::stdout(), PopKeyboardEnhancementFlags);
}

/// The blocking draw/read loop. Returns when the reducer yields a terminal
/// outcome (`Run`/`Cancel`).
fn event_loop(terminal: &mut DefaultTerminal, state: &mut TuiState) -> anyhow::Result<TuiOutcome> {
    loop {
        terminal.draw(|frame| render(frame, state))?;
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        let Some(action) = map_key(key.code, key.modifiers, state.is_editing()) else {
            continue;
        };
        match state.apply(action) {
            TuiOutcome::Continue => continue,
            terminal_outcome => return Ok(terminal_outcome),
        }
    }
}

/// Translate a crossterm key + modifiers into a reducer action, or `None` to
/// ignore. `editing` routes the keymap into a text-editor mode where printable
/// keys insert characters; otherwise keys drive the tree (Activate/Expand,
/// Cycle, Move) and Esc/Ctrl-C cancel.
fn map_key(code: KeyCode, mods: KeyModifiers, editing: bool) -> Option<TuiAction> {
    let shift = mods.contains(KeyModifiers::SHIFT);
    let ctrl = mods.contains(KeyModifiers::CONTROL);
    if editing {
        return match code {
            // Ctrl-C aborts the edit; it must NOT insert a control character.
            KeyCode::Char('c') if ctrl => Some(TuiAction::EditAbort),
            KeyCode::Char(c) if !ctrl => Some(TuiAction::EditChar(c)),
            KeyCode::Backspace => Some(TuiAction::EditBackspace),
            KeyCode::Enter => Some(TuiAction::EditCommit),
            KeyCode::Esc => Some(TuiAction::EditAbort),
            _ => None,
        };
    }
    match code {
        KeyCode::Up if shift => Some(TuiAction::MoveUp),
        KeyCode::Down if shift => Some(TuiAction::MoveDown),
        KeyCode::Up => Some(TuiAction::Up),
        KeyCode::Down => Some(TuiAction::Down),
        KeyCode::Left => Some(TuiAction::CycleLeft),
        KeyCode::Right => Some(TuiAction::CycleRight),
        // Tab is the UNIVERSAL expand/collapse toggle: every terminal delivers it
        // unmodified. Shift+Enter/Shift+Space below also expand, but the SHIFT bit
        // only reaches us when the keyboard-enhancement protocol is active (see
        // `run_picker`); without it those keys are indistinguishable from plain
        // Enter/Space, so Tab is the dependable fallback. Plain Space/Enter remain
        // Activate.
        KeyCode::Tab => Some(TuiAction::Expand),
        KeyCode::Enter | KeyCode::Char(' ') if shift => Some(TuiAction::Expand),
        KeyCode::Enter | KeyCode::Char(' ') => Some(TuiAction::Activate),
        KeyCode::Esc => Some(TuiAction::Cancel),
        KeyCode::Char('c') if ctrl => Some(TuiAction::Cancel),
        _ => None,
    }
}

/// Draw one full frame: header, the proxy/settings tree, the action buttons, and
/// a footer (chain preview · optional hint · key-help). Pure presentation; all
/// state and all value text come from `state` / [`reducer::settings`].
fn render(frame: &mut Frame, state: &TuiState) {
    let header = "poverty-mode · select proxies";
    let lines = tree_lines(state);
    let body_height = lines.len() as u16;
    let [header_area, _gap1, body_area, _gap2, footer_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(body_height.max(1)),
        Constraint::Length(1),
        Constraint::Length(2),
    ])
    .areas(frame.area());

    frame.render_widget(
        Paragraph::new(Span::styled(header, Style::new().add_modifier(Modifier::BOLD))),
        header_area,
    );
    frame.render_widget(Paragraph::new(lines), body_area);
    frame.render_widget(Paragraph::new(footer_lines(state)), footer_area);
}

/// Build the tree body: each proxy header (`▸/▾ [x] name  description`), its
/// expanded setting rows, then the `Start`/`Cancel` buttons.
fn tree_lines(state: &TuiState) -> Vec<Line<'static>> {
    let mut lines: Vec<Line> = Vec::new();
    for row in state.rows() {
        let focused = state.focus() == Focus::Proxy(row.name);
        let caret = if row.expanded { "▾" } else { "▸" };
        let checkbox = if row.enabled { "[x]" } else { "[ ]" };
        let text = format!(
            "   {caret} {checkbox} {name:<9}  {desc}",
            name = row.name.as_str(),
            desc = settings::describe(row.name, &row.settings),
        );
        let mut style = Style::new();
        if focused {
            style = style.add_modifier(Modifier::REVERSED | Modifier::BOLD);
        } else if row.name == ProxyName::Central && !row.enabled {
            style = style.add_modifier(Modifier::DIM);
        }
        lines.push(Line::from(Span::styled(text, style)));

        if row.expanded {
            for &sid in settings_of(row.name) {
                let focused = state.focus() == Focus::Setting(row.name, sid);
                let value = match state.editing() {
                    Some(e) if e.proxy == row.name && e.setting == sid => {
                        format!("{}▮", e.buffer())
                    }
                    _ => settings::render_value(&row.settings, sid),
                };
                let text = format!("        {label:<13}  {value}", label = sid.label());
                let mut style = Style::new();
                if focused {
                    style = style.add_modifier(Modifier::REVERSED | Modifier::BOLD);
                }
                lines.push(Line::from(Span::styled(text, style)));
            }
        }
    }

    lines.push(button_line("[ Start ]", state.focus() == Focus::Start));
    lines.push(button_line("[ Cancel ]", state.focus() == Focus::Cancel));
    lines
}

/// One action-button line, highlighted when focused.
fn button_line(label: &str, focused: bool) -> Line<'static> {
    let mut style = Style::new();
    if focused {
        style = style.add_modifier(Modifier::REVERSED | Modifier::BOLD);
    }
    Line::from(Span::styled(format!("   {label}"), style))
}

/// The footer: chain preview, the transient hint (if any), then a key-help line.
fn footer_lines(state: &TuiState) -> Vec<Line<'static>> {
    let mut lines: Vec<Line> = vec![Line::from(Span::styled(
        format!("   chain:  {}", state.chain_preview()),
        Style::new().add_modifier(Modifier::DIM),
    ))];
    if let Some(hint) = state.hint() {
        lines.push(Line::from(Span::styled(
            format!("   {hint}"),
            Style::new().add_modifier(Modifier::DIM | Modifier::BOLD),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            "   ↑/↓ move · Space toggle/edit · Shift+↵ expand · ←/→ cycle · Shift+↑/↓ reorder · Esc cancel",
            Style::new().add_modifier(Modifier::DIM),
        )));
    }
    lines
}
