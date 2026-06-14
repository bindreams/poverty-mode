//! Pure, headless reducer for the interactive proxy-selection TUI.
//!
//! All UI *meaning* lives here so it can be unit-tested without a terminal.
//! `src/tui.rs` is a thin render/event shell that translates key events into
//! [`TuiAction`]s and feeds them to [`TuiState::apply`].

use crate::config::{ProxySettings, ResolvedProxy};
use crate::proxy::ProxyName;

#[cfg(test)]
#[path = "reducer_tests.rs"]
mod reducer_tests;

/// One selectable row in the TUI: a proxy and whether it is enabled.
///
/// The shape `{ name, enabled }` is locked by the implementation contract; the
/// per-proxy settings needed to build a [`ResolvedProxy`] on confirm are held
/// alongside in [`TuiState`] (a parallel `settings` vector), not on the item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TuiItem {
    pub name: ProxyName,
    pub enabled: bool,
}

/// Reducer state: the ordered list of rows, the cursor position, the per-row
/// settings carried through to confirm, and a transient UX hint.
pub struct TuiState {
    pub items: Vec<TuiItem>,
    pub cursor: usize,
    /// Parallel to `items`: `settings[i]` belongs to `items[i]`. Kept off
    /// `TuiItem` because the item shape is contract-locked to `{name, enabled}`.
    settings: Vec<ProxySettings>,
    /// Transient feedback for the last action (e.g. an attempt to move a row
    /// past central). `Some` only immediately after a rejected action; cleared
    /// by the next action that succeeds or is unconstrained. Surfaced by the
    /// render layer (spec §5.10: "reorder past it is rejected with a hint").
    hint: Option<&'static str>,
}

/// A user intent, produced by the render shell from a key event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TuiAction {
    Up,
    Down,
    Toggle,
    MoveUp,
    MoveDown,
    Confirm,
    Cancel,
}

/// Result of applying an action: whether to keep looping, run a resolved
/// chain, or cancel.
#[derive(Clone, Debug, PartialEq)]
pub enum TuiOutcome {
    Continue,
    Run(Vec<ResolvedProxy>),
    Cancel,
}

impl TuiState {
    /// Seed the reducer from `(item, settings)` pairs in display/chain order.
    ///
    /// The cursor starts at the top. The central-last invariant is enforced on
    /// construction so a malformed seed cannot present central above a later
    /// proxy. Row names are required to be unique (one row per `ProxyName`) —
    /// the v1 config guarantees this and several reducer operations
    /// (`restore_cursor`) rely on it, so it is asserted in debug builds.
    pub fn new(seed: Vec<(TuiItem, ProxySettings)>) -> Self {
        let (items, settings): (Vec<TuiItem>, Vec<ProxySettings>) = seed.into_iter().unzip();
        debug_assert!(
            {
                let mut names: Vec<ProxyName> = items.iter().map(|i| i.name).collect();
                names.sort_by_key(|n| n.as_str());
                names.dedup();
                names.len() == items.len()
            },
            "TuiState rows must have unique proxy names (one row per ProxyName)"
        );
        let mut st = TuiState {
            items,
            cursor: 0,
            settings,
            hint: None,
        };
        st.enforce_central_last();
        st
    }

    /// The transient UX hint from the last action, if any. Read by the render
    /// layer; pure so it is unit-testable.
    pub fn hint(&self) -> Option<&'static str> {
        self.hint
    }

    /// Apply one action, returning the outcome.
    pub fn apply(&mut self, a: TuiAction) -> TuiOutcome {
        match a {
            TuiAction::Up => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                }
                TuiOutcome::Continue
            }
            TuiAction::Down => {
                if self.cursor + 1 < self.items.len() {
                    self.cursor += 1;
                }
                TuiOutcome::Continue
            }
            TuiAction::Toggle => {
                let acted = self.cursor_name();
                if let Some(item) = self.items.get_mut(self.cursor) {
                    item.enabled = !item.enabled;
                }
                self.enforce_central_last();
                self.restore_cursor(acted);
                self.hint = None;
                TuiOutcome::Continue
            }
            TuiAction::MoveUp => {
                self.reorder(self.cursor.checked_sub(1));
                TuiOutcome::Continue
            }
            TuiAction::MoveDown => {
                self.reorder(Some(self.cursor + 1));
                TuiOutcome::Continue
            }
            TuiAction::Confirm | TuiAction::Cancel => TuiOutcome::Continue,
        }
    }

    /// Attempt to swap the cursor row with the row at `target` (if any/in range),
    /// then re-assert central-last. If the swap was undone by the central-last
    /// coercion (i.e. the user tried to push a row past central or lift central),
    /// set the reject hint; if the swap stuck or no swap happened (end clamp),
    /// clear it. The cursor follows the acted-on row in all cases.
    fn reorder(&mut self, target: Option<usize>) {
        let acted = self.cursor_name();
        let snapshot: Vec<ProxyName> = self.items.iter().map(|i| i.name).collect();
        let mut attempted = false;
        if let Some(j) = target {
            if j < self.items.len() {
                self.swap_rows(self.cursor, j);
                attempted = true;
            }
        }
        self.enforce_central_last();
        self.restore_cursor(acted);
        let order_now: Vec<ProxyName> = self.items.iter().map(|i| i.name).collect();
        // A hint is warranted only when a swap was attempted but the order is
        // back to where it started (central-last undid it). A clamped end-move
        // (no swap attempted) or a successful reorder leaves no hint.
        self.hint = if attempted && order_now == snapshot {
            Some("central must stay last")
        } else {
            None
        };
    }

    /// The proxy the cursor currently points at, if any.
    fn cursor_name(&self) -> Option<ProxyName> {
        self.items.get(self.cursor).map(|i| i.name)
    }

    /// Re-point the cursor at the row identified by `name` after a structural
    /// change. Relies on unique row names (asserted in `TuiState::new`). If
    /// `name` is gone (it never is in v1), the cursor is clamped in range.
    fn restore_cursor(&mut self, name: Option<ProxyName>) {
        if let Some(name) = name {
            if let Some(idx) = self.items.iter().position(|i| i.name == name) {
                self.cursor = idx;
                return;
            }
        }
        if self.cursor >= self.items.len() && !self.items.is_empty() {
            self.cursor = self.items.len() - 1;
        }
    }

    /// Swap two rows, keeping `items` and `settings` aligned so per-row
    /// settings travel with their row.
    fn swap_rows(&mut self, i: usize, j: usize) {
        self.items.swap(i, j);
        self.settings.swap(i, j);
    }

    /// Test-only view of the settings parallel to `items[i]`. Lets reorder tests
    /// confirm settings travel with their row before `Confirm` exists (M9.7).
    #[cfg(test)]
    pub(crate) fn settings_at(&self, i: usize) -> &ProxySettings {
        &self.settings[i]
    }

    /// Move `central` (if present) to the end of `items`/`settings`, preserving
    /// the relative order of every other row. No-op if central is absent or
    /// already last. The cursor is not adjusted here (callers that move rows
    /// adjust the cursor themselves).
    fn enforce_central_last(&mut self) {
        let Some(idx) = self.items.iter().position(|i| i.name.must_be_last()) else {
            return;
        };
        if idx == self.items.len() - 1 {
            return;
        }
        let item = self.items.remove(idx);
        let setting = self.settings.remove(idx);
        self.items.push(item);
        self.settings.push(setting);
    }
}
