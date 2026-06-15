//! Pure, headless reducer for the interactive proxy-selection TUI.
//!
//! All UI *meaning* lives here so it can be unit-tested without a terminal.
//! `src/tui.rs` is a thin render/event shell that translates key events into
//! [`TuiAction`]s and feeds them to [`TuiState::apply`].
//!
//! The model is a flat list of [`ProxyRow`]s (each a proxy, whether it is
//! enabled, whether its settings tree is expanded, and the settings carried to
//! confirm). Focus is **name-based** ([`Focus`] over [`ProxyName`]) so it
//! survives reorder/central-last with no re-anchoring. An optional inline
//! [`EditState`] holds the active text/list/number editor.

use crate::config::{Config, ProxyEntry, ProxySettings, ResolvedProxy};
use crate::proxy::ProxyName;
use edit::EditState;
use focus::{next_focus, prev_focus, visible_focus, Focus};
use settings::SettingKind;

pub mod edit;
pub mod focus;
pub mod settings;

#[cfg(test)]
#[path = "reducer_tests.rs"]
mod reducer_tests;

/// One proxy in the picker: its name, whether it is enabled, whether its
/// settings subtree is expanded, and the settings carried through to confirm.
pub struct ProxyRow {
    pub name: ProxyName,
    pub enabled: bool,
    pub expanded: bool,
    pub settings: ProxySettings,
}

/// Reducer state: the ordered proxy rows, the name-based focus, the optional
/// inline editor, and a transient UX hint (reject feedback or a parse error).
pub struct TuiState {
    rows: Vec<ProxyRow>,
    focus: Focus,
    editing: Option<EditState>,
    /// Transient feedback for the last action: a rejected reorder
    /// (`"central must stay last"`) or a setting parse error. `Some` only until
    /// the next action that succeeds or is unconstrained. Surfaced by the render
    /// layer; widened to `String` to carry parse-error text.
    hint: Option<String>,
}

/// A user intent, produced by the render shell from a key event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TuiAction {
    Up,
    Down,
    Activate,
    Expand,
    CycleLeft,
    CycleRight,
    MoveUp,
    MoveDown,
    Cancel,
    EditChar(char),
    EditBackspace,
    EditCommit,
    EditAbort,
}

/// Result of applying an action: keep looping, run with the full ordered proxy
/// state, or cancel.
#[derive(Clone, Debug, PartialEq)]
pub enum TuiOutcome {
    Continue,
    Run(Vec<ProxyEntry>),
    Cancel,
}

impl TuiState {
    /// Seed the reducer from the RESOLVED chain (spec §5.10: "Seeded from the
    /// resolved chain"), overlaid onto the full set of known proxies from
    /// `config` so every proxy stays togglable.
    ///
    /// The resolved chain (the caller's cli>env>file resolution — honoring
    /// `--proxies` / `POVERTY_PROXY_CHAIN`) supplies the enabled rows, in chain
    /// order and carrying the chain's settings. Every other known proxy from
    /// `config` follows disabled, in the config's relative order, keeping its
    /// existing settings so toggling it on later does not lose customizations.
    /// Central-last is enforced **on construction**, so an Enter that leaves the
    /// seed unmodified yields a central-last entry list. Focus starts on the
    /// first row; nothing is expanded; there is no editor or hint.
    pub fn from_config_and_resolved(config: &Config, resolved: &[ResolvedProxy]) -> Self {
        let in_chain: std::collections::HashSet<ProxyName> = resolved.iter().map(|r| r.name).collect();
        let mut rows: Vec<ProxyRow> = Vec::with_capacity(config.proxies.len());
        // Enabled resolved members first, in chain order, with resolved settings.
        for r in resolved {
            rows.push(ProxyRow {
                name: r.name,
                enabled: true,
                expanded: false,
                settings: r.settings.clone(),
            });
        }
        // Remaining known proxies, disabled, in config order, keeping settings.
        for entry in &config.proxies {
            if in_chain.contains(&entry.name) {
                continue;
            }
            rows.push(ProxyRow {
                name: entry.name,
                enabled: false,
                expanded: false,
                settings: entry.settings.clone(),
            });
        }
        debug_assert!(
            {
                let mut names: Vec<ProxyName> = rows.iter().map(|r| r.name).collect();
                names.sort_by_key(|n| n.as_str());
                names.dedup();
                names.len() == rows.len()
            },
            "TuiState rows must have unique proxy names (one row per ProxyName)"
        );
        let focus = rows.first().map(|r| Focus::Proxy(r.name)).unwrap_or(Focus::Start);
        let mut st = TuiState {
            rows,
            focus,
            editing: None,
            hint: None,
        };
        st.enforce_central_last();
        // The focus may name a row that central-last moved; name-based focus is
        // unaffected. Re-point only if the seed was empty (focus = Start already).
        st.focus = st.rows.first().map(|r| Focus::Proxy(r.name)).unwrap_or(Focus::Start);
        st
    }

    /// The transient UX hint from the last action, if any. Read by the render
    /// layer; pure so it is unit-testable. Exposed as `&str` (via `as_deref`) so
    /// existing literal comparisons (`Some("central must stay last")`) still hold.
    pub fn hint(&self) -> Option<&str> {
        self.hint.as_deref()
    }

    /// A human-readable preview of the resulting request path, e.g.
    /// `"claude → pino → headroom → api.anthropic.com"`. Used by the render
    /// layer; lives here so it is pure and unit-testable.
    pub fn chain_preview(&self) -> String {
        let mut parts: Vec<&str> = vec!["claude"];
        for row in &self.rows {
            if row.enabled {
                parts.push(row.name.as_str());
            }
        }
        parts.push("api.anthropic.com");
        parts.join(" → ")
    }

    /// Apply one action, returning the outcome.
    pub fn apply(&mut self, a: TuiAction) -> TuiOutcome {
        // Editing routes first: only edit actions act while the inline editor is
        // open; everything else is inert (Continue), so navigation/activation
        // keys can't escape an open editor.
        if self.editing.is_some() {
            return self.apply_editing(a);
        }
        match a {
            TuiAction::Up => {
                self.focus = prev_focus(&self.visible(), self.focus);
                self.hint = None;
                TuiOutcome::Continue
            }
            TuiAction::Down => {
                self.focus = next_focus(&self.visible(), self.focus);
                self.hint = None;
                TuiOutcome::Continue
            }
            TuiAction::Activate => self.activate(),
            TuiAction::Expand => {
                self.expand();
                TuiOutcome::Continue
            }
            TuiAction::CycleLeft => {
                self.cycle(-1);
                TuiOutcome::Continue
            }
            TuiAction::CycleRight => {
                self.cycle(1);
                TuiOutcome::Continue
            }
            TuiAction::MoveUp => {
                self.reorder(Direction::Up);
                TuiOutcome::Continue
            }
            TuiAction::MoveDown => {
                self.reorder(Direction::Down);
                TuiOutcome::Continue
            }
            TuiAction::Cancel => TuiOutcome::Cancel,
            // Edit actions are inert when not editing.
            TuiAction::EditChar(_) | TuiAction::EditBackspace | TuiAction::EditCommit | TuiAction::EditAbort => {
                TuiOutcome::Continue
            }
        }
    }

    /// Action routing while the inline editor is open. Only edit actions act; all
    /// others are no-ops (Continue).
    fn apply_editing(&mut self, a: TuiAction) -> TuiOutcome {
        match a {
            TuiAction::EditChar(c) => {
                if let Some(e) = self.editing.as_mut() {
                    e.push(c);
                }
                TuiOutcome::Continue
            }
            TuiAction::EditBackspace => {
                if let Some(e) = self.editing.as_mut() {
                    e.backspace();
                }
                TuiOutcome::Continue
            }
            TuiAction::EditAbort => {
                self.editing = None;
                self.hint = None;
                TuiOutcome::Continue
            }
            TuiAction::EditCommit => {
                self.commit_edit();
                TuiOutcome::Continue
            }
            _ => TuiOutcome::Continue,
        }
    }

    /// Commit the open editor's buffer to its setting. On success, clear the
    /// editor and hint; on a parse error, keep editing and surface the error as
    /// the hint.
    fn commit_edit(&mut self) {
        let Some(editor) = self.editing.as_ref() else {
            return;
        };
        let (proxy, setting, buf) = (editor.proxy, editor.setting, editor.buffer().to_string());
        let row = self.row_mut(proxy);
        match setting.commit_edit(&mut row.settings, &buf) {
            Ok(()) => {
                self.editing = None;
                self.hint = None;
            }
            Err(e) => {
                self.hint = Some(e.to_string());
            }
        }
    }

    /// Activate the focused target: toggle a proxy's enabled flag; toggle/cycle a
    /// bool/enum setting; open the editor for a text/list/number setting; run on
    /// `Start`; cancel on `Cancel`.
    fn activate(&mut self) -> TuiOutcome {
        match self.focus {
            Focus::Proxy(name) => {
                let row = self.row_mut(name);
                row.enabled = !row.enabled;
                self.enforce_central_last();
                self.hint = None;
                TuiOutcome::Continue
            }
            Focus::Setting(name, sid) => {
                match sid.kind() {
                    SettingKind::Bool => sid.toggle(&mut self.row_mut(name).settings),
                    SettingKind::Enum => sid.cycle(&mut self.row_mut(name).settings, 1),
                    SettingKind::Text | SettingKind::List | SettingKind::Number => {
                        let buf = sid.edit_buffer(&self.row(name).settings);
                        self.editing = Some(EditState::new(name, sid, buf));
                    }
                }
                self.hint = None;
                TuiOutcome::Continue
            }
            Focus::Start => TuiOutcome::Run(self.to_entries()),
            Focus::Cancel => TuiOutcome::Cancel,
        }
    }

    /// Expand/collapse from the focused target. On a proxy header, toggle its
    /// expanded flag. On a setting, collapse its proxy and move focus to the
    /// header. On a button, no-op.
    fn expand(&mut self) {
        match self.focus {
            Focus::Proxy(name) => {
                let row = self.row_mut(name);
                row.expanded = !row.expanded;
            }
            Focus::Setting(name, _) => {
                self.row_mut(name).expanded = false;
                self.focus = Focus::Proxy(name);
            }
            Focus::Start | Focus::Cancel => {}
        }
        self.hint = None;
    }

    /// Cycle/toggle the focused setting by `dir` (+1 right, -1 left). Enum cycles,
    /// bool toggles; anything else is a no-op. A cycle keypress always clears any
    /// stale hint, even when it lands on a non-cyclable setting.
    fn cycle(&mut self, dir: i8) {
        if let Focus::Setting(name, sid) = self.focus {
            match sid.kind() {
                SettingKind::Enum => sid.cycle(&mut self.row_mut(name).settings, dir),
                SettingKind::Bool => sid.toggle(&mut self.row_mut(name).settings),
                SettingKind::Text | SettingKind::List | SettingKind::Number => {}
            }
            self.hint = None;
        }
    }

    /// Reorder the focused proxy toward `dir`, then re-assert central-last. Only
    /// acts when a proxy header is focused. If the move is undone by central-last
    /// (pushing past central or lifting central), set the reject hint; a
    /// successful move or a clamped end-move clears it. Focus is name-based, so it
    /// stays on the moved proxy automatically.
    fn reorder(&mut self, dir: Direction) {
        let Focus::Proxy(name) = self.focus else {
            return;
        };
        let Some(i) = self.rows.iter().position(|r| r.name == name) else {
            return;
        };
        let target = match dir {
            Direction::Up => i.checked_sub(1),
            Direction::Down => Some(i + 1),
        };
        let snapshot: Vec<ProxyName> = self.rows.iter().map(|r| r.name).collect();
        let mut attempted = false;
        if let Some(j) = target {
            if j < self.rows.len() {
                self.rows.swap(i, j);
                attempted = true;
            }
        }
        self.enforce_central_last();
        let order_now: Vec<ProxyName> = self.rows.iter().map(|r| r.name).collect();
        // A hint is warranted only when a swap was attempted but the order is back
        // to where it started (central-last undid it). A clamped end-move (no swap)
        // or a successful reorder leaves no hint.
        self.hint = if attempted && order_now == snapshot {
            Some("central must stay last".to_string())
        } else {
            None
        };
    }

    /// The ordered focusable targets derived from the current rows.
    fn visible(&self) -> Vec<Focus> {
        visible_focus(&self.rows.iter().map(|r| (r.name, r.expanded)).collect::<Vec<_>>())
    }

    /// The row for `name` (panics if absent; v1 config guarantees presence).
    fn row(&self, name: ProxyName) -> &ProxyRow {
        self.rows.iter().find(|r| r.name == name).expect("row exists")
    }

    /// The mutable row for `name` (panics if absent).
    fn row_mut(&mut self, name: ProxyName) -> &mut ProxyRow {
        self.rows.iter_mut().find(|r| r.name == name).expect("row exists")
    }

    /// The full ordered proxy list as [`ProxyEntry`]s (the picker's complete
    /// state), in row order with each row's enabled flag and carried settings.
    fn to_entries(&self) -> Vec<ProxyEntry> {
        self.rows
            .iter()
            .map(|r| ProxyEntry {
                name: r.name,
                enabled: r.enabled,
                settings: r.settings.clone(),
            })
            .collect()
    }

    /// Move the central row (if present) to the tail, preserving the relative
    /// order of every other row. No-op if central is absent or already last.
    /// Focus is name-based, so it is unaffected by this move.
    fn enforce_central_last(&mut self) {
        let Some(idx) = self.rows.iter().position(|r| r.name.must_be_last()) else {
            return;
        };
        if idx == self.rows.len() - 1 {
            return;
        }
        let row = self.rows.remove(idx);
        self.rows.push(row);
    }

    // Render accessors ------------------------------------------------------------------------------------------------

    /// The proxy rows in display order (read by the render layer).
    pub fn rows(&self) -> &[ProxyRow] {
        &self.rows
    }

    /// The currently focused target.
    pub fn focus(&self) -> Focus {
        self.focus
    }

    /// Whether the inline editor is open.
    pub fn is_editing(&self) -> bool {
        self.editing.is_some()
    }

    /// The open inline editor, if any.
    pub fn editing(&self) -> Option<&EditState> {
        self.editing.as_ref()
    }

    // Test-only accessors ---------------------------------------------------------------------------------------------

    /// Set the focus directly (test-only seam for asserting per-target behavior).
    #[cfg(test)]
    pub(crate) fn set_focus(&mut self, f: Focus) {
        self.focus = f;
    }

    /// The settings of the proxy named `name` (test-only).
    #[cfg(test)]
    pub(crate) fn settings_of_proxy(&self, name: ProxyName) -> &ProxySettings {
        &self.row(name).settings
    }

    /// The current row order by name (test-only).
    #[cfg(test)]
    pub(crate) fn rows_order(&self) -> Vec<ProxyName> {
        self.rows.iter().map(|r| r.name).collect()
    }
}

/// Direction of a reorder move.
#[derive(Clone, Copy)]
enum Direction {
    Up,
    Down,
}
