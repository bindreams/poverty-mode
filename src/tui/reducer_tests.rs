use super::*;
use crate::config::{CentralSettings, Config, Defaults, ProxyEntry, ProxySettings};
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
                main_ttl: crate::proxy::pino::CacheTtl::OneHour,
                sub_ttl: crate::proxy::pino::CacheTtl::FiveMin,
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
                main_ttl: crate::proxy::pino::CacheTtl::OneHour,
                sub_ttl: crate::proxy::pino::CacheTtl::FiveMin,
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
                main_ttl: crate::proxy::pino::CacheTtl::OneHour,
                sub_ttl: crate::proxy::pino::CacheTtl::FiveMin,
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
                main_ttl: crate::proxy::pino::CacheTtl::OneHour,
                sub_ttl: crate::proxy::pino::CacheTtl::FiveMin,
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

#[test]
fn confirm_resolves_enabled_rows_in_order() {
    let mut st = seed_all_disabled();
    // Enable pino (0) and central (2); leave headroom (1) disabled.
    st.cursor = 0;
    st.apply(TuiAction::Toggle);
    st.cursor = 2;
    st.apply(TuiAction::Toggle);
    let outcome = st.apply(TuiAction::Confirm);
    match outcome {
        TuiOutcome::Run(resolved) => {
            assert_eq!(resolved.len(), 2);
            assert_eq!(resolved[0].name, ProxyName::Pino);
            assert_eq!(resolved[1].name, ProxyName::Central);
            assert!(matches!(resolved[0].settings, ProxySettings::Pino(_)));
            assert!(matches!(resolved[1].settings, ProxySettings::Central(_)));
        }
        other => panic!("expected Run, got {other:?}"),
    }
}

#[test]
fn confirm_with_nothing_enabled_runs_empty_chain() {
    let mut st = seed_all_disabled();
    match st.apply(TuiAction::Confirm) {
        TuiOutcome::Run(resolved) => assert!(resolved.is_empty()),
        other => panic!("expected Run(empty), got {other:?}"),
    }
}

#[test]
fn confirm_respects_reordered_enabled_rows() {
    let mut st = seed_all_disabled();
    // Enable pino and headroom, then move headroom above pino.
    st.cursor = 0;
    st.apply(TuiAction::Toggle); // pino on
    st.cursor = 1;
    st.apply(TuiAction::Toggle); // headroom on
    st.apply(TuiAction::MoveUp); // headroom -> index 0, pino -> index 1
    match st.apply(TuiAction::Confirm) {
        TuiOutcome::Run(resolved) => {
            assert_eq!(resolved.len(), 2);
            assert_eq!(resolved[0].name, ProxyName::Headroom);
            assert_eq!(resolved[1].name, ProxyName::Pino);
        }
        other => panic!("expected Run, got {other:?}"),
    }
}

#[test]
fn confirm_carries_settings_into_resolved_chain() {
    // End-to-end settings-carry across a reorder, now that Confirm exists.
    let mut st = seed_all_disabled();
    st.cursor = 0;
    st.apply(TuiAction::Toggle); // pino enabled at index 0
    st.apply(TuiAction::MoveDown); // pino -> index 1 (headroom -> 0)
    match st.apply(TuiAction::Confirm) {
        TuiOutcome::Run(resolved) => {
            assert_eq!(resolved.len(), 1); // only pino enabled
            assert_eq!(resolved[0].name, ProxyName::Pino);
            match &resolved[0].settings {
                ProxySettings::Pino(p) => assert!(p.auto_cache),
                other => panic!("expected pino settings, got {other:?}"),
            }
        }
        other => panic!("expected Run, got {other:?}"),
    }
}

#[test]
fn chain_preview_lists_enabled_in_order() {
    let mut st = seed_all_disabled();
    st.cursor = 0;
    st.apply(TuiAction::Toggle); // pino
    st.cursor = 1;
    st.apply(TuiAction::Toggle); // headroom
    assert_eq!(
        st.chain_preview(),
        "claude → pino → headroom → api.anthropic.com"
    );
}

#[test]
fn chain_preview_empty_when_none_enabled() {
    let st = seed_all_disabled();
    assert_eq!(st.chain_preview(), "claude → api.anthropic.com");
}

#[test]
fn chain_preview_includes_central_last() {
    let mut st = seed_all_disabled();
    st.cursor = 0;
    st.apply(TuiAction::Toggle); // pino
    st.cursor = 2;
    st.apply(TuiAction::Toggle); // central (already last)
    assert_eq!(
        st.chain_preview(),
        "claude → pino → central → api.anthropic.com"
    );
}

#[test]
fn cancel_returns_cancel_outcome() {
    let mut st = seed_all_disabled();
    assert_eq!(st.apply(TuiAction::Cancel), TuiOutcome::Cancel);
}

#[test]
fn confirm_and_cancel_do_not_mutate_state() {
    let mut st = seed_all_disabled();
    st.cursor = 1;
    st.apply(TuiAction::Toggle); // headroom on
    let items_before = st.items.clone();
    let cursor_before = st.cursor;
    let hint_before = st.hint();
    let _ = st.apply(TuiAction::Confirm);
    assert_eq!(st.items, items_before);
    assert_eq!(st.cursor, cursor_before);
    assert_eq!(st.hint(), hint_before);
    let _ = st.apply(TuiAction::Cancel);
    assert_eq!(st.items, items_before);
    assert_eq!(st.cursor, cursor_before);
    assert_eq!(st.hint(), hint_before);
}

fn cfg_pino_headroom_central(pino_on: bool, central_on: bool) -> Config {
    Config {
        version: 1,
        proxies: vec![
            ProxyEntry {
                name: ProxyName::Pino,
                enabled: pino_on,
                settings: ProxySettings::Pino(PinoSettings {
                    auto_cache: true,
                    main_ttl: crate::proxy::pino::CacheTtl::OneHour,
                    sub_ttl: crate::proxy::pino::CacheTtl::FiveMin,
                    drop_tools: vec![],
                    strip_ansi: true,
                    model_override: None,
                }),
            },
            ProxyEntry {
                name: ProxyName::Headroom,
                enabled: false,
                settings: ProxySettings::Headroom(HeadroomSettings { compression: false }),
            },
            ProxyEntry {
                name: ProxyName::Central,
                enabled: central_on,
                settings: ProxySettings::Central(CentralSettings {
                    port: None,
                    pinned_version: None,
                }),
            },
        ],
        defaults: Defaults {
            enable_tool_search: true,
        },
    }
}

// --- from_config_and_resolved: TUI seeded from the RESOLVED chain (spec §5.10) -----

#[test]
fn from_resolved_enables_and_orders_resolved_members_first() {
    // Config has all rows disabled in canonical order. The resolved chain
    // (e.g. from `--proxies headroom,pino` or POVERTY_PROXY_CHAIN) enables
    // headroom then pino, in THAT order — the TUI must reflect it, not the
    // config-file order/enabled flags.
    let cfg = cfg_pino_headroom_central(false, false);
    let resolved = vec![
        ResolvedProxy {
            name: ProxyName::Headroom,
            settings: ProxySettings::Headroom(HeadroomSettings { compression: false }),
        },
        ResolvedProxy {
            name: ProxyName::Pino,
            settings: ProxySettings::Pino(PinoSettings {
                auto_cache: true,
                main_ttl: crate::proxy::pino::CacheTtl::OneHour,
                sub_ttl: crate::proxy::pino::CacheTtl::FiveMin,
                drop_tools: vec![],
                strip_ansi: true,
                model_override: None,
            }),
        },
    ];
    let st = TuiState::from_config_and_resolved(&cfg, &resolved);
    // Resolved members come first, enabled, in chain order: headroom, pino.
    assert_eq!(st.items[0].name, ProxyName::Headroom);
    assert!(st.items[0].enabled);
    assert_eq!(st.items[1].name, ProxyName::Pino);
    assert!(st.items[1].enabled);
    // The remaining known proxy (central) follows, disabled, and is last.
    assert_eq!(st.items[2].name, ProxyName::Central);
    assert!(!st.items[2].enabled);
    assert_eq!(st.items.len(), 3);
    assert_eq!(st.cursor, 0);
    assert_eq!(st.hint(), None);
}

#[test]
fn from_resolved_keeps_all_known_rows_togglable() {
    // Even with an empty resolved chain, every known proxy is present as a
    // (disabled) row so the user can still toggle it on.
    let cfg = cfg_pino_headroom_central(false, false);
    let st = TuiState::from_config_and_resolved(&cfg, &[]);
    assert_eq!(st.items.len(), 3);
    assert!(st.items.iter().all(|i| !i.enabled));
    // Disabled-and-absent rows keep the config's relative order.
    assert_eq!(st.items[0].name, ProxyName::Pino);
    assert_eq!(st.items[1].name, ProxyName::Headroom);
    assert_eq!(st.items[2].name, ProxyName::Central);
}

#[test]
fn from_resolved_central_forced_last_even_if_not_tail_of_resolved() {
    // Central is always coerced last regardless of where it appears in inputs.
    let cfg = cfg_pino_headroom_central(false, false);
    let resolved = vec![
        ResolvedProxy {
            name: ProxyName::Pino,
            settings: ProxySettings::Pino(PinoSettings {
                auto_cache: true,
                main_ttl: crate::proxy::pino::CacheTtl::OneHour,
                sub_ttl: crate::proxy::pino::CacheTtl::FiveMin,
                drop_tools: vec![],
                strip_ansi: true,
                model_override: None,
            }),
        },
        ResolvedProxy {
            name: ProxyName::Central,
            settings: ProxySettings::Central(CentralSettings {
                port: None,
                pinned_version: None,
            }),
        },
    ];
    let st = TuiState::from_config_and_resolved(&cfg, &resolved);
    assert_eq!(st.items.last().unwrap().name, ProxyName::Central);
    assert!(st.items.last().unwrap().enabled);
}

#[test]
fn from_resolved_then_confirm_yields_the_resolved_chain() {
    // Round-trip: the chain confirmed back out matches the resolved seed
    // (enabled members, in order). This is the property the picker relies on so
    // an unmodified Enter reproduces `--proxies` / POVERTY_PROXY_CHAIN.
    let cfg = cfg_pino_headroom_central(false, false);
    let resolved = vec![
        ResolvedProxy {
            name: ProxyName::Headroom,
            settings: ProxySettings::Headroom(HeadroomSettings { compression: false }),
        },
        ResolvedProxy {
            name: ProxyName::Pino,
            settings: ProxySettings::Pino(PinoSettings {
                auto_cache: true,
                main_ttl: crate::proxy::pino::CacheTtl::OneHour,
                sub_ttl: crate::proxy::pino::CacheTtl::FiveMin,
                drop_tools: vec![],
                strip_ansi: true,
                model_override: Some("seed-model".to_string()),
            }),
        },
    ];
    let mut st = TuiState::from_config_and_resolved(&cfg, &resolved);
    match st.apply(TuiAction::Confirm) {
        TuiOutcome::Run(out) => {
            assert_eq!(
                out, resolved,
                "confirmed chain must equal the resolved seed"
            );
        }
        other => panic!("expected Run, got {other:?}"),
    }
}

// --- central-last property guards (added green-on-arrival; not red->green) -----

#[test]
fn central_remains_last_under_an_adversarial_action_sequence() {
    let mut st = seed_all_disabled();
    // A scripted barrage that repeatedly tries to displace central.
    let script = [
        TuiAction::Down,     // -> headroom
        TuiAction::Toggle,   // enable headroom
        TuiAction::MoveDown, // try to push headroom below central (illegal)
        TuiAction::Down,     // -> central
        TuiAction::Toggle,   // enable central
        TuiAction::MoveUp,   // try to lift central (illegal)
        TuiAction::Up,       // -> some row
        TuiAction::MoveDown, // push toward central
        TuiAction::Down,
        TuiAction::MoveDown,
        TuiAction::Toggle,
        TuiAction::MoveUp,
    ];
    for a in script {
        let out = st.apply(a);
        // No scripted action is Confirm/Cancel, so we always Continue.
        assert_eq!(out, TuiOutcome::Continue);
        // Invariant after EVERY step: central, if present, is the last row.
        let last = st.items.last().expect("non-empty");
        assert_eq!(last.name, ProxyName::Central, "central must stay last");
        // Cursor always in bounds.
        assert!(st.cursor < st.items.len());
    }
}

#[test]
fn confirm_after_adversarial_sequence_keeps_central_last_in_chain() {
    let mut st = seed_all_disabled();
    // Enable all three, then thrash the order.
    for idx in 0..3 {
        st.cursor = idx;
        st.apply(TuiAction::Toggle);
    }
    st.cursor = 0;
    st.apply(TuiAction::MoveDown);
    st.apply(TuiAction::MoveDown);
    st.cursor = 2;
    st.apply(TuiAction::MoveUp); // central can't rise
    match st.apply(TuiAction::Confirm) {
        TuiOutcome::Run(resolved) => {
            assert_eq!(resolved.len(), 3);
            assert_eq!(
                resolved.last().expect("non-empty").name,
                ProxyName::Central,
                "central must be last in the resolved chain"
            );
        }
        other => panic!("expected Run, got {other:?}"),
    }
}
