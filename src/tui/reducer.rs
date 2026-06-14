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
                if let Some(item) = self.items.get_mut(self.cursor) {
                    item.enabled = !item.enabled;
                }
                TuiOutcome::Continue
            }
            TuiAction::MoveUp | TuiAction::MoveDown | TuiAction::Confirm | TuiAction::Cancel => {
                TuiOutcome::Continue
            }
        }
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
