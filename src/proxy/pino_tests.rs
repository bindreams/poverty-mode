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

// M4.3 ===== model_override: replace body.model + rewrite model self-references
// in system blocks (port of reference/pino/src/model.js rewriteSystemModelRefs +
// the server.js `parsed.model = MODEL_OVERRIDE` step). R18: closure replacement
// so a '$' in the override is emitted verbatim (never a regex template).

fn model_override_settings(model: &str) -> PinoSettings {
    PinoSettings {
        auto_cache: false,
        tail_ttl: TailTtl::FiveMin,
        drop_tools: vec![],
        strip_ansi: false,
        model_override: Some(model.to_string()),
    }
}

#[test]
fn model_override_replaces_top_level_model_field() {
    let t = PinoTransform {
        settings: model_override_settings("claude-opus-4-6"),
    };
    let mut body = json!({ "model": "claude-sonnet-4-5", "messages": [] });
    t.transform(&mut body).unwrap();
    assert_eq!(body["model"], json!("claude-opus-4-6"));
}

#[test]
fn model_override_rewrites_source_id_in_system_string() {
    let t = PinoTransform {
        settings: model_override_settings("claude-opus-4-6"),
    };
    let mut body = json!({
        "model": "x",
        "system": "You are claude-opus-4-7-20260101, also called Opus 4.7.",
        "messages": []
    });
    t.transform(&mut body).unwrap();
    assert_eq!(
        body["system"],
        json!("You are claude-opus-4-6, also called Opus 4.6.")
    );
}

#[test]
fn model_override_rewrites_source_id_in_system_blocks_array() {
    let t = PinoTransform {
        settings: model_override_settings("claude-sonnet-4-6-20260301"),
    };
    let mut body = json!({
        "model": "x",
        "system": [
            { "type": "text", "text": "Model: claude-opus-4-7. Name: Opus 4.7." },
            { "type": "text", "text": "no refs here" }
        ],
        "messages": []
    });
    t.transform(&mut body).unwrap();
    // Override base = claude-sonnet-4-6 (date suffix stripped) => friendly "Sonnet 4.6".
    // The bare source id (no date) is replaced with the FULL override INCLUDING the date.
    assert_eq!(
        body["system"][0]["text"],
        json!("Model: claude-sonnet-4-6-20260301. Name: Sonnet 4.6.")
    );
    assert_eq!(body["system"][1]["text"], json!("no refs here"));
}

#[test]
fn model_override_unknown_target_uses_base_id_as_friendly_name() {
    let t = PinoTransform {
        settings: model_override_settings("claude-future-9-9"),
    };
    let mut body = json!({
        "model": "x",
        "system": "id claude-opus-4-7 and name Opus 4.7",
        "messages": []
    });
    t.transform(&mut body).unwrap();
    // Unknown base => friendly falls back to the base id itself (Node `|| base`).
    assert_eq!(
        body["system"],
        json!("id claude-future-9-9 and name claude-future-9-9")
    );
}

#[test]
fn model_override_none_leaves_system_untouched() {
    let t = PinoTransform {
        settings: no_op_settings(),
    };
    let mut body = json!({
        "model": "claude-opus-4-7",
        "system": "I am claude-opus-4-7 (Opus 4.7)",
        "messages": []
    });
    t.transform(&mut body).unwrap();
    assert_eq!(body["model"], json!("claude-opus-4-7"));
    assert_eq!(body["system"], json!("I am claude-opus-4-7 (Opus 4.7)"));
}

// R18 / Finding 3: an override containing a literal '$' must be emitted verbatim,
// NOT interpreted as a regex replacement template ($name / ${name} / $N).
#[test]
fn model_override_with_literal_dollar_is_not_treated_as_template() {
    let t = PinoTransform {
        settings: model_override_settings("claude-$weird-4-6"),
    };
    let mut body = json!({
        "model": "x",
        "system": "self: claude-opus-4-7",
        "messages": []
    });
    t.transform(&mut body).unwrap();
    assert_eq!(body["model"], json!("claude-$weird-4-6"));
    assert_eq!(body["system"], json!("self: claude-$weird-4-6"));
}

#[test]
fn model_override_friendly_with_literal_dollar_is_not_treated_as_template() {
    // Both the id-replacement AND the friendly-name replacement use closures.
    // Override "claude-$x" has unknown base => friendly == base == "claude-$x",
    // so the "Opus 4.7" -> friendly substitution must also emit '$' literally.
    let t = PinoTransform {
        settings: model_override_settings("claude-$x"),
    };
    let mut body = json!({
        "model": "x",
        "system": "name Opus 4.7",
        "messages": []
    });
    t.transform(&mut body).unwrap();
    assert_eq!(body["system"], json!("name claude-$x"));
}

// M4.4 ===== strip_ansi: scrub ANSI/CSI escapes from message text + tool-result
// content (port of stripAnsiFromMessages / stripAnsi, default.js lines 42-70).
// Node regex /\x1b\[[0-9;]*[A-Za-z]/g. Default-on, gated by settings.strip_ansi.

fn strip_only_settings() -> PinoSettings {
    PinoSettings {
        auto_cache: false,
        tail_ttl: TailTtl::FiveMin,
        drop_tools: vec![],
        strip_ansi: true,
        model_override: None,
    }
}

#[test]
fn strip_ansi_cleans_string_message_content() {
    let t = PinoTransform {
        settings: strip_only_settings(),
    };
    let mut body = json!({
        "messages": [
            { "role": "user", "content": "\u{1b}[31mred\u{1b}[0m text" }
        ]
    });
    t.transform(&mut body).unwrap();
    assert_eq!(body["messages"][0]["content"], json!("red text"));
}

#[test]
fn strip_ansi_cleans_block_text_and_block_content_string() {
    let t = PinoTransform {
        settings: strip_only_settings(),
    };
    let mut body = json!({
        "messages": [
            { "role": "user", "content": [
                { "type": "text", "text": "\u{1b}[1mbold\u{1b}[22m" },
                { "type": "tool_result", "content": "\u{1b}[32mok\u{1b}[0m" }
            ] }
        ]
    });
    t.transform(&mut body).unwrap();
    assert_eq!(body["messages"][0]["content"][0]["text"], json!("bold"));
    assert_eq!(body["messages"][0]["content"][1]["content"], json!("ok"));
}

#[test]
fn strip_ansi_cleans_nested_tool_result_content_array() {
    let t = PinoTransform {
        settings: strip_only_settings(),
    };
    let mut body = json!({
        "messages": [
            { "role": "user", "content": [
                { "type": "tool_result", "content": [
                    { "type": "text", "text": "\u{1b}[33mwarn\u{1b}[39m line" }
                ] }
            ] }
        ]
    });
    t.transform(&mut body).unwrap();
    assert_eq!(
        body["messages"][0]["content"][0]["content"][0]["text"],
        json!("warn line")
    );
}

#[test]
fn strip_ansi_disabled_leaves_escapes_intact() {
    let mut s = strip_only_settings();
    s.strip_ansi = false;
    let t = PinoTransform { settings: s };
    let mut body = json!({
        "messages": [ { "role": "user", "content": "\u{1b}[31mred\u{1b}[0m" } ]
    });
    t.transform(&mut body).unwrap();
    assert_eq!(
        body["messages"][0]["content"],
        json!("\u{1b}[31mred\u{1b}[0m")
    );
}

#[test]
fn strip_ansi_only_matches_csi_sgr_form_not_arbitrary_text() {
    // The Node regex only matches ESC [ <params> <letter>; literal "[31m" without ESC stays.
    let t = PinoTransform {
        settings: strip_only_settings(),
    };
    let mut body = json!({
        "messages": [ { "role": "user", "content": "literal [31m stays" } ]
    });
    t.transform(&mut body).unwrap();
    assert_eq!(body["messages"][0]["content"], json!("literal [31m stays"));
}
