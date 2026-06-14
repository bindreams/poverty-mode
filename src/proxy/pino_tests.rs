use super::*;

#[test]
fn pino_settings_default_round_trips_yaml() {
    let s = PinoSettings {
        auto_cache: true,
        tail_ttl: TailTtl::FiveMin,
        drop_tools: vec![],
        strip_ansi: true,
        model_override: None,
    };
    let yaml = serde_yaml::to_string(&s).unwrap();
    let back: PinoSettings = serde_yaml::from_str(&yaml).unwrap();
    assert_eq!(s, back);
}

#[test]
fn tail_ttl_serializes_as_short_strings() {
    assert_eq!(
        serde_yaml::to_string(&TailTtl::FiveMin).unwrap().trim(),
        "5m"
    );
    assert_eq!(
        serde_yaml::to_string(&TailTtl::OneHour).unwrap().trim(),
        "1h"
    );
    let five: TailTtl = serde_yaml::from_str("\"5m\"").unwrap();
    let hour: TailTtl = serde_yaml::from_str("\"1h\"").unwrap();
    assert_eq!(five, TailTtl::FiveMin);
    assert_eq!(hour, TailTtl::OneHour);
}

#[test]
fn tail_ttl_invalid_value_falls_back_to_five_min() {
    // R22/R23k: the custom lenient Deserialize maps any invalid string to
    // FiveMin (Node parseTailTtl parity) instead of erroring. M2 also asserts
    // this from the config layer; M4 relies on it.
    let parsed: TailTtl = serde_yaml::from_str("\"7m\"").unwrap();
    assert_eq!(parsed, TailTtl::FiveMin);
    let parsed: TailTtl = serde_yaml::from_str("\"\"").unwrap();
    assert_eq!(parsed, TailTtl::FiveMin);
    let parsed: TailTtl = serde_yaml::from_str("\"banana\"").unwrap();
    assert_eq!(parsed, TailTtl::FiveMin);
}

#[test]
fn pino_settings_rejects_unknown_fields() {
    let yaml = "auto_cache: true\ntail_ttl: 5m\ndrop_tools: []\nstrip_ansi: true\nmodel_override: null\nbogus: 1\n";
    let err = serde_yaml::from_str::<PinoSettings>(yaml).unwrap_err();
    assert!(
        err.to_string().contains("bogus") || err.to_string().contains("unknown field"),
        "deny_unknown_fields should reject `bogus`, got: {err}"
    );
}

#[test]
fn pino_transform_apply_headers_is_noop_until_m4() {
    let t = PinoTransform {
        settings: PinoSettings {
            auto_cache: true,
            tail_ttl: TailTtl::FiveMin,
            drop_tools: vec![],
            strip_ansi: true,
            model_override: None,
        },
    };
    let mut headers = http::HeaderMap::new();
    crate::proxy::BodyTransform::apply_headers(&t, &mut headers);
    assert!(headers.is_empty());
}

// M4.1 ===== lock the PinoSettings / TailTtl serde wire shape + lenient
// tail_ttl fallback (Node parseTailTtl parity). `PinoSettings`/`TailTtl` are
// already in scope via `use super::*;` at the top of this file.

fn sample_settings() -> PinoSettings {
    PinoSettings {
        auto_cache: true,
        tail_ttl: TailTtl::FiveMin,
        drop_tools: vec!["NotebookEdit".to_string(), "CronList".to_string()],
        strip_ansi: true,
        model_override: None,
    }
}

// --- characterization guards: lock the serde wire shape (R12: added after the
// --- types already exist; NOT a red->green cycle) -----

#[test]
fn tail_ttl_serializes_as_human_strings() {
    assert_eq!(serde_json::to_string(&TailTtl::FiveMin).unwrap(), "\"5m\"");
    assert_eq!(serde_json::to_string(&TailTtl::OneHour).unwrap(), "\"1h\"");
}

#[test]
fn tail_ttl_deserializes_from_human_strings() {
    let five: TailTtl = serde_json::from_str("\"5m\"").unwrap();
    let hour: TailTtl = serde_json::from_str("\"1h\"").unwrap();
    assert_eq!(five, TailTtl::FiveMin);
    assert_eq!(hour, TailTtl::OneHour);
}

#[test]
fn pino_settings_round_trips_through_json() {
    let s = sample_settings();
    let json = serde_json::to_string(&s).unwrap();
    let back: PinoSettings = serde_json::from_str(&json).unwrap();
    assert_eq!(s, back);
}

#[test]
fn pino_settings_yaml_shape_matches_config_file() {
    // Mirrors the config.yaml default block in the design doc (spec 5.2):
    // settings: { auto_cache: true, tail_ttl: 5m, drop_tools: [], strip_ansi: true, model_override: null }
    let yaml =
        "auto_cache: true\ntail_ttl: 5m\ndrop_tools: []\nstrip_ansi: true\nmodel_override: null\n";
    let s: PinoSettings = serde_yaml::from_str(yaml).unwrap();
    assert!(s.auto_cache);
    assert_eq!(s.tail_ttl, TailTtl::FiveMin);
    assert!(s.drop_tools.is_empty());
    assert!(s.strip_ansi);
    assert_eq!(s.model_override, None);
}

// --- genuine red: Node parseTailTtl lowercases+trims before matching, and falls
// --- back to 5m on any unknown value (reference/pino/src/config.js lines 36-44).
// --- The M1 Deserialize is lenient but does an EXACT match, so "  1H " degrades
// --- to FiveMin instead of mapping to OneHour; this asserts the case-insensitive
// --- + trim parity this task adds. -----

#[test]
fn tail_ttl_invalid_value_falls_back_to_five_min_json() {
    let v: TailTtl = serde_json::from_str("\"10m\"").unwrap();
    assert_eq!(
        v,
        TailTtl::FiveMin,
        "unknown tail_ttl must degrade to 5m, not error"
    );
    let from_yaml: TailTtl = serde_yaml::from_str("nonsense").unwrap();
    assert_eq!(from_yaml, TailTtl::FiveMin);
}

#[test]
fn tail_ttl_is_case_insensitive_like_node() {
    // Node lowercases+trims before matching: "  1H " -> "1h".
    let v: TailTtl = serde_json::from_str("\"  1H \"").unwrap();
    assert_eq!(v, TailTtl::OneHour);
    let v2: TailTtl = serde_json::from_str("\"5M\"").unwrap();
    assert_eq!(v2, TailTtl::FiveMin);
}

// M4.2 ===== real dispatch skeleton + cache constants + no-op gate. With every
// feature off, `transform` must be a byte-faithful passthrough; the cache
// constants must match the Node config. (`PinoSettings`/`TailTtl`/`PinoTransform`
// are in scope via `use super::*;`; the constants + trait are imported below.)

use super::{BREAKPOINT_CEILING, MIN_SYSTEM_CACHE_CHARS};
use crate::proxy::BodyTransform;
use serde_json::json;

fn no_op_settings() -> PinoSettings {
    PinoSettings {
        auto_cache: false,
        tail_ttl: TailTtl::FiveMin,
        drop_tools: vec![],
        strip_ansi: false,
        model_override: None,
    }
}

#[test]
fn constants_match_node_config() {
    assert_eq!(BREAKPOINT_CEILING, 4);
    assert_eq!(MIN_SYSTEM_CACHE_CHARS, 500);
}

#[test]
fn all_features_off_is_a_no_op() {
    let t = PinoTransform {
        settings: no_op_settings(),
    };
    let original = json!({
        "model": "claude-sonnet-4-5",
        "system": [{ "type": "text", "text": "you are helpful" }],
        "tools": [{ "name": "Bash", "description": "run shell" }],
        "messages": [
            { "role": "user", "content": [{ "type": "text", "text": "hi" }] }
        ]
    });
    let mut body = original.clone();
    t.transform(&mut body).unwrap();
    assert_eq!(
        body, original,
        "no feature enabled => byte-faithful passthrough"
    );
}

#[test]
fn non_object_body_is_left_untouched_and_ok() {
    let t = PinoTransform {
        settings: no_op_settings(),
    };
    let mut body = json!("not an object");
    t.transform(&mut body).unwrap();
    assert_eq!(body, json!("not an object"));
}
