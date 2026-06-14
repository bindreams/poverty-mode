//! Interactive proxy-selection TUI (ratatui + crossterm).
//!
//! The render loop here is intentionally thin: it owns terminal lifecycle and
//! key mapping only. Every decision is delegated to [`reducer::TuiState`].
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

use crate::config::{Config, ResolvedProxy};
use crate::proxy::ProxyName;
use reducer::{TuiAction, TuiOutcome, TuiState};

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
/// terminal [`TuiOutcome`] (`Run(..)` on Enter, `Cancel` on Esc/Ctrl-C).
///
/// Terminal lifecycle: `ratatui::init()` enters raw mode + the alternate screen
/// and installs a panic hook that restores the terminal if the loop panics; the
/// explicit `ratatui::restore()` below runs on the normal and `?`-error return
/// paths (a panic is covered by that installed hook instead). If stdio is not a
/// TTY this returns [`TuiError::NotATerminal`] without touching the terminal.
pub fn run_picker(config: &Config, resolved: &[ResolvedProxy]) -> anyhow::Result<TuiOutcome> {
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        return Err(TuiError::NotATerminal.into());
    }
    let mut state = TuiState::from_config_and_resolved(config, resolved);
    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, &mut state);
    ratatui::restore();
    result
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
        let Some(action) = map_key(key.code, key.modifiers) else {
            continue;
        };
        match state.apply(action) {
            TuiOutcome::Continue => continue,
            terminal_outcome => return Ok(terminal_outcome),
        }
    }
}

/// Translate a crossterm key + modifiers into a reducer action, or `None` to
/// ignore. Shift+Up/Down are the reorder keys; Esc and Ctrl-C cancel.
fn map_key(code: KeyCode, mods: KeyModifiers) -> Option<TuiAction> {
    let shift = mods.contains(KeyModifiers::SHIFT);
    let ctrl = mods.contains(KeyModifiers::CONTROL);
    match code {
        KeyCode::Up if shift => Some(TuiAction::MoveUp),
        KeyCode::Down if shift => Some(TuiAction::MoveDown),
        KeyCode::Up => Some(TuiAction::Up),
        KeyCode::Down => Some(TuiAction::Down),
        KeyCode::Char(' ') => Some(TuiAction::Toggle),
        KeyCode::Enter => Some(TuiAction::Confirm),
        KeyCode::Esc => Some(TuiAction::Cancel),
        KeyCode::Char('c') if ctrl => Some(TuiAction::Cancel),
        _ => None,
    }
}

/// A fixed one-line description for each proxy, shown to the right of its name.
fn description(name: ProxyName) -> &'static str {
    match name {
        ProxyName::Pino => "cache-injection + 1h TTL",
        ProxyName::Headroom => "context compression",
        ProxyName::Central => "JetBrains AI  (always last)",
    }
}

/// Draw one full frame: header, rows, chain preview, optional hint. Pure
/// presentation; all state comes from `state`.
fn render(frame: &mut Frame, state: &TuiState) {
    let header =
        "poverty-mode · select proxies (Space toggle · Shift+↑/↓ reorder · Enter run · Esc cancel)";
    let rows = state.items.len() as u16;
    let [header_area, _gap1, list_area, _gap2, footer_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(rows.max(1)),
        Constraint::Length(1),
        Constraint::Length(2),
    ])
    .areas(frame.area());

    frame.render_widget(
        Paragraph::new(Span::styled(
            header,
            Style::new().add_modifier(Modifier::BOLD),
        )),
        header_area,
    );

    let mut lines: Vec<Line> = Vec::with_capacity(state.items.len());
    for (idx, item) in state.items.iter().enumerate() {
        let cursor_marker = if idx == state.cursor { "▸ " } else { "  " };
        let checkbox = if item.enabled { "[x]" } else { "[ ]" };
        let text = format!(
            "   {cursor}{check} {name:<9} {desc}",
            cursor = cursor_marker,
            check = checkbox,
            name = item.name.as_str(),
            desc = description(item.name),
        );
        let mut style = Style::new();
        if idx == state.cursor {
            style = style.add_modifier(Modifier::REVERSED | Modifier::BOLD);
        } else if item.name == ProxyName::Central && !item.enabled {
            style = style.add_modifier(Modifier::DIM);
        }
        lines.push(Line::from(Span::styled(text, style)));
    }
    frame.render_widget(Paragraph::new(lines), list_area);

    // Footer block: chain preview, plus the reject hint when present.
    let mut footer_lines: Vec<Line> = vec![Line::from(Span::styled(
        format!("   chain:  {}", state.chain_preview()),
        Style::new().add_modifier(Modifier::DIM),
    ))];
    if let Some(hint) = state.hint() {
        footer_lines.push(Line::from(Span::styled(
            format!("   {hint}"),
            Style::new().add_modifier(Modifier::DIM | Modifier::BOLD),
        )));
    }
    frame.render_widget(Paragraph::new(footer_lines), footer_area);
}
