use super::*;
use crate::config::{CentralSettings, ProxySettings};
use crate::proxy::headroom::HeadroomSettings;
use crate::proxy::pino::PinoSettings;
use crate::proxy::ProxyName;

/// Build the three-proxy seed used across the reducer tests: pino, headroom,
/// central — in that file order, all disabled by default.
fn seed_all_disabled() -> TuiState {
    TuiState::new(vec![
        (
            TuiItem {
                name: ProxyName::Pino,
                enabled: false,
            },
            ProxySettings::Pino(PinoSettings {
                auto_cache: true,
                tail_ttl: crate::proxy::pino::TailTtl::FiveMin,
                drop_tools: vec![],
                strip_ansi: true,
                model_override: None,
            }),
        ),
        (
            TuiItem {
                name: ProxyName::Headroom,
                enabled: false,
            },
            ProxySettings::Headroom(HeadroomSettings { compression: false }),
        ),
        (
            TuiItem {
                name: ProxyName::Central,
                enabled: false,
            },
            ProxySettings::Central(CentralSettings {
                port: None,
                pinned_version: None,
            }),
        ),
    ])
}

#[test]
fn new_seeds_items_in_order_with_cursor_at_top() {
    let st = seed_all_disabled();
    assert_eq!(st.items.len(), 3);
    assert_eq!(st.items[0].name, ProxyName::Pino);
    assert_eq!(st.items[1].name, ProxyName::Headroom);
    assert_eq!(st.items[2].name, ProxyName::Central);
    assert!(st.items.iter().all(|i| !i.enabled));
    assert_eq!(st.cursor, 0);
    // No hint until a constraint is violated.
    assert_eq!(st.hint(), None);
}

#[test]
fn down_moves_cursor_and_clamps_at_bottom() {
    let mut st = seed_all_disabled(); // 3 rows, cursor 0
    assert_eq!(st.apply(TuiAction::Down), TuiOutcome::Continue);
    assert_eq!(st.cursor, 1);
    assert_eq!(st.apply(TuiAction::Down), TuiOutcome::Continue);
    assert_eq!(st.cursor, 2);
    // Already at the last row: clamp, do not wrap.
    assert_eq!(st.apply(TuiAction::Down), TuiOutcome::Continue);
    assert_eq!(st.cursor, 2);
}

#[test]
fn up_moves_cursor_and_clamps_at_top() {
    let mut st = seed_all_disabled();
    st.cursor = 2;
    assert_eq!(st.apply(TuiAction::Up), TuiOutcome::Continue);
    assert_eq!(st.cursor, 1);
    assert_eq!(st.apply(TuiAction::Up), TuiOutcome::Continue);
    assert_eq!(st.cursor, 0);
    // Already at the top: clamp, do not wrap.
    assert_eq!(st.apply(TuiAction::Up), TuiOutcome::Continue);
    assert_eq!(st.cursor, 0);
}

#[test]
fn cursor_movement_never_changes_order_or_selection() {
    let mut st = seed_all_disabled();
    let before: Vec<_> = st.items.clone();
    st.apply(TuiAction::Down);
    st.apply(TuiAction::Down);
    st.apply(TuiAction::Up);
    assert_eq!(st.items, before);
}

#[test]
fn toggle_flips_enabled_at_cursor() {
    let mut st = seed_all_disabled(); // cursor 0 -> pino
    assert!(!st.items[0].enabled);
    assert_eq!(st.apply(TuiAction::Toggle), TuiOutcome::Continue);
    assert!(st.items[0].enabled);
    // Toggle again disables it.
    assert_eq!(st.apply(TuiAction::Toggle), TuiOutcome::Continue);
    assert!(!st.items[0].enabled);
}

#[test]
fn toggle_only_affects_row_under_cursor() {
    let mut st = seed_all_disabled();
    st.cursor = 1; // headroom
    st.apply(TuiAction::Toggle);
    assert!(!st.items[0].enabled); // pino untouched
    assert!(st.items[1].enabled); // headroom flipped
    assert!(!st.items[2].enabled); // central untouched
}

#[test]
fn toggle_does_not_move_cursor() {
    let mut st = seed_all_disabled();
    st.cursor = 1;
    st.apply(TuiAction::Toggle);
    assert_eq!(st.cursor, 1);
}

#[test]
fn toggle_central_in_place_when_already_last() {
    let mut st = seed_all_disabled();
    st.cursor = 2; // central, already last
    st.apply(TuiAction::Toggle);
    assert!(st.items[2].enabled);
    assert_eq!(st.items[2].name, ProxyName::Central);
    assert_eq!(st.cursor, 2);
}

#[test]
fn move_down_swaps_with_next_and_follows_cursor() {
    let mut st = seed_all_disabled(); // [pino, headroom, central], cursor 0
    assert_eq!(st.apply(TuiAction::MoveDown), TuiOutcome::Continue);
    assert_eq!(st.items[0].name, ProxyName::Headroom);
    assert_eq!(st.items[1].name, ProxyName::Pino);
    assert_eq!(st.items[2].name, ProxyName::Central);
    // Cursor follows the moved (pino) row to index 1.
    assert_eq!(st.cursor, 1);
}

#[test]
fn move_up_swaps_with_prev_and_follows_cursor() {
    let mut st = seed_all_disabled();
    st.cursor = 1; // headroom
    assert_eq!(st.apply(TuiAction::MoveUp), TuiOutcome::Continue);
    assert_eq!(st.items[0].name, ProxyName::Headroom);
    assert_eq!(st.items[1].name, ProxyName::Pino);
    assert_eq!(st.cursor, 0);
}

#[test]
fn move_up_at_top_is_noop() {
    let mut st = seed_all_disabled();
    let before: Vec<_> = st.items.clone();
    st.cursor = 0;
    assert_eq!(st.apply(TuiAction::MoveUp), TuiOutcome::Continue);
    assert_eq!(st.items, before);
    assert_eq!(st.cursor, 0);
}

#[test]
fn move_down_at_bottom_is_noop() {
    // Seed without central so the bottom row is freely "moveable" yet clamps.
    let mut st = TuiState::new(vec![
        (
            TuiItem {
                name: ProxyName::Pino,
                enabled: false,
            },
            ProxySettings::Pino(PinoSettings {
                auto_cache: true,
                tail_ttl: crate::proxy::pino::TailTtl::FiveMin,
                drop_tools: vec![],
                strip_ansi: true,
                model_override: None,
            }),
        ),
        (
            TuiItem {
                name: ProxyName::Headroom,
                enabled: false,
            },
            ProxySettings::Headroom(HeadroomSettings { compression: false }),
        ),
    ]);
    let before: Vec<_> = st.items.clone();
    st.cursor = 1; // last row
    assert_eq!(st.apply(TuiAction::MoveDown), TuiOutcome::Continue);
    assert_eq!(st.items, before);
    assert_eq!(st.cursor, 1);
}

#[test]
fn reorder_carries_settings_with_the_row() {
    // pino has a distinctive setting (model_override) we can recognize after a move.
    let mut st = TuiState::new(vec![
        (
            TuiItem {
                name: ProxyName::Pino,
                enabled: true,
            },
            ProxySettings::Pino(PinoSettings {
                auto_cache: true,
                tail_ttl: crate::proxy::pino::TailTtl::FiveMin,
                drop_tools: vec![],
                strip_ansi: true,
                model_override: Some("sonnet-test".to_string()),
            }),
        ),
        (
            TuiItem {
                name: ProxyName::Headroom,
                enabled: false,
            },
            ProxySettings::Headroom(HeadroomSettings { compression: false }),
        ),
    ]);
    st.cursor = 0;
    st.apply(TuiAction::MoveDown); // pino -> index 1
    assert_eq!(st.items[1].name, ProxyName::Pino);
    // Its settings moved with it: settings_at(1) is pino's distinctive override.
    match st.settings_at(1) {
        ProxySettings::Pino(p) => {
            assert_eq!(p.model_override.as_deref(), Some("sonnet-test"));
        }
        other => panic!("expected pino settings at index 1, got {other:?}"),
    }
    // And index 0 now holds headroom's settings.
    assert!(matches!(st.settings_at(0), ProxySettings::Headroom(_)));
}

#[test]
fn cannot_move_a_row_below_central() {
    // [pino, headroom, central]; cursor on headroom (index 1).
    let mut st = seed_all_disabled();
    st.cursor = 1;
    // MoveDown would put headroom at index 2 and central at index 1 -> illegal.
    assert_eq!(st.apply(TuiAction::MoveDown), TuiOutcome::Continue);
    // Order is unchanged; central stays last.
    assert_eq!(st.items[0].name, ProxyName::Pino);
    assert_eq!(st.items[1].name, ProxyName::Headroom);
    assert_eq!(st.items[2].name, ProxyName::Central);
    // Cursor stays on headroom (the row the user tried to move).
    assert_eq!(st.cursor, 1);
    // The rejection is surfaced as a hint.
    assert_eq!(st.hint(), Some("central must stay last"));
}

#[test]
fn central_cannot_be_moved_up() {
    let mut st = seed_all_disabled();
    st.cursor = 2; // central
    assert_eq!(st.apply(TuiAction::MoveUp), TuiOutcome::Continue);
    // Central re-coerced last; order unchanged.
    assert_eq!(st.items[2].name, ProxyName::Central);
    assert_eq!(st.items[0].name, ProxyName::Pino);
    assert_eq!(st.items[1].name, ProxyName::Headroom);
    // Cursor remains on central (still index 2).
    assert_eq!(st.cursor, 2);
    assert_eq!(st.hint(), Some("central must stay last"));
}

#[test]
fn central_stays_last_after_unrelated_reorder_and_clears_hint() {
    // Moving pino down past headroom is fine; central must remain last.
    let mut st = seed_all_disabled();
    // First provoke a hint so we can prove it clears.
    st.cursor = 1;
    st.apply(TuiAction::MoveDown); // rejected -> hint set
    assert_eq!(st.hint(), Some("central must stay last"));
    // Now an unconstrained move clears it.
    st.cursor = 0; // pino
    st.apply(TuiAction::MoveDown); // pino <-> headroom (legal)
    assert_eq!(st.items[0].name, ProxyName::Headroom);
    assert_eq!(st.items[1].name, ProxyName::Pino);
    assert_eq!(st.items[2].name, ProxyName::Central);
    assert_eq!(st.cursor, 1);
    assert_eq!(st.hint(), None);
}

#[test]
fn toggling_central_keeps_it_last_and_clears_hint() {
    let mut st = seed_all_disabled();
    // Provoke a hint.
    st.cursor = 2;
    st.apply(TuiAction::MoveUp); // rejected -> hint set
    assert_eq!(st.hint(), Some("central must stay last"));
    // Toggling central keeps it last and clears the hint.
    st.cursor = 2;
    st.apply(TuiAction::Toggle); // enable central
    assert!(st.items[2].enabled);
    assert_eq!(st.items[2].name, ProxyName::Central);
    assert_eq!(st.hint(), None);
    st.apply(TuiAction::Toggle); // disable central
    assert!(!st.items[2].enabled);
    assert_eq!(st.items[2].name, ProxyName::Central);
}

#[test]
fn no_central_present_reorder_is_unconstrained() {
    // Seed without central: pino, headroom only.
    let mut st = TuiState::new(vec![
        (
            TuiItem {
                name: ProxyName::Pino,
                enabled: true,
            },
            ProxySettings::Pino(PinoSettings {
                auto_cache: true,
                tail_ttl: crate::proxy::pino::TailTtl::FiveMin,
                drop_tools: vec![],
                strip_ansi: true,
                model_override: None,
            }),
        ),
        (
            TuiItem {
                name: ProxyName::Headroom,
                enabled: true,
            },
            ProxySettings::Headroom(HeadroomSettings { compression: false }),
        ),
    ]);
    st.cursor = 0;
    st.apply(TuiAction::MoveDown); // free to swap; no central to constrain
    assert_eq!(st.items[0].name, ProxyName::Headroom);
    assert_eq!(st.items[1].name, ProxyName::Pino);
    assert_eq!(st.cursor, 1);
    assert_eq!(st.hint(), None);
}
