//! Inline-edit buffer state for the picker.
//!
//! [`EditState`] is the transient editor for a single `Text`/`List`/`Number`
//! setting. It is keyed by [`ProxyName`] + [`SettingId`] (consistent with
//! [`super::focus::Focus`]), so it survives reorder/central-last unchanged; the
//! reducer commits the [`buffer`](EditState::buffer) via `SettingId::commit_edit`.

use super::settings::SettingId;
use crate::proxy::ProxyName;

#[cfg(test)]
#[path = "edit_tests.rs"]
mod edit_tests;

/// The active inline editor: which `(proxy, setting)` it targets and the text
/// typed so far.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditState {
    pub proxy: ProxyName,
    pub setting: SettingId,
    buffer: String,
}

impl EditState {
    /// Start editing `(proxy, setting)` with `initial` as the seed buffer.
    pub fn new(proxy: ProxyName, setting: SettingId, initial: impl Into<String>) -> Self {
        Self {
            proxy,
            setting,
            buffer: initial.into(),
        }
    }

    /// The current editor text.
    pub fn buffer(&self) -> &str {
        &self.buffer
    }

    /// Append a typed character.
    pub fn push(&mut self, c: char) {
        self.buffer.push(c);
    }

    /// Delete the last character (no-op on an empty buffer).
    pub fn backspace(&mut self) {
        self.buffer.pop();
    }
}
