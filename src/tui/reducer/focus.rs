//! Name-based focus model for the interactive picker.
//!
//! [`Focus`] carries a [`ProxyName`] (not a row index), so a focused target
//! survives `enforce_central_last`/reorder unchanged. [`visible_focus`] flattens
//! the rows (each `(name, expanded)`) into the ordered list of focusable targets;
//! [`next_focus`]/[`prev_focus`] step within that list, clamping at the ends.

use super::settings::{settings_of, SettingId};
use crate::proxy::ProxyName;

#[cfg(test)]
#[path = "focus_tests.rs"]
mod focus_tests;

/// A focusable target in the picker: a proxy header, one of its expanded
/// settings, or one of the two action buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Proxy(ProxyName),
    Setting(ProxyName, SettingId),
    Start,
    Cancel,
}

/// Ordered focusable targets from each row's `(name, expanded)`: each proxy
/// header, the proxy's settings when expanded, then the `Start`/`Cancel` buttons.
pub fn visible_focus(rows: &[(ProxyName, bool)]) -> Vec<Focus> {
    let mut out = Vec::new();
    for (name, expanded) in rows {
        out.push(Focus::Proxy(*name));
        if *expanded {
            for sid in settings_of(*name) {
                out.push(Focus::Setting(*name, *sid));
            }
        }
    }
    out.push(Focus::Start);
    out.push(Focus::Cancel);
    out
}

/// The next focusable target after `cur`, clamped at the tail. An unknown `cur`
/// falls back to the first target (or `Start` when the list is empty).
pub fn next_focus(vis: &[Focus], cur: Focus) -> Focus {
    match vis.iter().position(|f| *f == cur) {
        Some(i) if i + 1 < vis.len() => vis[i + 1],
        Some(i) => vis[i],
        None => *vis.first().unwrap_or(&Focus::Start),
    }
}

/// The previous focusable target before `cur`, clamped at the head. An unknown
/// `cur` falls back to the first target (or `Start` when the list is empty).
pub fn prev_focus(vis: &[Focus], cur: Focus) -> Focus {
    match vis.iter().position(|f| *f == cur) {
        Some(0) => vis[0],
        Some(i) => vis[i - 1],
        None => *vis.first().unwrap_or(&Focus::Start),
    }
}
