use super::*;
use crate::tui::reducer::TuiAction;
use crossterm::event::{KeyCode, KeyModifiers};

fn m(code: KeyCode, mods: KeyModifiers, editing: bool) -> Option<TuiAction> {
    map_key(code, mods, editing)
}

#[test]
fn space_and_enter_activate_when_not_editing() {
    assert_eq!(
        m(KeyCode::Char(' '), KeyModifiers::NONE, false),
        Some(TuiAction::Activate)
    );
    assert_eq!(m(KeyCode::Enter, KeyModifiers::NONE, false), Some(TuiAction::Activate));
}

#[test]
fn shift_enter_and_shift_space_expand() {
    assert_eq!(m(KeyCode::Enter, KeyModifiers::SHIFT, false), Some(TuiAction::Expand));
    assert_eq!(
        m(KeyCode::Char(' '), KeyModifiers::SHIFT, false),
        Some(TuiAction::Expand)
    );
}

#[test]
fn tab_expands_when_not_editing() {
    // Tab is the universal expand/collapse toggle (works in every terminal,
    // unlike Shift+Enter/Shift+Space which need the keyboard-enhancement protocol).
    assert_eq!(m(KeyCode::Tab, KeyModifiers::NONE, false), Some(TuiAction::Expand));
}

#[test]
fn tab_is_ignored_while_editing() {
    // Inside the inline text editor Tab does nothing (no field navigation here).
    assert_eq!(m(KeyCode::Tab, KeyModifiers::NONE, true), None);
}

#[test]
fn arrows_cycle_and_reorder() {
    assert_eq!(m(KeyCode::Left, KeyModifiers::NONE, false), Some(TuiAction::CycleLeft));
    assert_eq!(
        m(KeyCode::Right, KeyModifiers::NONE, false),
        Some(TuiAction::CycleRight)
    );
    assert_eq!(m(KeyCode::Up, KeyModifiers::SHIFT, false), Some(TuiAction::MoveUp));
    assert_eq!(m(KeyCode::Down, KeyModifiers::SHIFT, false), Some(TuiAction::MoveDown));
    assert_eq!(m(KeyCode::Up, KeyModifiers::NONE, false), Some(TuiAction::Up));
    assert_eq!(m(KeyCode::Down, KeyModifiers::NONE, false), Some(TuiAction::Down));
}

#[test]
fn editing_routes_text_keys_and_ctrl_c_aborts() {
    assert_eq!(
        m(KeyCode::Char('x'), KeyModifiers::NONE, true),
        Some(TuiAction::EditChar('x'))
    );
    assert_eq!(
        m(KeyCode::Backspace, KeyModifiers::NONE, true),
        Some(TuiAction::EditBackspace)
    );
    assert_eq!(m(KeyCode::Enter, KeyModifiers::NONE, true), Some(TuiAction::EditCommit));
    assert_eq!(m(KeyCode::Esc, KeyModifiers::NONE, true), Some(TuiAction::EditAbort));
    // Ctrl-C while editing aborts the edit, does NOT insert a control char.
    assert_eq!(
        m(KeyCode::Char('c'), KeyModifiers::CONTROL, true),
        Some(TuiAction::EditAbort)
    );
}

#[test]
fn ctrl_c_cancels_when_not_editing() {
    assert_eq!(
        m(KeyCode::Char('c'), KeyModifiers::CONTROL, false),
        Some(TuiAction::Cancel)
    );
}

#[test]
fn esc_cancels_when_not_editing() {
    assert_eq!(m(KeyCode::Esc, KeyModifiers::NONE, false), Some(TuiAction::Cancel));
}
