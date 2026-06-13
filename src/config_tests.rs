// The plan (R12) authors these assertions with explicit `assert_eq!(field, true)`
// /`false` forms to read each boolean config field by name; keep them verbatim and
// scope the clippy lint to this test module rather than rewrite the plan's tests.
#![allow(clippy::bool_assert_comparison)]

use super::*;
use crate::proxy::headroom::HeadroomSettings;
use crate::proxy::pino::{PinoSettings, TailTtl};
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
    assert_eq!(names, vec![ProxyName::Pino, ProxyName::Headroom, ProxyName::Central]);

    for e in &cfg.proxies {
        assert_eq!(e.enabled, false, "proxy {:?} must default disabled", e.name);
    }
}

#[test]
fn default_all_disabled_has_expected_per_proxy_settings() {
    let cfg = Config::default_all_disabled();

    let pino = pino_of(&cfg.proxies[0].settings);
    assert_eq!(pino.auto_cache, true);
    assert_eq!(pino.tail_ttl, TailTtl::FiveMin);
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
fn yaml_uses_lowercase_proxy_names_and_5m_tail_ttl() {
    let cfg = Config::default_all_disabled();
    let yaml = serde_yaml::to_string(&cfg).unwrap();
    assert!(yaml.contains("name: pino"), "yaml was:\n{yaml}");
    assert!(yaml.contains("name: headroom"), "yaml was:\n{yaml}");
    assert!(yaml.contains("name: central"), "yaml was:\n{yaml}");
    assert!(yaml.contains("5m"), "tail_ttl should serialize as 5m; yaml was:\n{yaml}");
}

#[test]
fn untagged_settings_parse_pino_not_headroom() {
    // A mapping with pino-only fields must parse as Pino, never Headroom.
    let yaml = r#"
auto_cache: false
tail_ttl: 1h
drop_tools: ["Foo", "Bar"]
strip_ansi: false
model_override: claude-x
"#;
    let s: ProxySettings = serde_yaml::from_str(yaml).unwrap();
    let p = pino_of(&s);
    assert_eq!(p.auto_cache, false);
    assert_eq!(p.tail_ttl, TailTtl::OneHour);
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
fn pino_settings_invalid_tail_ttl_falls_back_to_five_min_not_an_error() {
    // R22/R23k: M1 defines `TailTtl` with a CUSTOM lenient `Deserialize` that maps
    // any unrecognized value to `FiveMin` (Node parseTailTtl parity) — it must NOT
    // hard-error. M2 asserts the fallback here (M4 relies on it). A pino settings
    // mapping with `tail_ttl: 7m` therefore parses successfully with tail_ttl ==
    // FiveMin (the rest of the pino fields parse normally).
    let yaml = r#"
auto_cache: true
tail_ttl: 7m
drop_tools: []
strip_ansi: true
model_override: null
"#;
    let s: ProxySettings = serde_yaml::from_str(yaml)
        .expect("invalid tail_ttl must NOT be a deserialization error (lenient parse)");
    let p = pino_of(&s);
    assert_eq!(
        p.tail_ttl,
        TailTtl::FiveMin,
        "an unrecognized tail_ttl must fall back to FiveMin, not error"
    );
}

#[test]
fn tail_ttl_invalid_value_deserializes_to_five_min() {
    // Direct check of the lenient TailTtl::Deserialize contract from M1 (R23k):
    // a bare invalid scalar maps to FiveMin rather than failing.
    let t: TailTtl = serde_yaml::from_str("nonsense\n")
        .expect("invalid TailTtl must deserialize leniently to FiveMin");
    assert_eq!(t, TailTtl::FiveMin);
    // And the valid tokens still parse exactly.
    assert_eq!(serde_yaml::from_str::<TailTtl>("5m\n").unwrap(), TailTtl::FiveMin);
    assert_eq!(serde_yaml::from_str::<TailTtl>("1h\n").unwrap(), TailTtl::OneHour);
}
