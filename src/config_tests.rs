// The plan (R12) authors these assertions with explicit `assert_eq!(field, true)`
// /`false` forms to read each boolean config field by name; keep them verbatim and
// scope the clippy lint to this test module rather than rewrite the plan's tests.
#![allow(clippy::bool_assert_comparison)]

use super::*;
use crate::proxy::headroom::HeadroomSettings;
use crate::proxy::pino::{CacheTtl, PinoSettings};
use crate::proxy::ProxyName;

fn pino_of(s: &ProxySettings) -> &PinoSettings {
    match s {
        ProxySettings::Pino(p) => p,
        other => panic!("expected Pino settings, got {other:?}"),
    }
}
fn headroom_of(s: &ProxySettings) -> &HeadroomSettings {
    match s {
        ProxySettings::Headroom(h) => h,
        other => panic!("expected Headroom settings, got {other:?}"),
    }
}
fn central_of(s: &ProxySettings) -> &CentralSettings {
    match s {
        ProxySettings::Central(c) => c,
        other => panic!("expected Central settings, got {other:?}"),
    }
}

#[test]
fn default_all_disabled_lists_three_proxies_all_disabled_in_order() {
    let cfg = Config::default_all_disabled();
    assert_eq!(cfg.version, 1);
    assert_eq!(cfg.defaults.enable_tool_search, true);

    let names: Vec<ProxyName> = cfg.proxies.iter().map(|e| e.name).collect();
    assert_eq!(
        names,
        vec![ProxyName::Pino, ProxyName::Headroom, ProxyName::Central]
    );

    for e in &cfg.proxies {
        assert_eq!(e.enabled, false, "proxy {:?} must default disabled", e.name);
    }
}

#[test]
fn default_all_disabled_has_expected_per_proxy_settings() {
    let cfg = Config::default_all_disabled();

    let pino = pino_of(&cfg.proxies[0].settings);
    assert_eq!(pino.auto_cache, true);
    assert_eq!(pino.main_ttl, CacheTtl::OneHour);
    assert_eq!(pino.sub_ttl, CacheTtl::FiveMin);
    assert_eq!(pino.drop_tools, Vec::<String>::new());
    assert_eq!(pino.strip_ansi, true);
    assert_eq!(pino.model_override, None);

    let headroom = headroom_of(&cfg.proxies[1].settings);
    assert_eq!(headroom.compression, false);

    let central = central_of(&cfg.proxies[2].settings);
    assert_eq!(central.port, None);
    assert_eq!(central.pinned_version, None);
}

#[test]
fn default_config_round_trips_through_yaml() {
    let cfg = Config::default_all_disabled();
    let yaml = serde_yaml::to_string(&cfg).unwrap();
    let back: Config = serde_yaml::from_str(&yaml).unwrap();

    assert_eq!(back, cfg);
}

#[test]
fn yaml_uses_lowercase_proxy_names_and_main_1h_sub_5m() {
    let cfg = Config::default_all_disabled();
    let yaml = serde_yaml::to_string(&cfg).unwrap();
    assert!(yaml.contains("name: pino"), "yaml was:\n{yaml}");
    assert!(yaml.contains("name: headroom"), "yaml was:\n{yaml}");
    assert!(yaml.contains("name: central"), "yaml was:\n{yaml}");
    assert!(
        yaml.contains("main_ttl: 1h"),
        "main_ttl should serialize as 1h; yaml was:\n{yaml}"
    );
    assert!(
        yaml.contains("sub_ttl: 5m"),
        "sub_ttl should serialize as 5m; yaml was:\n{yaml}"
    );
}

#[test]
fn untagged_settings_parse_pino_not_headroom() {
    // A mapping with pino-only fields must parse as Pino, never Headroom.
    let yaml = r#"
auto_cache: false
main_ttl: 1h
sub_ttl: 5m
drop_tools: ["Foo", "Bar"]
strip_ansi: false
model_override: claude-x
"#;
    let s: ProxySettings = serde_yaml::from_str(yaml).unwrap();
    let p = pino_of(&s);
    assert_eq!(p.auto_cache, false);
    assert_eq!(p.main_ttl, CacheTtl::OneHour);
    assert_eq!(p.sub_ttl, CacheTtl::FiveMin);
    assert_eq!(p.drop_tools, vec!["Foo".to_string(), "Bar".to_string()]);
    assert_eq!(p.strip_ansi, false);
    assert_eq!(p.model_override, Some("claude-x".to_string()));
}

#[test]
fn untagged_settings_parse_headroom() {
    let s: ProxySettings = serde_yaml::from_str("compression: true\n").unwrap();
    assert_eq!(headroom_of(&s).compression, true);
}

#[test]
fn untagged_settings_parse_central() {
    let s: ProxySettings = serde_yaml::from_str("port: 5599\npinned_version: 1.2.3\n").unwrap();
    let c = central_of(&s);
    assert_eq!(c.port, Some(5599));
    assert_eq!(c.pinned_version, Some("1.2.3".to_string()));
}

#[test]
fn central_settings_default_when_fields_omitted() {
    // Both fields are optional; an empty central mapping yields all-None.
    let s: ProxySettings = serde_yaml::from_str("{}\n").unwrap();
    // An empty mapping is structurally ambiguous only if a variant has no required
    // fields; CentralSettings is the only all-optional variant, so it wins.
    let c = central_of(&s);
    assert_eq!(c.port, None);
    assert_eq!(c.pinned_version, None);
}

#[test]
fn pino_settings_invalid_cache_ttl_falls_back_to_five_min_not_an_error() {
    // R22/R23k: M1 defines `CacheTtl` with a CUSTOM lenient `Deserialize` that maps
    // any unrecognized value to `FiveMin` (Node parseTailTtl parity) — it must NOT
    // hard-error. M2 asserts the fallback here (M4 relies on it). A pino settings
    // mapping with an invalid `sub_ttl: 7m` therefore parses successfully with
    // sub_ttl == FiveMin (the rest of the pino fields parse normally).
    let yaml = r#"
auto_cache: true
main_ttl: 1h
sub_ttl: 7m
drop_tools: []
strip_ansi: true
model_override: null
"#;
    let s: ProxySettings = serde_yaml::from_str(yaml)
        .expect("invalid cache TTL must NOT be a deserialization error (lenient parse)");
    let p = pino_of(&s);
    assert_eq!(p.main_ttl, CacheTtl::OneHour);
    assert_eq!(
        p.sub_ttl,
        CacheTtl::FiveMin,
        "an unrecognized cache TTL must fall back to FiveMin, not error"
    );
}

#[test]
fn cache_ttl_invalid_value_deserializes_to_five_min() {
    // Direct check of the lenient CacheTtl::Deserialize contract from M1 (R23k):
    // a bare invalid scalar maps to FiveMin rather than failing.
    let t: CacheTtl = serde_yaml::from_str("nonsense\n")
        .expect("invalid CacheTtl must deserialize leniently to FiveMin");
    assert_eq!(t, CacheTtl::FiveMin);
    // And the valid tokens still parse exactly.
    assert_eq!(
        serde_yaml::from_str::<CacheTtl>("5m\n").unwrap(),
        CacheTtl::FiveMin
    );
    assert_eq!(
        serde_yaml::from_str::<CacheTtl>("1h\n").unwrap(),
        CacheTtl::OneHour
    );
}

#[test]
fn pino_settings_rejects_legacy_tail_ttl_key() {
    let yaml =
        "auto_cache: true\ntail_ttl: 5m\ndrop_tools: []\nstrip_ansi: true\nmodel_override: null\n";
    let err = serde_yaml::from_str::<PinoSettings>(yaml).unwrap_err();
    assert!(
        err.to_string().contains("tail_ttl") || err.to_string().contains("unknown field"),
        "legacy tail_ttl must be rejected by deny_unknown_fields, got: {err}"
    );
}

use crate::test_support::ConfigHomeGuard;
use std::path::Path;

fn write_file(path: &Path, contents: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

#[test]
fn load_or_create_writes_default_when_absent() {
    let g = ConfigHomeGuard::new();
    assert!(!g.config_file().exists());

    let cfg = Config::load_or_create().unwrap();
    assert_eq!(cfg, Config::default_all_disabled());
    // It actually wrote the file.
    assert!(g.config_file().exists());

    // The written bytes parse back to the same config.
    let on_disk = std::fs::read_to_string(g.config_file()).unwrap();
    let parsed: Config = serde_yaml::from_str(&on_disk).unwrap();
    assert_eq!(parsed, Config::default_all_disabled());
}

#[test]
fn load_or_create_is_idempotent_and_does_not_overwrite_user_edits() {
    let g = ConfigHomeGuard::new();
    // First call creates the default.
    let _ = Config::load_or_create().unwrap();

    // User enables pino by hand (main_ttl 1h).
    let edited = r#"version: 1
proxies:
  - name: pino
    enabled: true
    settings:
      auto_cache: true
      main_ttl: 1h
      sub_ttl: 5m
      drop_tools: []
      strip_ansi: true
      model_override: null
  - name: headroom
    enabled: false
    settings:
      compression: false
  - name: central
    enabled: false
    settings:
      port: null
      pinned_version: null
defaults:
  enable_tool_search: true
"#;
    write_file(&g.config_file(), edited);

    let cfg = Config::load_or_create().unwrap();
    assert_eq!(cfg.proxies[0].name, ProxyName::Pino);
    assert_eq!(cfg.proxies[0].enabled, true);
    match &cfg.proxies[0].settings {
        ProxySettings::Pino(p) => assert_eq!(p.main_ttl, CacheTtl::OneHour),
        other => panic!("expected pino, got {other:?}"),
    }
}

#[test]
fn load_or_create_errors_when_settings_variant_mismatches_name() {
    let g = ConfigHomeGuard::new();
    // `name: pino` but settings are headroom-shaped (compression).
    let bad = r#"version: 1
proxies:
  - name: pino
    enabled: true
    settings:
      compression: true
defaults:
  enable_tool_search: true
"#;
    write_file(&g.config_file(), bad);

    let err = Config::load_or_create().unwrap_err();
    let msg = err.to_string().to_lowercase();
    assert!(
        msg.contains("pino"),
        "error should mention the proxy name: {msg}"
    );
    assert!(
        msg.contains("settings") || msg.contains("mismatch"),
        "error should mention settings mismatch: {msg}"
    );
}

#[test]
fn load_or_create_errors_when_central_not_last() {
    let g = ConfigHomeGuard::new();
    let bad = r#"version: 1
proxies:
  - name: central
    enabled: true
    settings:
      port: null
      pinned_version: null
  - name: pino
    enabled: true
    settings:
      auto_cache: true
      main_ttl: 1h
      sub_ttl: 5m
      drop_tools: []
      strip_ansi: true
      model_override: null
defaults:
  enable_tool_search: true
"#;
    write_file(&g.config_file(), bad);

    let err = Config::load_or_create().unwrap_err();
    let msg = err.to_string().to_lowercase();
    assert!(
        msg.contains("central"),
        "error should mention central: {msg}"
    );
    assert!(
        msg.contains("last"),
        "error should mention last-position rule: {msg}"
    );
}

#[test]
fn save_writes_atomically_and_reloads_equal() {
    let g = ConfigHomeGuard::new();
    let mut cfg = Config::default_all_disabled();
    // Enable headroom with compression on.
    cfg.proxies[1].enabled = true;
    cfg.proxies[1].settings = ProxySettings::Headroom(HeadroomSettings { compression: true });

    cfg.save().unwrap();
    assert!(g.config_file().exists());

    let reloaded = Config::load_or_create().unwrap();
    assert_eq!(reloaded, cfg);
    assert_eq!(reloaded.proxies[1].enabled, true);
    match &reloaded.proxies[1].settings {
        ProxySettings::Headroom(h) => assert_eq!(h.compression, true),
        other => panic!("expected headroom, got {other:?}"),
    }
}

#[test]
fn save_leaves_no_temp_files_in_config_dir() {
    let g = ConfigHomeGuard::new();
    let cfg = Config::default_all_disabled();
    cfg.save().unwrap();

    let entries: Vec<String> = std::fs::read_dir(g.config_file().parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(entries, vec!["poverty-mode.yaml".to_string()]);
}

#[test]
fn save_rejects_central_not_last() {
    let _g = ConfigHomeGuard::new();
    let mut cfg = Config::default_all_disabled();
    // Move central to the front => invalid.
    cfg.proxies.rotate_right(1);
    assert_eq!(cfg.proxies[0].name, ProxyName::Central);

    let err = cfg.save().unwrap_err();
    let msg = err.to_string().to_lowercase();
    assert!(msg.contains("central"));
    assert!(msg.contains("last"));
}

#[cfg(unix)]
#[test]
fn save_writes_config_file_0600_on_unix() {
    use std::os::unix::fs::PermissionsExt;
    let g = ConfigHomeGuard::new();
    let cfg = Config::default_all_disabled();
    cfg.save().unwrap();

    let mode = std::fs::metadata(g.config_file())
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(
        mode, 0o600,
        "config file must be owner-only on POSIX, got {mode:o}"
    );
}

fn enabled_default() -> Config {
    // pino + headroom + central all present and enabled, so file-order resolution
    // returns all three (central last).
    let mut c = Config::default_all_disabled();
    for e in &mut c.proxies {
        e.enabled = true;
    }
    c
}

fn names_of(chain: &[ResolvedProxy]) -> Vec<ProxyName> {
    chain.iter().map(|r| r.name).collect()
}

#[test]
fn resolve_uses_config_file_order_when_no_cli_no_env() {
    let cfg = enabled_default();
    let chain = cfg.resolve_chain(None, None).unwrap();
    assert_eq!(
        names_of(&chain),
        vec![ProxyName::Pino, ProxyName::Headroom, ProxyName::Central]
    );
}

#[test]
fn resolve_file_order_skips_disabled() {
    let mut cfg = Config::default_all_disabled();
    cfg.proxies[0].enabled = true; // pino
    cfg.proxies[1].enabled = false; // headroom off
    cfg.proxies[2].enabled = true; // central
    let chain = cfg.resolve_chain(None, None).unwrap();
    assert_eq!(names_of(&chain), vec![ProxyName::Pino, ProxyName::Central]);
}

#[test]
fn resolve_empty_when_nothing_enabled() {
    let cfg = Config::default_all_disabled(); // all disabled
    let chain = cfg.resolve_chain(None, None).unwrap();
    assert!(chain.is_empty());
}

#[test]
fn resolve_cli_overrides_env_and_file() {
    let cfg = enabled_default();
    // CLI says headroom only; env says pino,central; file says all three.
    let chain = cfg
        .resolve_chain(Some(&[ProxyName::Headroom]), Some("pino,central"))
        .unwrap();
    assert_eq!(names_of(&chain), vec![ProxyName::Headroom]);
}

#[test]
fn resolve_env_overrides_file_when_no_cli() {
    let cfg = enabled_default();
    let chain = cfg.resolve_chain(None, Some("headroom,pino")).unwrap();
    // env order is honored (central not requested): headroom then pino.
    assert_eq!(names_of(&chain), vec![ProxyName::Headroom, ProxyName::Pino]);
}

#[test]
fn resolve_env_empty_string_is_empty_chain() {
    let cfg = enabled_default();
    let chain = cfg.resolve_chain(None, Some("")).unwrap();
    assert!(chain.is_empty());
}

#[test]
fn resolve_env_trims_whitespace_around_names() {
    let cfg = enabled_default();
    let chain = cfg.resolve_chain(None, Some("  pino , headroom ")).unwrap();
    assert_eq!(names_of(&chain), vec![ProxyName::Pino, ProxyName::Headroom]);
}

#[test]
fn resolve_cli_carries_settings_from_config_entry() {
    let mut cfg = enabled_default();
    // Customize pino settings in the config.
    cfg.proxies[0].settings = ProxySettings::Pino(PinoSettings {
        auto_cache: false,
        main_ttl: CacheTtl::OneHour,
        sub_ttl: CacheTtl::FiveMin,
        drop_tools: vec!["Bash".to_string()],
        strip_ansi: false,
        model_override: Some("claude-z".to_string()),
    });
    let chain = cfg.resolve_chain(Some(&[ProxyName::Pino]), None).unwrap();
    assert_eq!(chain.len(), 1);
    match &chain[0].settings {
        ProxySettings::Pino(p) => {
            assert_eq!(p.auto_cache, false);
            assert_eq!(p.main_ttl, CacheTtl::OneHour);
            assert_eq!(p.drop_tools, vec!["Bash".to_string()]);
            assert_eq!(p.strip_ansi, false);
            assert_eq!(p.model_override, Some("claude-z".to_string()));
        }
        other => panic!("expected pino settings, got {other:?}"),
    }
}

#[test]
fn resolve_coerces_central_last_when_cli_lists_it_last_already() {
    let cfg = enabled_default();
    let chain = cfg
        .resolve_chain(Some(&[ProxyName::Pino, ProxyName::Central]), None)
        .unwrap();
    assert_eq!(names_of(&chain), vec![ProxyName::Pino, ProxyName::Central]);
}

#[test]
fn resolve_errors_when_cli_requests_central_not_last() {
    let cfg = enabled_default();
    let err = cfg
        .resolve_chain(Some(&[ProxyName::Central, ProxyName::Pino]), None)
        .unwrap_err();
    let msg = err.to_string().to_lowercase();
    assert!(msg.contains("central"), "msg: {msg}");
    assert!(msg.contains("last"), "msg: {msg}");
}

#[test]
fn resolve_errors_when_env_requests_central_not_last() {
    let cfg = enabled_default();
    let err = cfg.resolve_chain(None, Some("central,pino")).unwrap_err();
    let msg = err.to_string().to_lowercase();
    assert!(msg.contains("central"));
    assert!(msg.contains("last"));
}

#[test]
fn resolve_errors_on_duplicate_in_cli() {
    let cfg = enabled_default();
    let err = cfg
        .resolve_chain(Some(&[ProxyName::Pino, ProxyName::Pino]), None)
        .unwrap_err();
    assert!(err.to_string().to_lowercase().contains("duplicate"));
}

#[test]
fn resolve_errors_on_duplicate_in_env() {
    let cfg = enabled_default();
    let err = cfg.resolve_chain(None, Some("pino,pino")).unwrap_err();
    assert!(err.to_string().to_lowercase().contains("duplicate"));
}

#[test]
fn resolve_errors_on_unknown_env_name() {
    let cfg = enabled_default();
    let err = cfg.resolve_chain(None, Some("pino,bogus")).unwrap_err();
    assert!(err.to_string().to_lowercase().contains("bogus"));
}

#[test]
fn resolve_errors_when_requested_central_missing_from_config() {
    // Config with no central entry, but CLI requests central => no settings.
    let mut cfg = Config::default_all_disabled();
    cfg.proxies.retain(|e| e.name != ProxyName::Central);
    let err = cfg
        .resolve_chain(Some(&[ProxyName::Central]), None)
        .unwrap_err();
    let msg = err.to_string().to_lowercase();
    assert!(msg.contains("central"), "msg: {msg}");
}

#[test]
fn resolve_errors_when_requested_pino_missing_from_config() {
    // Config trimmed to only headroom+central, but CLI requests pino => no settings.
    let mut cfg = enabled_default();
    cfg.proxies.retain(|e| e.name != ProxyName::Pino);
    let err = cfg
        .resolve_chain(Some(&[ProxyName::Pino]), None)
        .unwrap_err();
    let msg = err.to_string().to_lowercase();
    assert!(msg.contains("pino"), "msg: {msg}");
}

#[test]
fn resolve_file_source_rejects_central_not_last_invariant() {
    // A directly-constructed Config (no load/validate) with central NOT last and
    // everything enabled must NOT silently yield a central-not-last chain from the
    // file source. resolve_chain validates the invariant on every source path.
    let mut cfg = enabled_default();
    cfg.proxies.rotate_right(1); // central -> front
    assert_eq!(cfg.proxies[0].name, ProxyName::Central);

    let err = cfg.resolve_chain(None, None).unwrap_err();
    let msg = err.to_string().to_lowercase();
    assert!(msg.contains("central"), "msg: {msg}");
    assert!(msg.contains("last"), "msg: {msg}");
}

#[test]
fn resolve_file_source_rejects_settings_name_mismatch_invariant() {
    // A directly-constructed Config where pino's entry carries headroom settings
    // must be rejected by the file source too (validate runs on every path).
    let mut cfg = enabled_default();
    cfg.proxies[0].settings = ProxySettings::Headroom(HeadroomSettings { compression: true });
    let err = cfg.resolve_chain(None, None).unwrap_err();
    let msg = err.to_string().to_lowercase();
    assert!(msg.contains("pino"), "msg: {msg}");
    assert!(
        msg.contains("settings") || msg.contains("mismatch"),
        "msg: {msg}"
    );
}

#[test]
fn save_resolved_chain_rewrites_order_and_enabled_set() {
    // Start from a config where the file order is pino,headroom,central and only
    // pino is enabled; persist a *resolved* chain of [headroom, central]. The
    // on-disk config lists the enabled non-central chain members first (headroom),
    // then the disabled remainder (pino), with central forced LAST so the
    // central-last invariant holds. Exactly the chain members are enabled.
    let _g = ConfigHomeGuard::new();
    let mut cfg = Config::default_all_disabled();
    cfg.proxies[0].enabled = true; // pino enabled in the original

    let chain = cfg
        .resolve_chain(Some(&[ProxyName::Headroom, ProxyName::Central]), None)
        .unwrap();
    cfg.save_resolved_chain(&chain).unwrap();

    let reloaded = Config::load_or_create().unwrap();
    let names: Vec<ProxyName> = reloaded.proxies.iter().map(|e| e.name).collect();
    assert_eq!(
        names,
        vec![ProxyName::Headroom, ProxyName::Pino, ProxyName::Central]
    );
    let enabled: Vec<(ProxyName, bool)> = reloaded
        .proxies
        .iter()
        .map(|e| (e.name, e.enabled))
        .collect();
    assert_eq!(
        enabled,
        vec![
            (ProxyName::Headroom, true),
            (ProxyName::Pino, false),
            (ProxyName::Central, true),
        ]
    );
    // central-last invariant holds, so the reload validated cleanly (above).
    assert_eq!(reloaded.proxies.last().unwrap().name, ProxyName::Central);
}

#[test]
fn save_resolved_chain_carries_resolved_settings_and_reloads_equal() {
    let _g = ConfigHomeGuard::new();
    let mut cfg = enabled_default();
    cfg.proxies[0].settings = ProxySettings::Pino(PinoSettings {
        auto_cache: false,
        main_ttl: CacheTtl::OneHour,
        sub_ttl: CacheTtl::FiveMin,
        drop_tools: vec!["Bash".to_string()],
        strip_ansi: false,
        model_override: Some("claude-z".to_string()),
    });
    let chain = cfg.resolve_chain(Some(&[ProxyName::Pino]), None).unwrap();
    cfg.save_resolved_chain(&chain).unwrap();

    let reloaded = Config::load_or_create().unwrap();
    let pino = reloaded
        .proxies
        .iter()
        .find(|e| e.name == ProxyName::Pino)
        .unwrap();
    assert_eq!(pino.enabled, true);
    match &pino.settings {
        ProxySettings::Pino(p) => {
            assert_eq!(p.model_override, Some("claude-z".to_string()));
            assert_eq!(p.main_ttl, CacheTtl::OneHour);
        }
        other => panic!("expected pino settings, got {other:?}"),
    }
    // The persisted file is valid (central-last holds) and reloads without error.
    assert_eq!(reloaded, Config::load_or_create().unwrap());
}

#[test]
fn save_resolved_chain_keeps_central_last() {
    // A chain that ends in central must produce a central-last on-disk config that
    // passes validation on reload.
    let _g = ConfigHomeGuard::new();
    let cfg = enabled_default();
    let chain = cfg
        .resolve_chain(Some(&[ProxyName::Pino, ProxyName::Central]), None)
        .unwrap();
    cfg.save_resolved_chain(&chain).unwrap();

    let reloaded = Config::load_or_create().unwrap();
    assert_eq!(reloaded.proxies.last().unwrap().name, ProxyName::Central);
}

#[test]
fn characterization_default_yaml_has_spec_5_2_shape() {
    // Pure characterization guard (R12): pins the spec 5.2 first-run YAML so a
    // future serde-attribute drift in the shared settings structs is caught here.
    let yaml = serde_yaml::to_string(&Config::default_all_disabled()).unwrap();

    // version + defaults block.
    assert!(yaml.contains("version: 1"), "yaml:\n{yaml}");
    assert!(yaml.contains("enable_tool_search: true"), "yaml:\n{yaml}");

    // Three named proxies, all disabled, in canonical order.
    let pino_at = yaml.find("name: pino").expect("pino present");
    let headroom_at = yaml.find("name: headroom").expect("headroom present");
    let central_at = yaml.find("name: central").expect("central present");
    assert!(
        pino_at < headroom_at && headroom_at < central_at,
        "order; yaml:\n{yaml}"
    );
    assert_eq!(
        yaml.matches("enabled: false").count(),
        3,
        "all disabled; yaml:\n{yaml}"
    );

    // Pino settings shape.
    assert!(yaml.contains("auto_cache: true"), "yaml:\n{yaml}");
    assert!(yaml.contains("main_ttl: 1h"), "yaml:\n{yaml}");
    assert!(yaml.contains("sub_ttl: 5m"), "yaml:\n{yaml}");
    assert!(yaml.contains("strip_ansi: true"), "yaml:\n{yaml}");
    // Headroom + central settings shape.
    assert!(yaml.contains("compression: false"), "yaml:\n{yaml}");
    // central's null fields round-trip; re-parsing yields the canonical default.
    let back: Config = serde_yaml::from_str(&yaml).unwrap();
    assert_eq!(back, Config::default_all_disabled());
}

// `config show` rendering (FIX-D) =====

#[test]
fn render_config_matches_save_serialization_and_round_trips() {
    let cfg = Config::default_all_disabled();
    let rendered = render_config(&cfg).unwrap();
    // `show` renders exactly what `save` would write (same serde path).
    assert_eq!(rendered, serde_yaml::to_string(&cfg).unwrap());
    // The rendered text re-parses back into the same config.
    let back: Config = serde_yaml::from_str(&rendered).unwrap();
    assert_eq!(back, cfg);
}

// `config edit` editor resolution (FIX-D) =====

#[test]
fn resolve_editor_prefers_visual_then_editor_then_fallback() {
    // $VISUAL wins over $EDITOR.
    assert_eq!(
        resolve_editor(Some("vis"), Some("ed")),
        vec!["vis".to_string()]
    );
    // $EDITOR used when $VISUAL is unset.
    assert_eq!(resolve_editor(None, Some("ed")), vec!["ed".to_string()]);
    // Multi-word editor commands split into argv (e.g. `code --wait`).
    assert_eq!(
        resolve_editor(None, Some("code --wait")),
        vec!["code".to_string(), "--wait".to_string()]
    );
}

#[test]
fn resolve_editor_treats_blank_env_as_unset_and_falls_back() {
    // Whitespace-only values are ignored; an empty $VISUAL falls through to $EDITOR.
    assert_eq!(
        resolve_editor(Some("   "), Some("ed")),
        vec!["ed".to_string()]
    );
    // Neither set => a single-element platform fallback (notepad on Windows, vi elsewhere).
    let fallback = resolve_editor(None, None);
    assert_eq!(fallback.len(), 1);
    let expected = if cfg!(windows) { "notepad" } else { "vi" };
    assert_eq!(fallback, vec![expected.to_string()]);
}
