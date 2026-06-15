use super::*;
use crate::config::{CentralSettings, Config, Defaults, ProxyEntry, ProxySettings, ResolvedProxy};
use crate::proxy::headroom::HeadroomSettings;
use crate::proxy::pino::{CacheTtl, PinoSettings};
use crate::proxy::ProxyName;
use focus::Focus;
use settings::SettingId;

/// Build the canonical config (pino, headroom, central — all disabled) used to
/// seed reducer states.
fn cfg_all_disabled() -> Config {
    Config {
        version: 1,
        proxies: vec![
            ProxyEntry {
                name: ProxyName::Pino,
                enabled: false,
                settings: ProxySettings::Pino(PinoSettings {
                    auto_cache: true,
                    main_ttl: CacheTtl::OneHour,
                    sub_ttl: CacheTtl::FiveMin,
                    drop_tools: vec![],
                    strip_ansi: true,
                    model_override: None,
                }),
            },
            ProxyEntry {
                name: ProxyName::Headroom,
                enabled: false,
                settings: ProxySettings::Headroom(HeadroomSettings { compression: true }),
            },
            ProxyEntry {
                name: ProxyName::Central,
                enabled: false,
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

/// Seed a state with pino as the sole resolved (enabled) member, everything else
/// disabled in config order — the common starting point for these tests.
fn seeded() -> TuiState {
    let cfg = cfg_all_disabled();
    let resolved = vec![ResolvedProxy {
        name: ProxyName::Pino,
        settings: cfg.proxies[0].settings.clone(),
    }];
    TuiState::from_config_and_resolved(&cfg, &resolved)
}

/// A fully-disabled seed (empty resolved chain), preserving config order.
fn seeded_empty() -> TuiState {
    TuiState::from_config_and_resolved(&cfg_all_disabled(), &[])
}

/// Render the value of whatever setting the cursor is on; panics if not on a
/// setting.
fn focused_value(st: &TuiState) -> String {
    match st.focus() {
        Focus::Setting(name, sid) => settings::render_value(st.settings_of_proxy(name), sid),
        f => panic!("not on a setting: {f:?}"),
    }
}

// Activation semantics ============================================================

#[test]
fn enter_and_space_both_activate_not_run() {
    // Both Enter and Space map to Activate in the keymap; on a proxy header that
    // toggles enabled and never runs.
    let mut st = seeded();
    assert!(matches!(st.focus(), Focus::Proxy(ProxyName::Pino)));
    assert_eq!(st.apply(TuiAction::Activate), TuiOutcome::Continue);
    // pino was enabled (seeded), so the toggle disabled it; still Continue.
    let pino = st.settings_of_proxy(ProxyName::Pino);
    let _ = pino; // value unused; the point is no Run/Cancel happened.
}

#[test]
fn activate_on_proxy_toggles_enabled_does_not_run() {
    let mut st = seeded_empty(); // pino disabled, focus on pino
    assert!(matches!(st.focus(), Focus::Proxy(ProxyName::Pino)));
    assert_eq!(st.apply(TuiAction::Activate), TuiOutcome::Continue);
    // Confirm via the Run outcome that pino is now enabled.
    st.set_focus(Focus::Start);
    let TuiOutcome::Run(entries) = st.apply(TuiAction::Activate) else {
        panic!("expected Run")
    };
    assert!(entries
        .iter()
        .any(|e| e.name == ProxyName::Pino && e.enabled));
}

#[test]
fn only_start_button_runs() {
    let mut st = seeded();
    st.set_focus(Focus::Start);
    assert!(matches!(st.apply(TuiAction::Activate), TuiOutcome::Run(_)));
}

#[test]
fn cancel_paths() {
    let mut a = seeded();
    a.set_focus(Focus::Cancel);
    assert_eq!(a.apply(TuiAction::Activate), TuiOutcome::Cancel);
    let mut b = seeded();
    assert_eq!(b.apply(TuiAction::Cancel), TuiOutcome::Cancel);
}

// Navigation ======================================================================

#[test]
fn up_down_step_focus_and_clamp() {
    let mut st = seeded_empty(); // focus on pino (first)
    assert_eq!(st.apply(TuiAction::Up), TuiOutcome::Continue);
    assert!(matches!(st.focus(), Focus::Proxy(ProxyName::Pino))); // clamps at head
    assert_eq!(st.apply(TuiAction::Down), TuiOutcome::Continue);
    assert!(matches!(st.focus(), Focus::Proxy(ProxyName::Headroom)));
    // Walk to the tail (Cancel) and confirm it clamps there.
    while st.focus() != Focus::Cancel {
        st.apply(TuiAction::Down);
    }
    st.apply(TuiAction::Down);
    assert_eq!(st.focus(), Focus::Cancel);
}

// Expand / edit ===================================================================

#[test]
fn expand_reveals_and_edit_text_commits() {
    let mut st = seeded();
    st.apply(TuiAction::Expand);
    assert!(st
        .visible()
        .contains(&Focus::Setting(ProxyName::Pino, SettingId::AutoCache)));
    st.set_focus(Focus::Setting(ProxyName::Pino, SettingId::ModelOverride));
    st.apply(TuiAction::Activate);
    assert!(st.is_editing());
    for c in "opus".chars() {
        st.apply(TuiAction::EditChar(c));
    }
    st.apply(TuiAction::EditCommit);
    assert!(!st.is_editing());
    assert_eq!(focused_value(&st), "opus");
}

#[test]
fn edit_abort_discards_buffer() {
    let mut st = seeded();
    st.apply(TuiAction::Expand);
    st.set_focus(Focus::Setting(ProxyName::Pino, SettingId::ModelOverride));
    st.apply(TuiAction::Activate);
    for c in "junk".chars() {
        st.apply(TuiAction::EditChar(c));
    }
    st.apply(TuiAction::EditAbort);
    assert!(!st.is_editing());
    assert_eq!(focused_value(&st), "(default)"); // unchanged from seed
}

#[test]
fn edit_backspace_removes_last_char() {
    let mut st = seeded();
    st.apply(TuiAction::Expand);
    st.set_focus(Focus::Setting(ProxyName::Pino, SettingId::ModelOverride));
    st.apply(TuiAction::Activate);
    for c in "abc".chars() {
        st.apply(TuiAction::EditChar(c));
    }
    st.apply(TuiAction::EditBackspace);
    st.apply(TuiAction::EditCommit);
    assert_eq!(focused_value(&st), "ab");
}

#[test]
fn edit_commit_invalid_number_keeps_editing_and_sets_hint() {
    let mut st = seeded_empty();
    // Expand central and focus its Port (Number) setting.
    st.set_focus(Focus::Proxy(ProxyName::Central));
    st.apply(TuiAction::Expand);
    st.set_focus(Focus::Setting(ProxyName::Central, SettingId::Port));
    st.apply(TuiAction::Activate);
    assert!(st.is_editing());
    for c in "abc".chars() {
        st.apply(TuiAction::EditChar(c));
    }
    assert_eq!(st.apply(TuiAction::EditCommit), TuiOutcome::Continue);
    assert!(st.is_editing(), "invalid commit keeps editing");
    assert!(st.hint().is_some(), "parse error surfaced as hint");
    // A valid value then commits and clears the editor + hint.
    st.apply(TuiAction::EditBackspace);
    st.apply(TuiAction::EditBackspace);
    st.apply(TuiAction::EditBackspace);
    for c in "9000".chars() {
        st.apply(TuiAction::EditChar(c));
    }
    st.apply(TuiAction::EditCommit);
    assert!(!st.is_editing());
    assert_eq!(st.hint(), None);
    assert_eq!(focused_value(&st), "9000");
}

#[test]
fn editing_routes_only_edit_actions() {
    // While editing, navigation/activation actions are inert (return Continue and
    // do not change focus or leave the editor).
    let mut st = seeded();
    st.apply(TuiAction::Expand);
    st.set_focus(Focus::Setting(ProxyName::Pino, SettingId::ModelOverride));
    st.apply(TuiAction::Activate);
    let focus_before = st.focus();
    assert_eq!(st.apply(TuiAction::Down), TuiOutcome::Continue);
    assert!(st.is_editing());
    assert_eq!(st.focus(), focus_before);
    assert_eq!(st.apply(TuiAction::Activate), TuiOutcome::Continue);
    assert!(st.is_editing());
}

// Bool / enum activation + cycle ==================================================

#[test]
fn activate_on_bool_toggles_in_place() {
    let mut st = seeded();
    st.apply(TuiAction::Expand);
    st.set_focus(Focus::Setting(ProxyName::Pino, SettingId::AutoCache));
    assert_eq!(focused_value(&st), "[x]"); // seeded auto_cache: true
    st.apply(TuiAction::Activate);
    assert!(!st.is_editing(), "bool activation does not enter edit mode");
    assert_eq!(focused_value(&st), "[ ]");
}

#[test]
fn cycle_right_on_enum_changes_value() {
    let mut st = seeded();
    st.apply(TuiAction::Expand);
    st.set_focus(Focus::Setting(ProxyName::Pino, SettingId::SubTtl));
    assert_eq!(focused_value(&st), "‹ 5m ›"); // seeded sub_ttl: 5m
    st.apply(TuiAction::CycleRight);
    assert_eq!(focused_value(&st), "‹ 1h ›");
    st.apply(TuiAction::CycleLeft);
    assert_eq!(focused_value(&st), "‹ 5m ›");
}

#[test]
fn cycle_on_bool_toggles() {
    let mut st = seeded();
    st.apply(TuiAction::Expand);
    st.set_focus(Focus::Setting(ProxyName::Pino, SettingId::AutoCache));
    assert_eq!(focused_value(&st), "[x]");
    st.apply(TuiAction::CycleRight);
    assert_eq!(focused_value(&st), "[ ]");
}

#[test]
fn expand_on_setting_collapses_back_to_proxy() {
    let mut st = seeded();
    st.apply(TuiAction::Expand);
    st.set_focus(Focus::Setting(ProxyName::Pino, SettingId::SubTtl));
    st.apply(TuiAction::Expand); // collapse from a setting
    assert_eq!(st.focus(), Focus::Proxy(ProxyName::Pino));
    assert!(!st
        .visible()
        .contains(&Focus::Setting(ProxyName::Pino, SettingId::SubTtl)));
}

// Reorder =========================================================================

#[test]
fn move_only_acts_on_proxy_headers() {
    // On a setting, MoveDown/MoveUp are no-ops (focus unchanged, no reorder).
    let mut st = seeded();
    st.apply(TuiAction::Expand);
    st.set_focus(Focus::Setting(ProxyName::Pino, SettingId::SubTtl));
    let before = st.rows_order();
    assert_eq!(st.apply(TuiAction::MoveDown), TuiOutcome::Continue);
    assert_eq!(st.rows_order(), before);
    assert_eq!(
        st.focus(),
        Focus::Setting(ProxyName::Pino, SettingId::SubTtl)
    );
}

#[test]
fn move_down_reorders_proxy_and_keeps_focus() {
    let mut st = seeded_empty(); // [pino, headroom, central], focus on pino
    assert_eq!(st.apply(TuiAction::MoveDown), TuiOutcome::Continue);
    assert_eq!(
        st.rows_order(),
        vec![ProxyName::Headroom, ProxyName::Pino, ProxyName::Central]
    );
    assert_eq!(st.focus(), Focus::Proxy(ProxyName::Pino)); // focus follows by name
    assert_eq!(st.hint(), None);
}

#[test]
fn cannot_move_a_proxy_below_central_sets_hint() {
    let mut st = seeded_empty();
    st.set_focus(Focus::Proxy(ProxyName::Headroom));
    assert_eq!(st.apply(TuiAction::MoveDown), TuiOutcome::Continue);
    // Order unchanged; central stays last.
    assert_eq!(
        st.rows_order(),
        vec![ProxyName::Pino, ProxyName::Headroom, ProxyName::Central]
    );
    assert_eq!(st.focus(), Focus::Proxy(ProxyName::Headroom));
    assert_eq!(st.hint(), Some("central must stay last"));
}

#[test]
fn central_cannot_be_moved_up_sets_hint() {
    let mut st = seeded_empty();
    st.set_focus(Focus::Proxy(ProxyName::Central));
    assert_eq!(st.apply(TuiAction::MoveUp), TuiOutcome::Continue);
    assert_eq!(
        st.rows_order(),
        vec![ProxyName::Pino, ProxyName::Headroom, ProxyName::Central]
    );
    assert_eq!(st.focus(), Focus::Proxy(ProxyName::Central));
    assert_eq!(st.hint(), Some("central must stay last"));
}

#[test]
fn name_based_focus_survives_reorder() {
    // After moving pino down, the focus is still on pino by NAME even though its
    // index changed — the regression this redesign fixes.
    let mut st = seeded_empty();
    st.set_focus(Focus::Proxy(ProxyName::Pino));
    st.apply(TuiAction::MoveDown);
    assert_eq!(st.focus(), Focus::Proxy(ProxyName::Pino));
    assert_eq!(st.rows_order()[1], ProxyName::Pino); // pino is now at index 1
}

#[test]
fn reorder_then_clear_hint_on_legal_move() {
    let mut st = seeded_empty();
    // Provoke a hint.
    st.set_focus(Focus::Proxy(ProxyName::Headroom));
    st.apply(TuiAction::MoveDown);
    assert_eq!(st.hint(), Some("central must stay last"));
    // A legal move clears it.
    st.set_focus(Focus::Proxy(ProxyName::Pino));
    st.apply(TuiAction::MoveDown);
    assert_eq!(st.hint(), None);
}

// Run outcome / central-last ======================================================

#[test]
fn run_outcome_full_state_central_last() {
    let mut st = seeded();
    st.set_focus(Focus::Start);
    let TuiOutcome::Run(entries) = st.apply(TuiAction::Activate) else {
        panic!()
    };
    assert_eq!(entries.len(), 3);
    assert_eq!(entries.last().unwrap().name, ProxyName::Central);
    assert!(entries
        .iter()
        .any(|e| e.name == ProxyName::Pino && e.enabled));
}

#[test]
fn freshly_seeded_to_entries_is_central_last_with_zero_actions() {
    // Ported from `from_resolved_central_forced_last_even_if_not_tail_of_resolved`:
    // central is forced last ON CONSTRUCTION, so a zero-action Start yields a
    // central-last entry list even when central appears mid-chain in the seed.
    let cfg = cfg_all_disabled();
    let resolved = vec![
        ResolvedProxy {
            name: ProxyName::Pino,
            settings: cfg.proxies[0].settings.clone(),
        },
        ResolvedProxy {
            name: ProxyName::Central,
            settings: ProxySettings::Central(CentralSettings {
                port: None,
                pinned_version: None,
            }),
        },
    ];
    let mut st = TuiState::from_config_and_resolved(&cfg, &resolved);
    st.set_focus(Focus::Start);
    let TuiOutcome::Run(entries) = st.apply(TuiAction::Activate) else {
        panic!()
    };
    assert_eq!(entries.last().unwrap().name, ProxyName::Central);
    assert!(entries.last().unwrap().enabled);
}

#[test]
fn run_outcome_carries_per_setting_edits() {
    // Edit pino's model_override, then Start: the entry list carries the edit.
    let mut st = seeded();
    st.apply(TuiAction::Expand);
    st.set_focus(Focus::Setting(ProxyName::Pino, SettingId::ModelOverride));
    st.apply(TuiAction::Activate);
    for c in "claude-x".chars() {
        st.apply(TuiAction::EditChar(c));
    }
    st.apply(TuiAction::EditCommit);
    st.set_focus(Focus::Start);
    let TuiOutcome::Run(entries) = st.apply(TuiAction::Activate) else {
        panic!()
    };
    let pino = entries.iter().find(|e| e.name == ProxyName::Pino).unwrap();
    match &pino.settings {
        ProxySettings::Pino(p) => assert_eq!(p.model_override.as_deref(), Some("claude-x")),
        other => panic!("expected pino settings, got {other:?}"),
    }
}

#[test]
fn central_stays_last_under_adversarial_sequence() {
    let mut st = seeded_empty();
    let script = [
        TuiAction::Down,
        TuiAction::Activate,
        TuiAction::MoveDown,
        TuiAction::Down,
        TuiAction::Activate,
        TuiAction::MoveUp,
        TuiAction::Up,
        TuiAction::MoveDown,
    ];
    for a in script {
        assert_eq!(st.apply(a), TuiOutcome::Continue);
        assert_eq!(*st.rows_order().last().unwrap(), ProxyName::Central);
    }
}

// chain_preview ===================================================================

#[test]
fn from_resolved_enables_and_orders_members_first() {
    let cfg = cfg_all_disabled();
    let resolved = vec![
        ResolvedProxy {
            name: ProxyName::Headroom,
            settings: ProxySettings::Headroom(HeadroomSettings { compression: true }),
        },
        ResolvedProxy {
            name: ProxyName::Pino,
            settings: cfg.proxies[0].settings.clone(),
        },
    ];
    let st = TuiState::from_config_and_resolved(&cfg, &resolved);
    assert_eq!(
        st.rows_order(),
        vec![ProxyName::Headroom, ProxyName::Pino, ProxyName::Central]
    );
    assert_eq!(st.focus(), Focus::Proxy(ProxyName::Headroom));
    assert_eq!(st.hint(), None);
}

#[test]
fn chain_preview_lists_enabled_in_order() {
    let mut st = seeded_empty();
    st.set_focus(Focus::Proxy(ProxyName::Pino));
    st.apply(TuiAction::Activate); // pino on
    st.set_focus(Focus::Proxy(ProxyName::Headroom));
    st.apply(TuiAction::Activate); // headroom on
    assert_eq!(
        st.chain_preview(),
        "claude → pino → headroom → api.anthropic.com"
    );
}

#[test]
fn chain_preview_empty_when_none_enabled() {
    let st = seeded_empty();
    assert_eq!(st.chain_preview(), "claude → api.anthropic.com");
}

// No-op focus targets =============================================================

#[test]
fn cycle_and_expand_on_buttons_are_noops() {
    // On the action buttons, Cycle and Expand do nothing (no panic, no reorder,
    // no focus change).
    let mut st = seeded_empty();
    for button in [Focus::Start, Focus::Cancel] {
        st.set_focus(button);
        let before = st.rows_order();
        assert_eq!(st.apply(TuiAction::CycleLeft), TuiOutcome::Continue);
        assert_eq!(st.apply(TuiAction::CycleRight), TuiOutcome::Continue);
        assert_eq!(st.apply(TuiAction::Expand), TuiOutcome::Continue);
        assert_eq!(st.focus(), button);
        assert_eq!(st.rows_order(), before);
    }
}

#[test]
fn cycle_on_text_setting_clears_stale_hint() {
    // Cycling a non-cyclable (Text) setting is a no-op on the value but still
    // clears any stale reject hint, like cycling a bool/enum does.
    let mut st = seeded_empty();
    // Provoke a hint via an illegal reorder.
    st.set_focus(Focus::Proxy(ProxyName::Central));
    st.apply(TuiAction::MoveUp);
    assert_eq!(st.hint(), Some("central must stay last"));
    // Now cycle a Text setting (model_override): value unchanged, hint cleared.
    st.set_focus(Focus::Proxy(ProxyName::Pino));
    st.apply(TuiAction::Expand);
    st.set_focus(Focus::Setting(ProxyName::Pino, SettingId::ModelOverride));
    assert_eq!(focused_value(&st), "(default)");
    st.apply(TuiAction::CycleRight);
    assert_eq!(focused_value(&st), "(default)"); // no change
    assert_eq!(st.hint(), None); // hint cleared
}
