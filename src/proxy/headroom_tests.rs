use super::*;
use crate::proxy::BodyTransform;
use serde_json::json;

#[test]
fn headroom_settings_default_round_trips_yaml() {
    let s = HeadroomSettings { compression: false };
    let yaml = serde_yaml::to_string(&s).unwrap();
    let back: HeadroomSettings = serde_yaml::from_str(&yaml).unwrap();
    assert_eq!(s, back);
}

#[test]
fn headroom_settings_rejects_unknown_fields() {
    let yaml = "compression: true\nbogus: 1\n";
    let err = serde_yaml::from_str::<HeadroomSettings>(yaml).unwrap_err();
    assert!(
        err.to_string().contains("bogus") || err.to_string().contains("unknown field"),
        "deny_unknown_fields should reject `bogus`, got: {err}"
    );
}

/// A representative Anthropic request body with a large, highly compressible
/// JSON-array tool_result. With compression DISABLED, the transform must not
/// touch a single byte — the serialized Value must be identical before/after.
fn compressible_body() -> serde_json::Value {
    let array: Vec<serde_json::Value> = (0..200)
        .map(|i| json!({ "id": i, "status": "ok", "value": format!("repeat-pattern-{}", i % 3) }))
        .collect();
    let payload = serde_json::to_string(&array).unwrap();
    json!({
        "model": "claude-sonnet-4-6",
        "max_tokens": 64,
        "system": "you are a helpful assistant",
        "messages": [{
            "role": "user",
            "content": [{
                "type": "tool_result",
                "tool_use_id": "toolu_pm_test",
                "content": payload,
            }],
        }],
    })
}

#[test]
fn disabled_compression_is_byte_equal_noop() {
    let t = HeadroomTransform {
        settings: HeadroomSettings { compression: false },
    };
    let mut body = compressible_body();
    let before = serde_json::to_vec(&body).unwrap();
    t.transform(&mut body)
        .expect("disabled transform must be Ok");
    let after = serde_json::to_vec(&body).unwrap();
    assert_eq!(
        before, after,
        "disabled compression must be a byte-equal no-op"
    );
}

/// A body whose JSON-array tool_result is BELOW the 512-byte JSON-array
/// threshold. The dispatcher returns NoChange, so even with compression
/// enabled the body must be byte-identical after transform.
fn tiny_array_body() -> serde_json::Value {
    // Three small dicts -> well under 512 bytes when serialized as a string.
    let array: Vec<serde_json::Value> = (0..3).map(|i| json!({ "id": i, "ok": true })).collect();
    let payload = serde_json::to_string(&array).unwrap();
    assert!(
        payload.len() < 512,
        "fixture must be below the 512B JSON-array threshold"
    );
    json!({
        "model": "claude-sonnet-4-6",
        "max_tokens": 64,
        "system": "you are a helpful assistant",
        "messages": [{
            "role": "user",
            "content": [{
                "type": "tool_result",
                "tool_use_id": "toolu_pm_tiny",
                "content": payload,
            }],
        }],
    })
}

#[test]
fn enabled_but_nothing_shrinks_is_byte_equal() {
    let t = HeadroomTransform {
        settings: HeadroomSettings { compression: true },
    };
    let mut body = tiny_array_body();
    let before = serde_json::to_vec(&body).unwrap();
    t.transform(&mut body)
        .expect("enabled transform must be Ok on a valid body");
    let after = serde_json::to_vec(&body).unwrap();
    assert_eq!(
        before, after,
        "NoChange outcome must leave the body byte-identical"
    );
}

// Characterization / regression tests (R12) ==========================
//
// These lock the full claimed compressor surface (spec 5.6): JSON tool
// output (SmartCrusher), build/test logs (LogCompressor), grep/search
// results (SearchCompressor), and git diffs (DiffCompressor). The
// behavior they exercise was already implemented by the `Modified`
// splice path in M5.3 (`feat(headroom): ... NoChange is byte-equal,
// Modified splices`). They are LABELED characterization tests added
// AFTER that behavior exists: there is no red->green cycle here, they
// are expected green immediately. A RED here signals a real defect in
// M5.3's wiring, not a missing feature.

/// Reach into a body and return the inner tool_result `content` string of
/// messages[0].content[0]. Panics if the shape is not as constructed.
fn tool_result_content(body: &serde_json::Value) -> String {
    body["messages"][0]["content"][0]["content"]
        .as_str()
        .expect("tool_result content is a JSON string")
        .to_string()
}

/// Build a single-user-message body whose only tool_result content is `text`.
fn body_with_tool_result(text: &str) -> serde_json::Value {
    json!({
        "model": "claude-sonnet-4-6",
        "max_tokens": 64,
        "system": "you are a helpful assistant",
        "messages": [{
            "role": "user",
            "content": [{
                "type": "tool_result",
                "tool_use_id": "toolu_pm_shrink",
                "content": text,
            }],
        }],
    })
}

/// Run a compression-enabled transform and return (before_content_len,
/// after_content_len, before_total_len, after_total_len). Asserts the
/// document survives as a valid Anthropic shape.
fn shrink_metrics(text: &str) -> (usize, usize, usize, usize) {
    let t = HeadroomTransform {
        settings: HeadroomSettings { compression: true },
    };
    let mut body = body_with_tool_result(text);
    let before_content = tool_result_content(&body).len();
    let before_total = serde_json::to_vec(&body).unwrap().len();
    t.transform(&mut body)
        .expect("enabled transform must be Ok");
    let after_content = tool_result_content(&body).len();
    let after_total = serde_json::to_vec(&body).unwrap().len();
    // Document is still a valid Anthropic request after rewrite.
    assert_eq!(body["messages"][0]["role"], json!("user"));
    assert_eq!(
        body["messages"][0]["content"][0]["type"],
        json!("tool_result")
    );
    (before_content, after_content, before_total, after_total)
}

#[test]
fn shrinks_json_array_tool_result_smart_crusher() {
    // 200 homogeneous dicts, far above 512B -> SmartCrusher fodder
    // (ported from live_zone_dispatch.rs::json_tool_result_routes_to_smart_crusher).
    let array: Vec<serde_json::Value> = (0..200)
        .map(|i| json!({ "id": i, "status": "ok", "value": format!("repeat-pattern-{}", i % 3) }))
        .collect();
    let payload = serde_json::to_string(&array).unwrap();
    let (bc, ac, bt, at) = shrink_metrics(&payload);
    assert!(
        ac < bc,
        "SmartCrusher must shrink JSON-array content ({bc} -> {ac})"
    );
    assert!(
        at < bt,
        "whole body must be smaller after JSON compression ({bt} -> {at})"
    );
}

#[test]
fn shrinks_build_log_tool_result_log_compressor() {
    // 200 repetitive build/log lines the detector classifies as BuildOutput
    // (ported from live_zone_dispatch.rs::log_tool_result_routes_to_log_compressor).
    let mut lines = String::new();
    for i in 0..200 {
        lines.push_str(&format!(
            "[INFO] 2026-05-02T19:30:{:02}.000Z app=widget request_id=abc-{} pool=default ok\n",
            i % 60,
            i
        ));
    }
    let (bc, ac, bt, at) = shrink_metrics(&lines);
    assert!(
        ac < bc,
        "LogCompressor must shrink build-log content ({bc} -> {ac})"
    );
    assert!(
        at < bt,
        "whole body must be smaller after log compression ({bt} -> {at})"
    );
}

#[test]
fn shrinks_search_results_tool_result_search_compressor() {
    // grep -n style `file:line:content` results the detector classifies as
    // SearchResults (shape from content_detector.rs::search_results_detected),
    // scaled above the 512B SearchResults threshold with repetitive rows.
    let mut results = String::new();
    for i in 0..200 {
        results.push_str(&format!(
            "src/module_{}.rs:{}:    let value = compute(input, config); // repeated match line\n",
            i % 7,
            (i % 400) + 1
        ));
    }
    let (bc, ac, bt, at) = shrink_metrics(&results);
    assert!(
        ac < bc,
        "SearchCompressor must shrink search-result content ({bc} -> {ac})"
    );
    assert!(
        at < bt,
        "whole body must be smaller after search compression ({bt} -> {at})"
    );
}

#[test]
fn shrinks_git_diff_tool_result_diff_compressor() {
    // Unified diff with abundant context the diff compressor can trim, sized
    // comfortably > 1 KiB (ported from
    // live_zone_dispatch.rs::diff_tool_result_routes_to_diff_compressor).
    let mut diff = String::from("diff --git a/foo.rs b/foo.rs\n--- a/foo.rs\n+++ b/foo.rs\n");
    diff.push_str("@@ -1,80 +1,80 @@\n");
    for i in 0..40 {
        diff.push_str(&format!(" context line {i} with extra padding text\n"));
    }
    diff.push_str("-old line that needs to be replaced\n+new line replacing the old one\n");
    for i in 0..40 {
        diff.push_str(&format!(
            " context line {} with extra padding text\n",
            i + 40
        ));
    }
    assert!(
        diff.len() > 1024,
        "diff fixture must clear the GitDiff threshold; got {}",
        diff.len()
    );
    let (bc, ac, bt, at) = shrink_metrics(&diff);
    assert!(
        ac < bc,
        "DiffCompressor must shrink git-diff content ({bc} -> {ac})"
    );
    assert!(
        at < bt,
        "whole body must be smaller after diff compression ({bt} -> {at})"
    );
}

/// A body that combines:
///  (a) a cache-hot top-level `system` block and `tools` array (must survive
///      byte-identical);
///  (b) HISTORY: an older assistant turn before the latest user message (must
///      survive byte-identical);
///  (c) a `thinking` hot-zone block INSIDE the latest user message, sitting next
///      to the compressible tool_result (must survive byte-identical — proves
///      HOT_ZONE_BLOCK_TYPES exclusion);
///  (d) a compressible 200-dict JSON-array tool_result that WILL be rewritten;
///  (e) a high-precision integer literal and a `1.0` float that must not
///      collapse under f64.
fn mixed_body() -> serde_json::Value {
    let array: Vec<serde_json::Value> = (0..200)
        .map(|i| json!({ "id": i, "status": "ok", "value": format!("repeat-pattern-{}", i % 3) }))
        .collect();
    let payload = serde_json::to_string(&array).unwrap();
    // 12345678901234567 cannot be represented exactly as f64; with
    // arbitrary_precision serde_json keeps the literal digits.
    serde_json::from_str(&format!(
        r#"{{
            "model": "claude-sonnet-4-6",
            "max_tokens": 64,
            "metadata": {{ "big": 12345678901234567, "frac": 1.0 }},
            "system": [
                {{ "type": "text", "text": "you are a helpful assistant with a long stable preamble" }}
            ],
            "tools": [
                {{ "name": "Bash", "description": "run a shell command", "input_schema": {{ "type": "object" }} }}
            ],
            "messages": [
                {{ "role": "user", "content": [{{ "type": "text", "text": "earlier question from the user" }}] }},
                {{ "role": "assistant", "content": [{{ "type": "text", "text": "earlier assistant answer that is part of cache history" }}] }},
                {{
                    "role": "user",
                    "content": [
                        {{ "type": "thinking", "thinking": "internal reasoning that must never be rewritten" }},
                        {{
                            "type": "tool_result",
                            "tool_use_id": "toolu_pm_mixed",
                            "content": {payload:?}
                        }}
                    ]
                }}
            ]
        }}"#,
        payload = payload
    ))
    .expect("mixed_body is valid JSON")
}

#[test]
fn hot_zones_history_and_numeric_precision_preserved() {
    let t = HeadroomTransform {
        settings: HeadroomSettings { compression: true },
    };
    let mut body = mixed_body();

    // Snapshot every region that must survive byte-identical.
    let system_before = serde_json::to_vec(&body["system"]).unwrap();
    let tools_before = serde_json::to_vec(&body["tools"]).unwrap();
    let history_user_before = serde_json::to_vec(&body["messages"][0]).unwrap();
    let history_assistant_before = serde_json::to_vec(&body["messages"][1]).unwrap();
    let thinking_before = serde_json::to_vec(&body["messages"][2]["content"][0]).unwrap();
    let big_before = body["metadata"]["big"].to_string();
    let frac_before = body["metadata"]["frac"].to_string();
    // The compressible tool_result is messages[2].content[1] (after the thinking block).
    let tool_result_before = body["messages"][2]["content"][1]["content"]
        .as_str()
        .expect("tool_result content is a string")
        .len();

    t.transform(&mut body)
        .expect("enabled transform must be Ok");

    // (a) Cache-hot top-level zones byte-identical.
    assert_eq!(
        system_before,
        serde_json::to_vec(&body["system"]).unwrap(),
        "system block must be untouched"
    );
    assert_eq!(
        tools_before,
        serde_json::to_vec(&body["tools"]).unwrap(),
        "tools array must be untouched"
    );
    // (b) History (older user + assistant turns) byte-identical.
    assert_eq!(
        history_user_before,
        serde_json::to_vec(&body["messages"][0]).unwrap(),
        "older user turn (history) must be untouched"
    );
    assert_eq!(
        history_assistant_before,
        serde_json::to_vec(&body["messages"][1]).unwrap(),
        "older assistant turn (history) must be untouched"
    );
    // (c) thinking hot-zone block inside the latest user message byte-identical.
    assert_eq!(
        thinking_before,
        serde_json::to_vec(&body["messages"][2]["content"][0]).unwrap(),
        "thinking block in the live zone must be excluded and untouched"
    );
    // (e) High-precision integer literal preserved exactly (no f64 collapse).
    assert_eq!(big_before, "12345678901234567");
    assert_eq!(
        big_before,
        body["metadata"]["big"].to_string(),
        "large integer literal must survive the round-trip exactly"
    );
    // 1.0 must not collapse to 1.
    assert_eq!(frac_before, "1.0");
    assert_eq!(
        frac_before,
        body["metadata"]["frac"].to_string(),
        "float literal 1.0 must not collapse to 1"
    );
    // (d) The live-zone tool_result WAS rewritten -> the survival above is
    // meaningful, not vacuous (something actually happened).
    let tool_result_after = body["messages"][2]["content"][1]["content"]
        .as_str()
        .expect("tool_result content is still a string")
        .len();
    assert!(
        tool_result_after < tool_result_before,
        "live-zone tool_result must have been compressed ({tool_result_before} -> {tool_result_after})"
    );
}
