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
