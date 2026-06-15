use super::*;

fn main_ctx() -> crate::proxy::RequestContext {
    crate::proxy::RequestContext::default()
}

fn sub_ctx() -> crate::proxy::RequestContext {
    crate::proxy::RequestContext { is_subagent: true }
}

#[test]
fn pino_settings_default_round_trips_yaml() {
    let s = PinoSettings {
        auto_cache: true,
        main_ttl: CacheTtl::OneHour,
        sub_ttl: CacheTtl::FiveMin,
        drop_tools: vec![],
        strip_ansi: true,
        model_override: None,
    };
    let yaml = serde_yaml::to_string(&s).unwrap();
    let back: PinoSettings = serde_yaml::from_str(&yaml).unwrap();
    assert_eq!(s, back);
}

#[test]
fn cache_ttl_serializes_as_short_strings() {
    assert_eq!(serde_yaml::to_string(&CacheTtl::FiveMin).unwrap().trim(), "5m");
    assert_eq!(serde_yaml::to_string(&CacheTtl::OneHour).unwrap().trim(), "1h");
    let five: CacheTtl = serde_yaml::from_str("\"5m\"").unwrap();
    let hour: CacheTtl = serde_yaml::from_str("\"1h\"").unwrap();
    assert_eq!(five, CacheTtl::FiveMin);
    assert_eq!(hour, CacheTtl::OneHour);
}

#[test]
fn cache_ttl_invalid_value_falls_back_to_five_min() {
    // R22/R23k: the custom lenient Deserialize maps any invalid string to
    // FiveMin (Node parseTailTtl parity) instead of erroring. M2 also asserts
    // this from the config layer; M4 relies on it.
    let parsed: CacheTtl = serde_yaml::from_str("\"7m\"").unwrap();
    assert_eq!(parsed, CacheTtl::FiveMin);
    let parsed: CacheTtl = serde_yaml::from_str("\"\"").unwrap();
    assert_eq!(parsed, CacheTtl::FiveMin);
    let parsed: CacheTtl = serde_yaml::from_str("\"banana\"").unwrap();
    assert_eq!(parsed, CacheTtl::FiveMin);
}

#[test]
fn pino_settings_rejects_unknown_fields() {
    let yaml = "auto_cache: true\nmain_ttl: 1h\nsub_ttl: 5m\ndrop_tools: []\nstrip_ansi: true\nmodel_override: null\nbogus: 1\n";
    let err = serde_yaml::from_str::<PinoSettings>(yaml).unwrap_err();
    assert!(
        err.to_string().contains("bogus") || err.to_string().contains("unknown field"),
        "deny_unknown_fields should reject `bogus`, got: {err}"
    );
}

// M4.1 ===== lock the PinoSettings / CacheTtl serde wire shape + lenient
// cache-TTL fallback (Node parseTailTtl parity). `PinoSettings`/`CacheTtl` are
// already in scope via `use super::*;` at the top of this file.

fn sample_settings() -> PinoSettings {
    PinoSettings {
        auto_cache: true,
        main_ttl: CacheTtl::OneHour,
        sub_ttl: CacheTtl::FiveMin,
        drop_tools: vec!["NotebookEdit".to_string(), "CronList".to_string()],
        strip_ansi: true,
        model_override: None,
    }
}

// --- characterization guards: lock the serde wire shape (R12: added after the
// --- types already exist; NOT a red->green cycle) --------------------------------------------------------------------

#[test]
fn cache_ttl_serializes_as_human_strings() {
    assert_eq!(serde_json::to_string(&CacheTtl::FiveMin).unwrap(), "\"5m\"");
    assert_eq!(serde_json::to_string(&CacheTtl::OneHour).unwrap(), "\"1h\"");
}

#[test]
fn cache_ttl_deserializes_from_human_strings() {
    let five: CacheTtl = serde_json::from_str("\"5m\"").unwrap();
    let hour: CacheTtl = serde_json::from_str("\"1h\"").unwrap();
    assert_eq!(five, CacheTtl::FiveMin);
    assert_eq!(hour, CacheTtl::OneHour);
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
    // settings: { auto_cache: true, main_ttl: 1h, sub_ttl: 5m, drop_tools: [], strip_ansi: true, model_override: null }
    let yaml = "auto_cache: true\nmain_ttl: 1h\nsub_ttl: 5m\ndrop_tools: []\nstrip_ansi: true\nmodel_override: null\n";
    let s: PinoSettings = serde_yaml::from_str(yaml).unwrap();
    assert!(s.auto_cache);
    assert_eq!(s.main_ttl, CacheTtl::OneHour);
    assert_eq!(s.sub_ttl, CacheTtl::FiveMin);
    assert!(s.drop_tools.is_empty());
    assert!(s.strip_ansi);
    assert_eq!(s.model_override, None);
}

// --- genuine red: Node parseTailTtl lowercases+trims before matching, and falls
// --- back to 5m on any unknown value (reference/pino/src/config.js lines 36-44).
// --- The M1 Deserialize is lenient but does an EXACT match, so "  1H " degrades
// --- to FiveMin instead of mapping to OneHour; this asserts the case-insensitive
// --- + trim parity this task adds. -----------------------------------------------------------------------------------

#[test]
fn cache_ttl_invalid_value_falls_back_to_five_min_json() {
    let v: CacheTtl = serde_json::from_str("\"10m\"").unwrap();
    assert_eq!(v, CacheTtl::FiveMin, "unknown cache TTL must degrade to 5m, not error");
    let from_yaml: CacheTtl = serde_yaml::from_str("nonsense").unwrap();
    assert_eq!(from_yaml, CacheTtl::FiveMin);
}

#[test]
fn cache_ttl_is_case_insensitive_like_node() {
    // Node lowercases+trims before matching: "  1H " -> "1h".
    let v: CacheTtl = serde_json::from_str("\"  1H \"").unwrap();
    assert_eq!(v, CacheTtl::OneHour);
    let v2: CacheTtl = serde_json::from_str("\"5M\"").unwrap();
    assert_eq!(v2, CacheTtl::FiveMin);
}

// M4.2 ===== real dispatch skeleton + cache constants + no-op gate. With every
// feature off, `transform` must be a byte-faithful passthrough; the cache
// constants must match the Node config. (`PinoSettings`/`CacheTtl`/`PinoTransform`
// are in scope via `use super::*;`; the constants + trait are imported below.)

use super::{BREAKPOINT_CEILING, MIN_SYSTEM_CACHE_CHARS};
use crate::proxy::BodyTransform;
use serde_json::json;

fn no_op_settings() -> PinoSettings {
    PinoSettings {
        auto_cache: false,
        main_ttl: CacheTtl::OneHour,
        sub_ttl: CacheTtl::FiveMin,
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
    t.transform(&mut body, &main_ctx()).unwrap();
    assert_eq!(body, original, "no feature enabled => byte-faithful passthrough");
}

#[test]
fn non_object_body_is_left_untouched_and_ok() {
    let t = PinoTransform {
        settings: no_op_settings(),
    };
    let mut body = json!("not an object");
    t.transform(&mut body, &main_ctx()).unwrap();
    assert_eq!(body, json!("not an object"));
}

// M4.3 ===== model_override: replace body.model + rewrite model self-references
// in system blocks (port of reference/pino/src/model.js rewriteSystemModelRefs +
// the server.js `parsed.model = MODEL_OVERRIDE` step). R18: closure replacement
// so a '$' in the override is emitted verbatim (never a regex template).

fn model_override_settings(model: &str) -> PinoSettings {
    PinoSettings {
        auto_cache: false,
        main_ttl: CacheTtl::OneHour,
        sub_ttl: CacheTtl::FiveMin,
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
    t.transform(&mut body, &main_ctx()).unwrap();
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
    t.transform(&mut body, &main_ctx()).unwrap();
    assert_eq!(body["system"], json!("You are claude-opus-4-6, also called Opus 4.6."));
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
    t.transform(&mut body, &main_ctx()).unwrap();
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
    t.transform(&mut body, &main_ctx()).unwrap();
    // Unknown base => friendly falls back to the base id itself (Node `|| base`).
    assert_eq!(body["system"], json!("id claude-future-9-9 and name claude-future-9-9"));
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
    t.transform(&mut body, &main_ctx()).unwrap();
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
    t.transform(&mut body, &main_ctx()).unwrap();
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
    t.transform(&mut body, &main_ctx()).unwrap();
    assert_eq!(body["system"], json!("name claude-$x"));
}

// M4.4 ===== strip_ansi: scrub ANSI/CSI escapes from message text + tool-result
// content (port of stripAnsiFromMessages / stripAnsi, default.js lines 42-70).
// Node regex /\x1b\[[0-9;]*[A-Za-z]/g. Default-on, gated by settings.strip_ansi.

fn strip_only_settings() -> PinoSettings {
    PinoSettings {
        auto_cache: false,
        main_ttl: CacheTtl::OneHour,
        sub_ttl: CacheTtl::FiveMin,
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
    t.transform(&mut body, &main_ctx()).unwrap();
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
    t.transform(&mut body, &main_ctx()).unwrap();
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
    t.transform(&mut body, &main_ctx()).unwrap();
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
    t.transform(&mut body, &main_ctx()).unwrap();
    assert_eq!(body["messages"][0]["content"], json!("\u{1b}[31mred\u{1b}[0m"));
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
    t.transform(&mut body, &main_ctx()).unwrap();
    assert_eq!(body["messages"][0]["content"], json!("literal [31m stays"));
}

// M4.5 ===== drop_tools: filter body.tools by name AND scrub dropped names from
// <system-reminder> blocks that advertise deferred tools / ToolSearch (port of
// trimTools + stripDroppedToolsFromReminder + trimReminders, default.js 72-113).

fn drop_settings(names: &[&str]) -> PinoSettings {
    PinoSettings {
        auto_cache: false,
        main_ttl: CacheTtl::OneHour,
        sub_ttl: CacheTtl::FiveMin,
        drop_tools: names.iter().map(|s| s.to_string()).collect(),
        strip_ansi: false,
        model_override: None,
    }
}

#[test]
fn drop_tools_removes_named_tools_from_tools_array() {
    let t = PinoTransform {
        settings: drop_settings(&["NotebookEdit", "CronList"]),
    };
    let mut body = json!({
        "tools": [
            { "name": "Bash" },
            { "name": "NotebookEdit" },
            { "name": "Read" },
            { "name": "CronList" }
        ],
        "messages": []
    });
    t.transform(&mut body, &main_ctx()).unwrap();
    let names: Vec<&str> = body["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["Bash", "Read"]);
}

#[test]
fn drop_tools_empty_leaves_tools_untouched() {
    let t = PinoTransform {
        settings: drop_settings(&[]),
    };
    let original = json!({ "tools": [ { "name": "Bash" }, { "name": "Read" } ], "messages": [] });
    let mut body = original.clone();
    t.transform(&mut body, &main_ctx()).unwrap();
    assert_eq!(body, original);
}

#[test]
fn drop_tools_scrubs_names_from_deferred_tools_reminder_in_string_content() {
    let t = PinoTransform {
        settings: drop_settings(&["NotebookEdit", "CronList"]),
    };
    let reminder =
        "<system-reminder>\nThe following are deferred tools:\nNotebookEdit\nBash\nCronList\nRead\n</system-reminder>";
    let mut body = json!({
        "tools": [],
        "messages": [ { "role": "user", "content": reminder } ]
    });
    t.transform(&mut body, &main_ctx()).unwrap();
    let out = body["messages"][0]["content"].as_str().unwrap();
    assert!(!out.contains("NotebookEdit"), "dropped name must be scrubbed");
    assert!(!out.contains("CronList"), "dropped name must be scrubbed");
    assert!(out.contains("Bash"), "kept tool name stays");
    assert!(out.contains("Read"), "kept tool name stays");
    assert!(out.contains("<system-reminder>"), "wrapper tags preserved");
}

#[test]
fn drop_tools_scrubs_names_from_toolsearch_reminder_in_block_text() {
    let t = PinoTransform {
        settings: drop_settings(&["Monitor"]),
    };
    let reminder = "<system-reminder>\nUse ToolSearch to load:\nMonitor\nGlob\n</system-reminder>";
    let mut body = json!({
        "tools": [],
        "messages": [ { "role": "user", "content": [ { "type": "text", "text": reminder } ] } ]
    });
    t.transform(&mut body, &main_ctx()).unwrap();
    let out = body["messages"][0]["content"][0]["text"].as_str().unwrap();
    assert!(!out.contains("Monitor"));
    assert!(out.contains("Glob"));
}

#[test]
fn drop_tools_does_not_touch_reminders_without_deferred_or_toolsearch() {
    let t = PinoTransform {
        settings: drop_settings(&["NotebookEdit"]),
    };
    let reminder = "<system-reminder>\nUnrelated note.\nNotebookEdit\n</system-reminder>";
    let mut body = json!({
        "tools": [],
        "messages": [ { "role": "user", "content": reminder } ]
    });
    t.transform(&mut body, &main_ctx()).unwrap();
    // No "deferred tools"/"ToolSearch" marker => block left verbatim, name stays.
    assert_eq!(body["messages"][0]["content"], json!(reminder));
}

#[test]
fn drop_tools_scrubs_only_inside_reminder_not_surrounding_prose() {
    let t = PinoTransform {
        settings: drop_settings(&["NotebookEdit"]),
    };
    let text = "Keep NotebookEdit here.\n<system-reminder>\ndeferred tools:\nNotebookEdit\n</system-reminder>\nNotebookEdit after.";
    let mut body = json!({
        "tools": [],
        "messages": [ { "role": "user", "content": text } ]
    });
    t.transform(&mut body, &main_ctx()).unwrap();
    let out = body["messages"][0]["content"].as_str().unwrap();
    assert!(out.starts_with("Keep NotebookEdit here."));
    assert!(out.trim_end().ends_with("NotebookEdit after."));
    let reminder_inner = out
        .split("<system-reminder>")
        .nth(1)
        .unwrap()
        .split("</system-reminder>")
        .next()
        .unwrap();
    assert!(!reminder_inner.contains("NotebookEdit"));
}

// --- Finding 4: byte-faithfulness of the capture-rebuild on edge inner content.
// Each asserts the EXACT output a faithful port of the Node replace must produce.

#[test]
fn drop_tools_reminder_rebuild_preserves_crlf_line_endings() {
    // Node splits on "\n"; a CRLF body keeps the trailing '\r' on each kept line.
    let t = PinoTransform {
        settings: drop_settings(&["Drop"]),
    };
    let reminder = "<system-reminder>\r\ndeferred tools:\r\nDrop\r\nKeep\r\n</system-reminder>";
    let mut body = json!({ "tools": [], "messages": [ { "role": "user", "content": reminder } ] });
    t.transform(&mut body, &main_ctx()).unwrap();
    // "Drop\r".trim() == "Drop" -> dropped; every other "\r"-suffixed line kept verbatim.
    let expected = "<system-reminder>\r\ndeferred tools:\r\nKeep\r\n</system-reminder>";
    assert_eq!(body["messages"][0]["content"], json!(expected));
}

#[test]
fn drop_tools_reminder_rebuild_preserves_embedded_angle_brackets() {
    // Inner text containing '<' / '>' that is NOT the closing tag must survive.
    let t = PinoTransform {
        settings: drop_settings(&["Drop"]),
    };
    let reminder = "<system-reminder>\ndeferred tools:\nuse <T> generics\nDrop\n</system-reminder>";
    let mut body = json!({ "tools": [], "messages": [ { "role": "user", "content": reminder } ] });
    t.transform(&mut body, &main_ctx()).unwrap();
    let expected = "<system-reminder>\ndeferred tools:\nuse <T> generics\n</system-reminder>";
    assert_eq!(body["messages"][0]["content"], json!(expected));
}

#[test]
fn drop_tools_reminder_rebuild_preserves_blank_and_whitespace_lines() {
    // Blank lines and a whitespace-only line: "  ".trim() == "" != any drop name => kept.
    let t = PinoTransform {
        settings: drop_settings(&["Drop"]),
    };
    let reminder = "<system-reminder>\ndeferred tools:\n\nDrop\n  \nKeep\n</system-reminder>";
    let mut body = json!({ "tools": [], "messages": [ { "role": "user", "content": reminder } ] });
    t.transform(&mut body, &main_ctx()).unwrap();
    let expected = "<system-reminder>\ndeferred tools:\n\n  \nKeep\n</system-reminder>";
    assert_eq!(body["messages"][0]["content"], json!(expected));
}

#[test]
fn drop_tools_reminder_line_match_is_exact_trim_not_substring() {
    // "NotebookEdit" must drop only a line equal to it after trim; a line that
    // merely CONTAINS it survives (Node DROP_TOOLS.has(line.trim()) == Set membership).
    let t = PinoTransform {
        settings: drop_settings(&["NotebookEdit"]),
    };
    let reminder =
        "<system-reminder>\ndeferred tools:\nNotebookEdit\nNotebookEditExtra\n  NotebookEdit  \n</system-reminder>";
    let mut body = json!({ "tools": [], "messages": [ { "role": "user", "content": reminder } ] });
    t.transform(&mut body, &main_ctx()).unwrap();
    let out = body["messages"][0]["content"].as_str().unwrap();
    // The bare line and the whitespace-padded line drop; "NotebookEditExtra" stays.
    assert!(
        out.contains("NotebookEditExtra"),
        "substring-different line must survive"
    );
    let expected = "<system-reminder>\ndeferred tools:\nNotebookEditExtra\n</system-reminder>";
    assert_eq!(body["messages"][0]["content"], json!(expected));
}

// M4.5b ===== restructureV123: message-restructuring pass (R19, full parity).
// Ported verbatim from reference/pino/src/transforms/default.js lines 126-208.

fn restructure_settings() -> PinoSettings {
    // Only restructure runs (drop_tools empty, strip_ansi off, auto_cache off,
    // no model override) so these tests isolate restructureV123.
    PinoSettings {
        auto_cache: false,
        main_ttl: CacheTtl::OneHour,
        sub_ttl: CacheTtl::FiveMin,
        drop_tools: vec![],
        strip_ansi: false,
        model_override: None,
    }
}

#[test]
fn restructure_noop_for_single_message() {
    // length < 2 => early return, including no string->array normalization.
    let t = PinoTransform {
        settings: restructure_settings(),
    };
    let original = json!({
        "messages": [ { "role": "user", "content": "ToolSearch hint, single turn" } ]
    });
    let mut body = original.clone();
    t.transform(&mut body, &main_ctx()).unwrap();
    assert_eq!(body, original, "single-message body must be untouched by restructure");
}

#[test]
fn restructure_normalizes_string_content_to_arrays() {
    let t = PinoTransform {
        settings: restructure_settings(),
    };
    let mut body = json!({
        "messages": [
            { "role": "user", "content": "first turn" },
            { "role": "assistant", "content": "second turn" }
        ]
    });
    t.transform(&mut body, &main_ctx()).unwrap();
    assert_eq!(
        body["messages"][0]["content"],
        json!([{ "type": "text", "text": "first turn" }])
    );
    assert_eq!(
        body["messages"][1]["content"],
        json!([{ "type": "text", "text": "second turn" }])
    );
}

#[test]
fn restructure_extracts_core_context_into_msg0_and_sets_role_user() {
    let t = PinoTransform {
        settings: restructure_settings(),
    };
    let mut body = json!({
        "messages": [
            { "role": "user", "content": [ { "type": "text", "text": "plain msg0 prose" } ] },
            { "role": "assistant", "content": [ { "type": "text", "text": "ok" } ] },
            { "role": "user", "content": [
                { "type": "text", "text": "context with claudeMd path here" },
                { "type": "text", "text": "normal latest turn" }
            ] }
        ]
    });
    t.transform(&mut body, &main_ctx()).unwrap();
    // Core block moved to the FRONT of msg0; msg0 prose retained after it; role coerced.
    assert_eq!(body["messages"][0]["role"], json!("user"));
    assert_eq!(
        body["messages"][0]["content"],
        json!([
            { "type": "text", "text": "context with claudeMd path here" },
            { "type": "text", "text": "plain msg0 prose" }
        ])
    );
    // The core block is GONE from the tail message (extracted), normal turn remains.
    assert_eq!(
        body["messages"][2]["content"],
        json!([ { "type": "text", "text": "normal latest turn" } ])
    );
}

#[test]
fn restructure_dedupes_core_blocks_by_text_first_occurrence_wins() {
    let t = PinoTransform {
        settings: restructure_settings(),
    };
    let mut body = json!({
        "messages": [
            { "role": "user", "content": [ { "type": "text", "text": "msg0" } ] },
            { "role": "user", "content": [ { "type": "text", "text": "ToolSearch catalog A" } ] },
            { "role": "assistant", "content": [ { "type": "text", "text": "mid" } ] },
            { "role": "user", "content": [
                { "type": "text", "text": "ToolSearch catalog A" },
                { "type": "text", "text": "tail prose" }
            ] }
        ]
    });
    t.transform(&mut body, &main_ctx()).unwrap();
    // Only ONE copy of the duplicate core block, prepended to msg0 before its prose.
    assert_eq!(
        body["messages"][0]["content"],
        json!([
            { "type": "text", "text": "ToolSearch catalog A" },
            { "type": "text", "text": "msg0" }
        ])
    );
    // The message that became empty after extraction (index 1, only the core block)
    // is pruned. Remaining: msg0, the assistant "mid", and the tail.
    let roles: Vec<&str> = body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["role"].as_str().unwrap())
        .collect();
    assert_eq!(roles, vec!["user", "assistant", "user"]);
    // Tail kept only its non-core block.
    let last = body["messages"].as_array().unwrap().len() - 1;
    assert_eq!(
        body["messages"][last]["content"],
        json!([ { "type": "text", "text": "tail prose" } ])
    );
}

#[test]
fn restructure_removes_stale_scaffolding_from_history_but_not_tail() {
    let t = PinoTransform {
        settings: restructure_settings(),
    };
    let mut body = json!({
        "messages": [
            { "role": "user", "content": [
                { "type": "text", "text": "<system-reminder>stale in history</system-reminder>" },
                { "type": "text", "text": "kept history prose" }
            ] },
            { "role": "user", "content": [
                { "type": "text", "text": "<system-reminder>stale-looking but in TAIL</system-reminder>" }
            ] }
        ]
    });
    t.transform(&mut body, &main_ctx()).unwrap();
    // History stale-removable block dropped; non-stale prose kept.
    assert_eq!(
        body["messages"][0]["content"],
        json!([ { "type": "text", "text": "kept history prose" } ])
    );
    // Tail's stale-LOOKING block is preserved (isTail short-circuits removal).
    assert_eq!(
        body["messages"][1]["content"],
        json!([ { "type": "text", "text": "<system-reminder>stale-looking but in TAIL</system-reminder>" } ])
    );
}

#[test]
fn restructure_local_command_text_is_not_core_context() {
    // isCoreContext returns FALSE when the block contains <local-command-stdout>
    // or <local-command-caveat>, even if it also contains a core marker.
    let t = PinoTransform {
        settings: restructure_settings(),
    };
    let mut body = json!({
        "messages": [
            { "role": "user", "content": [ { "type": "text", "text": "msg0" } ] },
            { "role": "user", "content": [
                { "type": "text", "text": "<local-command-stdout>ToolSearch mentioned</local-command-stdout>" },
                { "type": "text", "text": "tail" }
            ] }
        ]
    });
    t.transform(&mut body, &main_ctx()).unwrap();
    // The local-command block is NOT core (so not extracted to msg0), and since it
    // is the TAIL it is also NOT stale-removed: it stays in the tail message.
    assert_eq!(
        body["messages"][0]["content"],
        json!([ { "type": "text", "text": "msg0" } ])
    );
    assert_eq!(
        body["messages"][1]["content"],
        json!([
            { "type": "text", "text": "<local-command-stdout>ToolSearch mentioned</local-command-stdout>" },
            { "type": "text", "text": "tail" }
        ])
    );
}

#[test]
fn restructure_prunes_emptied_messages() {
    let t = PinoTransform {
        settings: restructure_settings(),
    };
    let mut body = json!({
        "messages": [
            { "role": "user", "content": [ { "type": "text", "text": "msg0" } ] },
            // This entire message is stale scaffolding -> emptied -> pruned.
            { "role": "user", "content": [
                { "type": "text", "text": "<command-name>foo</command-name>" }
            ] },
            { "role": "user", "content": [ { "type": "text", "text": "tail" } ] }
        ]
    });
    t.transform(&mut body, &main_ctx()).unwrap();
    let texts: Vec<&str> = body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["content"][0]["text"].as_str().unwrap())
        .collect();
    assert_eq!(texts, vec!["msg0", "tail"], "emptied middle message pruned");
}

#[test]
fn restructure_preserves_non_text_blocks() {
    // tool_use / tool_result / image blocks (no string `text`) are always kept.
    let t = PinoTransform {
        settings: restructure_settings(),
    };
    let mut body = json!({
        "messages": [
            { "role": "user", "content": [ { "type": "text", "text": "msg0" } ] },
            { "role": "assistant", "content": [
                { "type": "tool_use", "id": "tu1", "name": "Bash", "input": {} }
            ] },
            { "role": "user", "content": [
                { "type": "tool_result", "tool_use_id": "tu1", "content": "out" }
            ] }
        ]
    });
    let original = body.clone();
    t.transform(&mut body, &main_ctx()).unwrap();
    // No core blocks, no stale-removable text blocks => only string->array no-op
    // (already arrays) and no pruning. Body is unchanged.
    assert_eq!(body, original);
}

#[test]
fn restructure_no_core_blocks_does_not_force_msg0_role() {
    // When coreBlocks is empty, Node does NOT reassemble msg0 or set role=user.
    let t = PinoTransform {
        settings: restructure_settings(),
    };
    let mut body = json!({
        "messages": [
            { "role": "assistant", "content": [ { "type": "text", "text": "no core here" } ] },
            { "role": "user", "content": [ { "type": "text", "text": "tail" } ] }
        ]
    });
    t.transform(&mut body, &main_ctx()).unwrap();
    // Role left as the original "assistant" because no core blocks were collected.
    assert_eq!(body["messages"][0]["role"], json!("assistant"));
    assert_eq!(
        body["messages"][0]["content"],
        json!([ { "type": "text", "text": "no core here" } ])
    );
}

// M4.6 ===== cache helpers: count_cache_breakpoints, strip_intermediate_message_
// breakpoints (both pub), and strip_small_system_breakpoints (UTF-16 length).
// Ported from reference/pino/src/cache.js lines 28-96.

use super::{count_cache_breakpoints, strip_intermediate_message_breakpoints};

#[test]
fn count_cache_breakpoints_counts_every_ephemeral() {
    let body = json!({
        "tools": [ { "name": "a", "cache_control": { "type": "ephemeral", "ttl": "1h" } } ],
        "system": [
            { "type": "text", "text": "s", "cache_control": { "type": "ephemeral" } },
            { "type": "text", "text": "no cc" }
        ],
        "messages": [
            { "role": "user", "content": [
                { "type": "text", "text": "m", "cache_control": { "type": "ephemeral", "ttl": "5m" } }
            ] }
        ]
    });
    assert_eq!(count_cache_breakpoints(&body), 3);
}

#[test]
fn count_cache_breakpoints_ignores_non_ephemeral() {
    let body = json!({ "x": { "cache_control": { "type": "persistent" } } });
    assert_eq!(count_cache_breakpoints(&body), 0);
}

#[test]
fn strip_intermediate_removes_cc_from_middle_messages_only() {
    let mut body = json!({
        "messages": [
            { "role": "user",      "content": [ { "type": "text", "text": "first",  "cache_control": { "type": "ephemeral" } } ] },
            { "role": "assistant", "content": [ { "type": "text", "text": "midA",   "cache_control": { "type": "ephemeral" } } ] },
            { "role": "user",      "content": [ { "type": "text", "text": "midB",   "cache_control": { "type": "ephemeral" } } ] },
            { "role": "assistant", "content": [ { "type": "text", "text": "last",   "cache_control": { "type": "ephemeral" } } ] }
        ]
    });
    let stripped = strip_intermediate_message_breakpoints(&mut body);
    assert_eq!(stripped, 2, "only the two middle messages are stripped");
    assert!(body["messages"][0]["content"][0].get("cache_control").is_some());
    assert!(body["messages"][3]["content"][0].get("cache_control").is_some());
    assert!(body["messages"][1]["content"][0].get("cache_control").is_none());
    assert!(body["messages"][2]["content"][0].get("cache_control").is_none());
}

#[test]
fn strip_intermediate_noop_when_two_or_fewer_messages() {
    let mut body = json!({
        "messages": [
            { "role": "user", "content": [ { "type": "text", "text": "a", "cache_control": { "type": "ephemeral" } } ] },
            { "role": "user", "content": [ { "type": "text", "text": "b", "cache_control": { "type": "ephemeral" } } ] }
        ]
    });
    let stripped = strip_intermediate_message_breakpoints(&mut body);
    assert_eq!(stripped, 0);
    assert!(body["messages"][0]["content"][0].get("cache_control").is_some());
    assert!(body["messages"][1]["content"][0].get("cache_control").is_some());
}

// --- strip_small_system_breakpoints is private; exercise it through the public
// --- injector (inject_breakpoint_if_absent lands in M4.7). For THIS task we test
// --- it directly via a thin pub(crate) test shim is unnecessary; instead these two
// --- tests assert the UTF-16 boundary behavior through count_cache_breakpoints
// --- after calling the injector — BUT the injector does not exist yet. To keep
// --- this task self-contained and the boundary locked NOW, expose the helper.

use super::strip_small_system_breakpoints;

#[test]
fn strip_small_system_removes_breakpoint_below_500_and_keeps_at_or_above() {
    let mut body = json!({
        "system": [
            { "type": "text", "text": "x".repeat(499), "cache_control": { "type": "ephemeral" } },
            { "type": "text", "text": "y".repeat(500), "cache_control": { "type": "ephemeral" } }
        ]
    });
    let stripped = strip_small_system_breakpoints(&mut body);
    assert_eq!(stripped, 1, "only the <500 block is stripped");
    assert!(
        body["system"][0].get("cache_control").is_none(),
        "499-char block stripped"
    );
    assert!(
        body["system"][1].get("cache_control").is_some(),
        "500-char block kept (len < 500 is false)"
    );
}

// R18 / Finding 5: length is measured in UTF-16 code units (Node String.length),
// NOT Unicode scalar values. An astral char ('\u{1F600}') is ONE scalar but TWO
// UTF-16 units. A string of 499 such chars is 499 scalars but 998 UTF-16 units,
// which is >= 500 => the breakpoint MUST be kept. Counting scalars would wrongly
// strip it.
#[test]
fn strip_small_system_uses_utf16_length_at_boundary() {
    let astral = "\u{1F600}"; // 1 scalar, 2 UTF-16 code units
    let mut body = json!({
        "system": [
            // 250 astral chars = 250 scalars = 500 UTF-16 units => NOT < 500 => kept.
            { "type": "text", "text": astral.repeat(250), "cache_control": { "type": "ephemeral" } },
            // 249 astral chars = 498 UTF-16 units => < 500 => stripped.
            { "type": "text", "text": astral.repeat(249), "cache_control": { "type": "ephemeral" } }
        ]
    });
    let stripped = strip_small_system_breakpoints(&mut body);
    assert_eq!(stripped, 1, "only the 498-UTF-16-unit block is stripped");
    assert!(
        body["system"][0].get("cache_control").is_some(),
        "500-UTF-16-unit block kept; scalar-counting (250) would wrongly strip it"
    );
    assert!(body["system"][1].get("cache_control").is_none());
}

// M4.7 ===== breakpoint injection within the 4-cap (tools, system, msg[0], tail).
// Ported from reference/pino/src/cache.js lines 98-181.

use super::inject_breakpoint_if_absent;

fn cc_of(block: &serde_json::Value) -> Option<&serde_json::Value> {
    block.get("cache_control")
}

#[test]
fn inject_places_tools_system_msg0_tail_within_cap() {
    let mut body = json!({
        "tools": [ { "name": "Bash" }, { "name": "Read" } ],
        "system": [
            { "type": "text", "text": "x".repeat(600) },
            { "type": "text", "text": "y".repeat(600) }
        ],
        "messages": [
            { "role": "user", "content": [
                { "type": "text", "text": "z".repeat(50) },
                { "type": "text", "text": "reminders ".repeat(60) }
            ] },
            { "role": "assistant", "content": [ { "type": "text", "text": "ok" } ] },
            { "role": "user", "content": [ { "type": "text", "text": "latest" } ] }
        ]
    });
    let tail_paths = inject_breakpoint_if_absent(&mut body, CacheTtl::FiveMin);

    // Uniform TTL: every injected slot carries the passed ttl (5m).
    let tools = body["tools"].as_array().unwrap();
    assert_eq!(cc_of(&tools[1]).unwrap(), &json!({ "type": "ephemeral", "ttl": "5m" }));
    assert!(cc_of(&tools[0]).is_none());

    let system = body["system"].as_array().unwrap();
    assert_eq!(cc_of(&system[1]).unwrap(), &json!({ "type": "ephemeral", "ttl": "5m" }));
    assert!(cc_of(&system[0]).is_none());

    let msg0 = body["messages"][0]["content"].as_array().unwrap();
    assert_eq!(cc_of(&msg0[1]).unwrap(), &json!({ "type": "ephemeral", "ttl": "5m" }));

    let last_block = &body["messages"][2]["content"][0];
    assert_eq!(cc_of(last_block).unwrap(), &json!({ "type": "ephemeral", "ttl": "5m" }));
    assert_eq!(tail_paths, vec!["/messages/2/content/0".to_string()]);

    assert_eq!(count_cache_breakpoints(&body), 4);
}

#[test]
fn inject_skips_tools_when_already_has_breakpoint() {
    let mut body = json!({
        "tools": [ { "name": "Bash", "cache_control": { "type": "ephemeral", "ttl": "5m" } }, { "name": "Read" } ],
        "messages": [ { "role": "user", "content": [ { "type": "text", "text": "hi" } ] } ]
    });
    inject_breakpoint_if_absent(&mut body, CacheTtl::FiveMin);
    assert!(cc_of(&body["tools"][1]).is_none());
}

#[test]
fn inject_converts_string_system_to_cached_array() {
    let mut body = json!({
        "system": "you are a helpful assistant",
        "messages": [ { "role": "user", "content": [ { "type": "text", "text": "hi" } ] } ]
    });
    inject_breakpoint_if_absent(&mut body, CacheTtl::FiveMin);
    assert_eq!(
        body["system"],
        json!([ { "type": "text", "text": "you are a helpful assistant",
                  "cache_control": { "type": "ephemeral", "ttl": "5m" } } ])
    );
}

#[test]
fn inject_skips_msg0_breakpoint_when_single_message() {
    // Only one message => msg0 dedicated breakpoint NOT placed; tail covers it.
    let mut body = json!({
        "messages": [ { "role": "user", "content": [ { "type": "text", "text": "only" } ] } ]
    });
    let tail_paths = inject_breakpoint_if_absent(&mut body, CacheTtl::OneHour);
    let blocks = body["messages"][0]["content"].as_array().unwrap();
    assert_eq!(count_cache_breakpoints(&body), 1);
    assert_eq!(cc_of(&blocks[0]).unwrap(), &json!({ "type": "ephemeral", "ttl": "1h" }));
    assert_eq!(tail_paths, vec!["/messages/0/content/0".to_string()]);
}

#[test]
fn inject_normalizes_string_message_tail_into_array() {
    let mut body = json!({
        "messages": [ { "role": "user", "content": "plain string turn" } ]
    });
    inject_breakpoint_if_absent(&mut body, CacheTtl::FiveMin);
    assert_eq!(
        body["messages"][0]["content"],
        json!([ { "type": "text", "text": "plain string turn",
                  "cache_control": { "type": "ephemeral", "ttl": "5m" } } ])
    );
}

#[test]
fn inject_strips_small_system_breakpoint_then_reuses_slot() {
    let mut body = json!({
        "system": [ { "type": "text", "text": "tiny", "cache_control": { "type": "ephemeral" } } ],
        "messages": [ { "role": "user", "content": [ { "type": "text", "text": "hi" } ] } ]
    });
    inject_breakpoint_if_absent(&mut body, CacheTtl::FiveMin);
    assert_eq!(
        cc_of(&body["system"][0]).unwrap(),
        &json!({ "type": "ephemeral", "ttl": "5m" })
    );
}

#[test]
fn inject_respects_cap_when_four_breakpoints_already_present() {
    let mut body = json!({
        "tools": [ { "name": "A", "cache_control": { "type": "ephemeral" } } ],
        "system": [ { "type": "text", "text": "s".repeat(600), "cache_control": { "type": "ephemeral" } } ],
        "messages": [
            { "role": "user", "content": [ { "type": "text", "text": "m0", "cache_control": { "type": "ephemeral" } } ] },
            { "role": "user", "content": [ { "type": "text", "text": "m1", "cache_control": { "type": "ephemeral" } } ] }
        ]
    });
    let tail_paths = inject_breakpoint_if_absent(&mut body, CacheTtl::FiveMin);
    assert_eq!(count_cache_breakpoints(&body), 4, "cap not exceeded");
    assert!(tail_paths.is_empty(), "no new tail injected at the cap");
}

// Review Finding 6: len>1 but ONLY msg0 is cacheable. msg0 gets its 1h breakpoint;
// the tail pass selects the SAME block but its `!cache_control` guard short-circuits,
// so NO 5m tail is added and tail_paths is empty (matching Node's `!tail.cache_control`).
#[test]
fn inject_len_gt_one_only_msg0_cacheable_no_duplicate_tail() {
    let mut body = json!({
        "messages": [
            { "role": "user", "content": [ { "type": "text", "text": "the only cacheable block" } ] },
            // Second message has NO text/tool_result/image block => not cacheable.
            { "role": "assistant", "content": [ { "type": "tool_use", "id": "t", "name": "X", "input": {} } ] }
        ]
    });
    let tail_paths = inject_breakpoint_if_absent(&mut body, CacheTtl::FiveMin);
    // msg0 block keeps its 5m (msg0 pass), tail pass adds nothing.
    assert_eq!(
        cc_of(&body["messages"][0]["content"][0]).unwrap(),
        &json!({ "type": "ephemeral", "ttl": "5m" })
    );
    assert_eq!(count_cache_breakpoints(&body), 1, "exactly one breakpoint (msg0)");
    assert!(
        tail_paths.is_empty(),
        "no extra tail added when the only cacheable block is msg0's"
    );
}

// M4.8 ===== tail normalization (client-placed tail breakpoints) + 1h rewrite
// with a path skip-set. Ports normalizeTailBreakpoints (cache.js 44-59) and
// rewriteCacheControl (cache.js 3-26).

use super::{normalize_tail_breakpoints, rewrite_cache_control};
use std::collections::HashSet;

#[test]
fn normalize_tail_forces_last_message_breakpoints_to_ttl() {
    let mut body = json!({
        "messages": [
            { "role": "user", "content": [ { "type": "text", "text": "a", "cache_control": { "type": "ephemeral", "ttl": "1h" } } ] },
            { "role": "user", "content": [
                { "type": "text", "text": "b", "cache_control": { "type": "ephemeral", "ttl": "1h" } },
                { "type": "tool_result", "content": "c", "cache_control": { "type": "ephemeral" } }
            ] }
        ]
    });
    let paths = normalize_tail_breakpoints(&mut body, CacheTtl::FiveMin);
    assert_eq!(body["messages"][1]["content"][0]["cache_control"]["ttl"], json!("5m"));
    assert_eq!(body["messages"][1]["content"][1]["cache_control"]["ttl"], json!("5m"));
    assert_eq!(body["messages"][0]["content"][0]["cache_control"]["ttl"], json!("1h"));
    let set: HashSet<String> = paths.into_iter().collect();
    assert!(set.contains("/messages/1/content/0"));
    assert!(set.contains("/messages/1/content/1"));
    assert_eq!(set.len(), 2);
}

#[test]
fn normalize_tail_empty_when_no_messages() {
    let mut body = json!({ "messages": [] });
    let paths = normalize_tail_breakpoints(&mut body, CacheTtl::OneHour);
    assert!(paths.is_empty());
}

#[test]
fn normalize_tail_records_block_path_not_cache_control_path() {
    // The recorded path must be the BLOCK's pointer (e.g. /messages/0/content/0),
    // NOT /messages/0/content/0/cache_control. This is what rewrite skips.
    let mut body = json!({
        "messages": [
            { "role": "user", "content": [ { "type": "text", "text": "only", "cache_control": { "type": "ephemeral", "ttl": "1h" } } ] }
        ]
    });
    let paths = normalize_tail_breakpoints(&mut body, CacheTtl::FiveMin);
    assert_eq!(paths, vec!["/messages/0/content/0".to_string()]);
}

#[test]
fn rewrite_bumps_every_ephemeral_to_ttl_except_skip() {
    let mut body = json!({
        "tools": [ { "name": "x", "cache_control": { "type": "ephemeral", "ttl": "5m" } } ],
        "system": [ { "type": "text", "text": "s", "cache_control": { "type": "ephemeral" } } ],
        "messages": [
            { "role": "user", "content": [
                { "type": "text", "text": "tail", "cache_control": { "type": "ephemeral", "ttl": "5m" } }
            ] }
        ]
    });
    let mut skip: HashSet<String> = HashSet::new();
    skip.insert("/messages/0/content/0".to_string());
    rewrite_cache_control(&mut body, &skip, CacheTtl::OneHour);
    assert_eq!(body["tools"][0]["cache_control"]["ttl"], json!("1h"));
    assert_eq!(body["system"][0]["cache_control"]["ttl"], json!("1h"));
    assert_eq!(body["messages"][0]["content"][0]["cache_control"]["ttl"], json!("5m"));
}

#[test]
fn rewrite_leaves_non_ephemeral_alone() {
    let mut body = json!({
        "x": { "cache_control": { "type": "persistent", "ttl": "5m" } }
    });
    rewrite_cache_control(&mut body, &HashSet::new(), CacheTtl::OneHour);
    assert_eq!(body["x"]["cache_control"]["ttl"], json!("5m"));
}

// M4.9 ===== apply_auto_cache pipeline (strip-intermediate -> inject -> normalize-tail
// ===== -> rewrite). End-to-end through PinoTransform::transform (server.js 88-98).

fn auto_cache_settings(main: CacheTtl, sub: CacheTtl) -> PinoSettings {
    PinoSettings {
        auto_cache: true,
        main_ttl: main,
        sub_ttl: sub,
        drop_tools: vec![],
        strip_ansi: false,
        model_override: None,
    }
}

// M4.9b ===== per-agent uniform TTL: the selected ttl (main vs sub by ctx) is
// applied to EVERY cache slot, not just the tail.

fn main_sub_settings() -> PinoSettings {
    PinoSettings {
        auto_cache: true,
        main_ttl: CacheTtl::OneHour,
        sub_ttl: CacheTtl::FiveMin,
        drop_tools: vec![],
        strip_ansi: false,
        model_override: None,
    }
}

fn ttl_at(body: &serde_json::Value, ptr: &str) -> String {
    body.pointer(ptr)
        .and_then(|b| b.get("cache_control"))
        .and_then(|cc| cc.get("ttl"))
        .and_then(|t| t.as_str())
        .unwrap_or("<none>")
        .to_string()
}

#[test]
fn main_request_applies_main_ttl_to_every_slot() {
    let t = PinoTransform {
        settings: main_sub_settings(),
    };
    let mut body = json!({
        "tools": [ { "name": "Bash" }, { "name": "Read" } ],
        "system": [ { "type": "text", "text": "s".repeat(600) } ],
        "messages": [
            { "role": "user", "content": [ { "type": "text", "text": "r".repeat(600) } ] },
            { "role": "assistant", "content": [ { "type": "text", "text": "mid" } ] },
            { "role": "user", "content": [ { "type": "text", "text": "tail turn" } ] }
        ]
    });
    t.transform(&mut body, &main_ctx()).unwrap();
    assert_eq!(count_cache_breakpoints(&body), 4);
    assert_eq!(ttl_at(&body, "/tools/1"), "1h");
    assert_eq!(ttl_at(&body, "/system/0"), "1h");
    assert_eq!(ttl_at(&body, "/messages/0/content/0"), "1h");
    assert_eq!(ttl_at(&body, "/messages/2/content/0"), "1h"); // tail also 1h (uniform)
}

#[test]
fn subagent_request_applies_sub_ttl_to_every_slot() {
    let t = PinoTransform {
        settings: main_sub_settings(),
    };
    let mut body = json!({
        "tools": [ { "name": "Bash" }, { "name": "Read" } ],
        "system": [ { "type": "text", "text": "s".repeat(600) } ],
        "messages": [
            { "role": "user", "content": [ { "type": "text", "text": "r".repeat(600) } ] },
            { "role": "assistant", "content": [ { "type": "text", "text": "mid" } ] },
            { "role": "user", "content": [ { "type": "text", "text": "tail turn" } ] }
        ]
    });
    t.transform(&mut body, &sub_ctx()).unwrap();
    assert_eq!(count_cache_breakpoints(&body), 4);
    assert_eq!(ttl_at(&body, "/tools/1"), "5m"); // head slot is 5m for a subagent
    assert_eq!(ttl_at(&body, "/system/0"), "5m");
    assert_eq!(ttl_at(&body, "/messages/0/content/0"), "5m");
    assert_eq!(ttl_at(&body, "/messages/2/content/0"), "5m");
}

#[test]
fn settings_values_are_honored_per_agent_type() {
    // Non-default: main=5m, sub=1h. Proves the value comes from settings, not a constant.
    let t = PinoTransform {
        settings: PinoSettings {
            auto_cache: true,
            main_ttl: CacheTtl::FiveMin,
            sub_ttl: CacheTtl::OneHour,
            drop_tools: vec![],
            strip_ansi: false,
            model_override: None,
        },
    };
    let body0 = json!({
        "system": [ { "type": "text", "text": "s".repeat(600) } ],
        "messages": [ { "role": "user", "content": [ { "type": "text", "text": "hi" } ] } ]
    });
    let mut bmain = body0.clone();
    t.transform(&mut bmain, &main_ctx()).unwrap();
    assert_eq!(ttl_at(&bmain, "/system/0"), "5m");

    let mut bsub = body0.clone();
    t.transform(&mut bsub, &sub_ctx()).unwrap();
    assert_eq!(ttl_at(&bsub, "/system/0"), "1h");
}

#[test]
fn auto_cache_caps_at_four_and_strips_intermediate() {
    let t = PinoTransform {
        settings: auto_cache_settings(CacheTtl::OneHour, CacheTtl::FiveMin),
    };
    let mut body = json!({
        "tools": [ { "name": "Bash" }, { "name": "Read" } ],
        "system": [ { "type": "text", "text": "s".repeat(600) } ],
        "messages": [
            { "role": "user", "content": [ { "type": "text", "text": "r".repeat(600) } ] },
            { "role": "assistant", "content": [ { "type": "text", "text": "mid", "cache_control": { "type": "ephemeral", "ttl": "5m" } } ] },
            { "role": "user", "content": [ { "type": "text", "text": "tail turn" } ] }
        ]
    });
    t.transform(&mut body, &main_ctx()).unwrap();

    assert_eq!(count_cache_breakpoints(&body), 4);
    assert!(
        body["messages"][1]["content"][0].get("cache_control").is_none(),
        "intermediate breakpoint stripped"
    );
    // Uniform under main: every slot, including the tail, carries main_ttl (1h).
    assert_eq!(body["tools"][1]["cache_control"]["ttl"], json!("1h"));
    assert_eq!(body["system"][0]["cache_control"]["ttl"], json!("1h"));
    assert_eq!(body["messages"][0]["content"][0]["cache_control"]["ttl"], json!("1h"));
    assert_eq!(body["messages"][2]["content"][0]["cache_control"]["ttl"], json!("1h"));
}

#[test]
fn auto_cache_main_rewrites_every_ephemeral_to_main_ttl() {
    let t = PinoTransform {
        settings: auto_cache_settings(CacheTtl::OneHour, CacheTtl::FiveMin),
    };
    let mut body = json!({
        "system": [ { "type": "text", "text": "s".repeat(600), "cache_control": { "type": "ephemeral" } } ],
        "messages": [
            { "role": "user", "content": [ { "type": "text", "text": "first" } ] },
            { "role": "user", "content": [ { "type": "text", "text": "client tail", "cache_control": { "type": "ephemeral", "ttl": "1h" } } ] }
        ]
    });
    t.transform(&mut body, &main_ctx()).unwrap();
    // Uniform TTL: the client tail is no longer special-cased to a different value.
    assert_eq!(body["system"][0]["cache_control"]["ttl"], json!("1h"));
    assert_eq!(body["messages"][1]["content"][0]["cache_control"]["ttl"], json!("1h"));
}

#[test]
fn auto_cache_main_single_turn_uses_main_ttl() {
    let t = PinoTransform {
        settings: auto_cache_settings(CacheTtl::OneHour, CacheTtl::FiveMin),
    };
    let mut body = json!({
        "messages": [ { "role": "user", "content": [ { "type": "text", "text": "only turn" } ] } ]
    });
    t.transform(&mut body, &main_ctx()).unwrap();
    assert_eq!(body["messages"][0]["content"][0]["cache_control"]["ttl"], json!("1h"));
    assert_eq!(count_cache_breakpoints(&body), 1);
}

#[test]
fn auto_cache_disabled_does_not_inject_any_breakpoint() {
    let t = PinoTransform {
        settings: PinoSettings {
            auto_cache: false,
            main_ttl: CacheTtl::OneHour,
            sub_ttl: CacheTtl::FiveMin,
            drop_tools: vec![],
            strip_ansi: false,
            model_override: None,
        },
    };
    let mut body = json!({
        "tools": [ { "name": "Bash" } ],
        "system": [ { "type": "text", "text": "s".repeat(600) } ],
        "messages": [ { "role": "user", "content": [ { "type": "text", "text": "hi" } ] } ]
    });
    t.transform(&mut body, &main_ctx()).unwrap();
    assert_eq!(count_cache_breakpoints(&body), 0);
}

// Finding 1/8: prove restructureV123 runs BEFORE injection inside transform and the
// cache breakpoint lands on the RESTRUCTURED layout. A core-context block in the tail
// is moved to msg0 by restructure, then the msg0 cache pass targets that moved block.
#[test]
fn auto_cache_targets_restructured_layout_not_pre_restructure() {
    let t = PinoTransform {
        settings: auto_cache_settings(CacheTtl::OneHour, CacheTtl::FiveMin),
    };
    let mut body = json!({
        "messages": [
            { "role": "user", "content": [ { "type": "text", "text": "msg0 prose" } ] },
            { "role": "assistant", "content": [ { "type": "text", "text": "mid turn" } ] },
            { "role": "user", "content": [
                { "type": "text", "text": "claudeMd core context block" },
                { "type": "text", "text": "latest user turn" }
            ] }
        ]
    });
    t.transform(&mut body, &main_ctx()).unwrap();
    // After restructure: msg0 = [core, "msg0 prose"]; tail (now index 2) = ["latest user turn"].
    assert_eq!(
        body["messages"][0]["content"][0]["text"],
        json!("claudeMd core context block")
    );
    // Uniform under main: msg0 dedicated breakpoint and the tail are both main_ttl (1h).
    assert_eq!(body["messages"][0]["content"][1]["cache_control"]["ttl"], json!("1h"));
    assert_eq!(body["messages"][2]["content"][0]["cache_control"]["ttl"], json!("1h"));
    assert_eq!(count_cache_breakpoints(&body), 2);
}

// M4.10 ===== ensure_beta_header (multi-value safe) + apply_headers R6 hook.

use super::{ensure_beta_header, BetaHeaderStatus, BETA_FLAG};
use http::header::HeaderValue;
use http::HeaderMap;

#[test]
fn beta_header_added_when_absent() {
    let mut headers = HeaderMap::new();
    let status = ensure_beta_header(&mut headers);
    assert_eq!(status, BetaHeaderStatus::Added);
    assert_eq!(headers.get("anthropic-beta").unwrap().to_str().unwrap(), BETA_FLAG);
}

#[test]
fn beta_header_present_when_flag_already_listed() {
    let mut headers = HeaderMap::new();
    headers.insert(
        "anthropic-beta",
        HeaderValue::from_str(&format!("token-efficient-tools-2024-11-01, {}", BETA_FLAG)).unwrap(),
    );
    let status = ensure_beta_header(&mut headers);
    assert_eq!(status, BetaHeaderStatus::Present);
    let v = headers.get("anthropic-beta").unwrap().to_str().unwrap();
    assert!(v.contains(BETA_FLAG));
    assert_eq!(v.matches(BETA_FLAG).count(), 1);
}

#[test]
fn beta_header_appended_when_other_flags_present() {
    let mut headers = HeaderMap::new();
    headers.insert(
        "anthropic-beta",
        HeaderValue::from_static("token-efficient-tools-2024-11-01"),
    );
    let status = ensure_beta_header(&mut headers);
    assert_eq!(status, BetaHeaderStatus::Appended);
    assert_eq!(
        headers.get("anthropic-beta").unwrap().to_str().unwrap(),
        format!("token-efficient-tools-2024-11-01,{}", BETA_FLAG)
    );
}

// Finding 9: a HeaderMap can carry MULTIPLE anthropic-beta lines. The flag may live
// in the SECOND value; merging across all values must detect it (status Present) and
// collapse to a single canonical value that still carries the flag exactly once.
#[test]
fn beta_header_present_when_flag_in_a_second_value() {
    let mut headers = HeaderMap::new();
    headers.append(
        "anthropic-beta",
        HeaderValue::from_static("token-efficient-tools-2024-11-01"),
    );
    headers.append("anthropic-beta", HeaderValue::from_static(BETA_FLAG));
    let status = ensure_beta_header(&mut headers);
    assert_eq!(status, BetaHeaderStatus::Present);
    // Collapsed to a single value; flag preserved exactly once; the other flag retained.
    assert_eq!(headers.get_all("anthropic-beta").iter().count(), 1);
    let v = headers.get("anthropic-beta").unwrap().to_str().unwrap();
    assert_eq!(v.matches(BETA_FLAG).count(), 1);
    assert!(v.contains("token-efficient-tools-2024-11-01"));
}

#[test]
fn beta_header_appended_across_multiple_values_without_dropping_any() {
    let mut headers = HeaderMap::new();
    headers.append("anthropic-beta", HeaderValue::from_static("flag-a"));
    headers.append("anthropic-beta", HeaderValue::from_static("flag-b"));
    let status = ensure_beta_header(&mut headers);
    assert_eq!(status, BetaHeaderStatus::Appended);
    assert_eq!(headers.get_all("anthropic-beta").iter().count(), 1);
    let v = headers.get("anthropic-beta").unwrap().to_str().unwrap();
    assert!(v.contains("flag-a"));
    assert!(v.contains("flag-b"));
    assert!(v.contains(BETA_FLAG));
}

// apply_headers wiring: pino applies the beta header only when auto_cache is on.
#[test]
fn apply_headers_sets_beta_when_auto_cache_on() {
    let t = PinoTransform {
        settings: PinoSettings {
            auto_cache: true,
            main_ttl: CacheTtl::OneHour,
            sub_ttl: CacheTtl::FiveMin,
            drop_tools: vec![],
            strip_ansi: false,
            model_override: None,
        },
    };
    let mut headers = HeaderMap::new();
    t.apply_headers(&mut headers);
    assert_eq!(headers.get("anthropic-beta").unwrap().to_str().unwrap(), BETA_FLAG);
}

#[test]
fn apply_headers_noop_when_auto_cache_off() {
    let t = PinoTransform {
        settings: no_op_settings(),
    };
    let mut headers = HeaderMap::new();
    t.apply_headers(&mut headers);
    assert!(
        headers.get("anthropic-beta").is_none(),
        "no beta flag advertised when auto_cache is off"
    );
}

// M4.11 ===== full-pipeline parity guard (operation order + restructure->cache).

// LABELED INVARIANT GUARD (R12): composition of M4.3-M4.10 behavior. Not a
// red->green cycle — the underlying reds were paid per stage. Locks operation
// order and the restructure->cache interaction against regressions.
#[test]
fn full_pipeline_parity_realistic_body() {
    let t = PinoTransform {
        settings: PinoSettings {
            auto_cache: true,
            main_ttl: CacheTtl::OneHour,
            sub_ttl: CacheTtl::FiveMin,
            drop_tools: vec!["NotebookEdit".to_string()],
            strip_ansi: true,
            model_override: Some("claude-opus-4-6".to_string()),
        },
    };
    let reminder = "<system-reminder>\nThe following are deferred tools:\nNotebookEdit\nGlob\n</system-reminder>";
    let mut body = json!({
        "model": "claude-sonnet-4-5",
        "system": [
            { "type": "text", "text": format!("You are claude-opus-4-7 (Opus 4.7). {}", "x".repeat(600)) }
        ],
        "tools": [
            { "name": "Bash", "description": "run" },
            { "name": "NotebookEdit", "description": "edit nb" },
            { "name": "Read", "description": "read" }
        ],
        "messages": [
            // msg0: plain prose (no core markers). Reminder is scrubbed (deferred-tools),
            // not stale-removed here because it does not START with the tag after the
            // "ctx" prefix... so we keep msg0 free of leading-tag stale content.
            { "role": "user", "content": [ { "type": "text", "text": format!("session start. {}", "ctx ".repeat(60)) } ] },
            // history assistant turn (kept).
            { "role": "assistant", "content": [ { "type": "text", "text": "working" } ] },
            // tail: a core-context block (extracted to msg0 by restructure) + a
            // tool_result with ANSI + the scrubbable reminder.
            { "role": "user", "content": [
                { "type": "text", "text": "claudeMd: project rules here" },
                { "type": "tool_result", "content": "\u{1b}[32mdone\u{1b}[0m output" },
                { "type": "text", "text": reminder }
            ] }
        ]
    });
    t.transform(&mut body, &main_ctx()).unwrap();

    // --- model override applied + system self-ref rewritten ---
    assert_eq!(body["model"], json!("claude-opus-4-6"));
    let sys_text = body["system"][0]["text"].as_str().unwrap();
    assert!(sys_text.contains("claude-opus-4-6"));
    assert!(sys_text.contains("Opus 4.6"));
    assert!(!sys_text.contains("claude-opus-4-7"));

    // --- dropped tool absent from tools + scrubbed from the reminder (still in tail) ---
    let names: Vec<&str> = body["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["Bash", "Read"]);

    // --- restructure: core block moved to FRONT of msg0; role coerced to user ---
    assert_eq!(body["messages"][0]["role"], json!("user"));
    assert_eq!(
        body["messages"][0]["content"][0]["text"],
        json!("claudeMd: project rules here")
    );

    // --- the tail (last message) no longer holds the core block; tool_result ANSI stripped ---
    let last = body["messages"].as_array().unwrap().len() - 1;
    // The tail still contains the tool_result and the (scrubbed) reminder block.
    let tail_content = body["messages"][last]["content"].as_array().unwrap();
    // ANSI stripped on the tool_result content.
    let tr = tail_content.iter().find(|b| b["type"] == json!("tool_result")).unwrap();
    assert_eq!(tr["content"], json!("done output"));
    // Reminder scrubbed: NotebookEdit gone, Glob kept.
    let rem = tail_content
        .iter()
        .find(|b| {
            b.get("text")
                .and_then(|x| x.as_str())
                .map(|s| s.contains("system-reminder"))
                .unwrap_or(false)
        })
        .unwrap();
    let rem_text = rem["text"].as_str().unwrap();
    assert!(!rem_text.contains("NotebookEdit"));
    assert!(rem_text.contains("Glob"));

    // --- caching: 4-cap respected; uniform under main => every slot at 1h ---
    assert_eq!(count_cache_breakpoints(&body), 4);
    assert_eq!(body["tools"][1]["cache_control"]["ttl"], json!("1h")); // last remaining tool = Read
    assert_eq!(body["system"][0]["cache_control"]["ttl"], json!("1h"));
    // msg0 dedicated breakpoint at 1h (on msg0's last cacheable block).
    let msg0 = body["messages"][0]["content"].as_array().unwrap();
    let msg0_last = msg0.len() - 1;
    assert_eq!(
        body["messages"][0]["content"][msg0_last]["cache_control"]["ttl"],
        json!("1h")
    );
    // tail breakpoint also at 1h (uniform main_ttl) on the last message's last cacheable block.
    let tail_last = tail_content.len() - 1;
    assert_eq!(
        body["messages"][last]["content"][tail_last]["cache_control"]["ttl"],
        json!("1h")
    );

    // --- apply_headers emits the beta flag (engine hook) ---
    let mut headers = http::HeaderMap::new();
    t.apply_headers(&mut headers);
    assert_eq!(headers.get("anthropic-beta").unwrap().to_str().unwrap(), BETA_FLAG);
}

// FIX-B: pino byte-fidelity seam ======================================================================================
//
// pino re-serialization is acceptable (prompt-cache relies on cross-turn
// CONSISTENCY, which a stable serialize preserves), but a TRUE passthrough
// (None) is required when NO feature is active, matching reference pino which
// only mutates when AUTO_CACHE / a transform / MODEL_OVERRIDE is set.

fn all_off_settings() -> PinoSettings {
    PinoSettings {
        auto_cache: false,
        main_ttl: CacheTtl::OneHour,
        sub_ttl: CacheTtl::FiveMin,
        drop_tools: vec![],
        strip_ansi: false,
        model_override: None,
    }
}

#[test]
fn transform_bytes_all_features_off_is_true_passthrough_none() {
    let t = PinoTransform {
        settings: all_off_settings(),
    };
    // Non-canonical byte-forms: if pino round-tripped through Value it would
    // canonicalize these; None proves it never parsed/re-serialized at all.
    let raw = br#"{"model":"claude-x","max_tokens":1e1,"messages":[{"role":"user","content":"a\/b"}]}"#;
    let out = t.transform_bytes(raw, &main_ctx()).expect("all-off transform is Ok");
    assert!(
        out.is_none(),
        "all-features-off pino must be a TRUE byte passthrough (None)"
    );
}

#[test]
fn transform_bytes_auto_cache_on_returns_some_mutated() {
    let t = PinoTransform {
        settings: PinoSettings {
            auto_cache: true,
            ..all_off_settings()
        },
    };
    let raw = br#"{"model":"claude-x","system":[{"type":"text","text":"hi"}],"messages":[{"role":"user","content":[{"type":"text","text":"hello"}]}]}"#;
    let out = t
        .transform_bytes(raw, &main_ctx())
        .expect("auto_cache transform is Ok")
        .expect("auto_cache is an active feature -> Some (re-serialized + mutated)");
    let v: serde_json::Value = serde_json::from_slice(&out).expect("output is valid JSON");
    assert!(
        count_cache_breakpoints(&v) > 0,
        "auto_cache must have injected at least one cache breakpoint"
    );
}

#[test]
fn transform_bytes_drop_tools_on_returns_some() {
    let t = PinoTransform {
        settings: PinoSettings {
            drop_tools: vec!["Bash".to_string()],
            ..all_off_settings()
        },
    };
    let raw = br#"{"model":"claude-x","tools":[{"name":"Bash"},{"name":"Read"}],"messages":[{"role":"user","content":"hi"}]}"#;
    let out = t
        .transform_bytes(raw, &main_ctx())
        .expect("drop_tools transform is Ok")
        .expect("drop_tools is an active feature -> Some");
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    let names: Vec<&str> = v["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["Read"], "Bash must be dropped");
}

#[test]
fn transform_bytes_model_override_on_returns_some() {
    let t = PinoTransform {
        settings: PinoSettings {
            model_override: Some("claude-sonnet-4-6".to_string()),
            ..all_off_settings()
        },
    };
    let raw = br#"{"model":"claude-x","messages":[{"role":"user","content":"hi"}]}"#;
    let out = t
        .transform_bytes(raw, &main_ctx())
        .expect("model_override transform is Ok")
        .expect("model_override is an active feature -> Some");
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["model"], json!("claude-sonnet-4-6"));
}

#[test]
fn transform_bytes_strip_ansi_on_returns_some() {
    let t = PinoTransform {
        settings: PinoSettings {
            strip_ansi: true,
            ..all_off_settings()
        },
    };
    // ESC is JSON-escaped as backslash-u-001b (a literal ESC byte inside a JSON string is
    // invalid); strip_ansi must still remove the decoded CSI sequence.
    let raw = br#"{"model":"claude-x","messages":[{"role":"user","content":"\u001b[31mred\u001b[0m"}]}"#;
    let out = t
        .transform_bytes(raw, &main_ctx())
        .expect("strip_ansi transform is Ok")
        .expect("strip_ansi is an active feature -> Some");
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["messages"][0]["content"], json!("red"), "ANSI stripped");
}
