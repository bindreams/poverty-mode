use super::*;
use crate::proxy::BodyTransform;
use serde_json::json;

fn main_ctx() -> crate::proxy::RequestContext {
    crate::proxy::RequestContext::default()
}

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
    t.transform(&mut body, &main_ctx())
        .expect("disabled transform must be Ok");
    let after = serde_json::to_vec(&body).unwrap();
    assert_eq!(before, after, "disabled compression must be a byte-equal no-op");
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
    t.transform(&mut body, &main_ctx())
        .expect("enabled transform must be Ok on a valid body");
    let after = serde_json::to_vec(&body).unwrap();
    assert_eq!(before, after, "NoChange outcome must leave the body byte-identical");
}

// Characterization / regression tests (R12) ===========================================================================
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
    t.transform(&mut body, &main_ctx())
        .expect("enabled transform must be Ok");
    let after_content = tool_result_content(&body).len();
    let after_total = serde_json::to_vec(&body).unwrap().len();
    // Document is still a valid Anthropic request after rewrite.
    assert_eq!(body["messages"][0]["role"], json!("user"));
    assert_eq!(body["messages"][0]["content"][0]["type"], json!("tool_result"));
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
    assert!(ac < bc, "SmartCrusher must shrink JSON-array content ({bc} -> {ac})");
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
    assert!(ac < bc, "LogCompressor must shrink build-log content ({bc} -> {ac})");
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
        diff.push_str(&format!(" context line {} with extra padding text\n", i + 40));
    }
    assert!(
        diff.len() > 1024,
        "diff fixture must clear the GitDiff threshold; got {}",
        diff.len()
    );
    let (bc, ac, bt, at) = shrink_metrics(&diff);
    assert!(ac < bc, "DiffCompressor must shrink git-diff content ({bc} -> {ac})");
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

    t.transform(&mut body, &main_ctx())
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

// FIX-B: real byte-fidelity tests for the bytes-oriented seam =========================================================
//
// These are NOT round-trips through `serde_json::to_vec`: they feed a
// hand-authored COMPACT JSON body whose bytes DIFFER from serde_json's
// canonical output in the cache-hot zone (a `1e1` number literal, an
// escaped `\/`, and a non-ASCII `é` escape) and assert that the bytes
// the transform emits preserve that cache-hot region BYTE-FOR-BYTE. A
// transform that round-trips through `serde_json::Value` would canonicalize
// those literals (`1e1`->`10.0`, `\/`->`/`, `é`->`é`) and FAIL here.

/// A hand-authored COMPACT request body whose cache-hot zone (top-level
/// `system` + `tools`) contains byte-forms serde_json would rewrite on a
/// round-trip, alongside a large compressible JSON-array tool_result in the
/// live zone (the latest user message) that the dispatcher WILL rewrite.
fn noncanonical_hotzone_body() -> Vec<u8> {
    let array: Vec<serde_json::Value> = (0..200)
        .map(|i| json!({ "id": i, "status": "ok", "value": format!("repeat-pattern-{}", i % 3) }))
        .collect();
    // The tool_result content is itself a JSON string; embed it as a JSON
    // string literal so the outer document is valid.
    let payload = serde_json::to_string(&array).unwrap();
    let payload_literal = serde_json::to_string(&payload).unwrap();
    // Cache-hot zone literals serde_json::to_vec would NOT reproduce:
    //   1e1            -> serde would emit 10.0
    //   "a\/b"         -> serde would emit "a/b" (drops the redundant escape)
    //   "café"    -> serde would emit "café" (collapses the unicode escape)
    format!(
        r#"{{"model":"claude-sonnet-4-6","max_tokens":1e1,"system":[{{"type":"text","text":"café a\/b stable preamble"}}],"tools":[{{"name":"Bash","description":"run a\/b shell"}}],"messages":[{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"toolu_pm_bytes","content":{payload_literal}}}]}}]}}"#
    )
    .into_bytes()
}

#[test]
fn transform_bytes_preserves_noncanonical_cache_hot_zone_verbatim() {
    let t = HeadroomTransform {
        settings: HeadroomSettings { compression: true },
    };
    let raw = noncanonical_hotzone_body();

    // Sanity: the cache-hot byte-forms really are non-canonical (a Value
    // round-trip would change them), so this test is not vacuous.
    let canonical =
        serde_json::to_vec(&serde_json::from_slice::<serde_json::Value>(&raw).expect("fixture is valid JSON")).unwrap();
    assert_ne!(
        raw, canonical,
        "fixture must use non-canonical byte-forms in the cache-hot zone"
    );

    let out = t
        .transform_bytes(&raw, &main_ctx())
        .expect("enabled transform must be Ok")
        .expect("a 200-dict tool_result must be compressed -> Some(bytes)");

    // The compressible live-zone tool_result was rewritten -> the output is
    // strictly smaller (proves the dispatcher actually ran).
    assert!(
        out.len() < raw.len(),
        "live-zone compression must shrink the body ({} -> {})",
        raw.len(),
        out.len()
    );

    // BYTE-FOR-BYTE: every non-canonical cache-hot literal must survive in
    // the emitted bytes EXACTLY as authored (no serde canonicalization).
    let out_str = std::str::from_utf8(&out).expect("output is UTF-8");
    assert!(
        out_str.contains(r#""max_tokens":1e1"#),
        "1e1 number literal must survive verbatim (would be 10.0 after a Value round-trip): {out_str}"
    );
    assert!(
        out_str.contains(r#""café a\/b stable preamble""#),
        "non-ASCII \\u00e9 and redundant \\/ escapes in `system` must survive verbatim: {out_str}"
    );
    assert!(
        out_str.contains(r#""description":"run a\/b shell""#),
        "redundant \\/ escape in `tools` must survive verbatim: {out_str}"
    );
}

#[test]
fn transform_bytes_nochange_returns_none() {
    // Sub-512B JSON-array tool_result -> dispatcher returns NoChange -> the
    // bytes-oriented seam returns None (engine forwards the ORIGINAL bytes).
    let t = HeadroomTransform {
        settings: HeadroomSettings { compression: true },
    };
    let raw = serde_json::to_vec(&tiny_array_body()).unwrap();
    let out = t.transform_bytes(&raw, &main_ctx()).expect("transform must be Ok");
    assert!(
        out.is_none(),
        "NoChange must map to None (forward original bytes verbatim)"
    );
}

#[test]
fn transform_bytes_disabled_returns_none() {
    // compression off -> None (true byte passthrough), no dispatch attempted.
    let t = HeadroomTransform {
        settings: HeadroomSettings { compression: false },
    };
    let raw = serde_json::to_vec(&compressible_body()).unwrap();
    let out = t.transform_bytes(&raw, &main_ctx()).expect("transform must be Ok");
    assert!(out.is_none(), "disabled compression must map to None");
}

/// A body whose live-zone tool_result is a grep/`file:line:content` search
/// block sized just over the 512 B SearchResults threshold. At this margin the
/// token-validation gate (live_zone.rs `compressed_tokens < original_tokens`)
/// FLIPS by tokenizer family: the Claude estimator (3.5 chars/token, used for
/// every `claude-*` model incl. DEFAULT_MODEL) rejects the compression
/// (NoChange), while the OpenAI tiktoken BPE counter accepts it (Modified).
/// This is the lever that makes `body["model"]` observable in the output.
fn model_sensitive_search_payload() -> String {
    let mut results = String::new();
    for i in 0..8 {
        results.push_str(&format!(
            "src/module_{}.rs:{}:    let value = compute(input, config); // match\n",
            i % 7,
            (i % 400) + 1
        ));
    }
    // Just over the 512 B SearchResults threshold so the keep/compress decision
    // sits exactly on the token gate.
    assert!(
        (512..768).contains(&results.len()),
        "fixture must straddle the gate just past 512 B; got {}",
        results.len()
    );
    results
}

#[test]
fn transform_bytes_reads_model_from_body_for_tokenizer_gate() {
    use headroom_core::transforms::live_zone::DEFAULT_MODEL;
    use headroom_core::transforms::{compress_anthropic_live_zone, AuthMode, LiveZoneOutcome};

    /// Run the dispatcher directly and reduce to the forwarded bytes (Modified)
    /// or None (NoChange).
    fn dispatch(raw: &[u8], model: &str) -> Option<Vec<u8>> {
        match compress_anthropic_live_zone(raw, 0, AuthMode::Payg, model).expect("dispatch ok") {
            LiveZoneOutcome::Modified { new_body, .. } => Some(new_body.get().as_bytes().to_vec()),
            LiveZoneOutcome::NoChange { .. } => None,
        }
    }

    // The body declares a tiktoken-family model that routes to a DIFFERENT
    // backend (BPE) than DEFAULT_MODEL (the Claude 3.5 chars/token estimator).
    let body_model = "gpt-4o";
    assert_ne!(body_model, DEFAULT_MODEL);
    let mut body = compressible_body();
    body["model"] = json!(body_model);
    body["messages"][0]["content"][0]["content"] = json!(model_sensitive_search_payload());
    let raw = serde_json::to_vec(&body).unwrap();

    // DISCRIMINATOR: at this margin the token gate flips by tokenizer family. A
    // hardcoded DEFAULT_MODEL (or any `claude-*`) yields NoChange (None); the
    // body's own `gpt-4o` yields Modified (Some). These MUST differ, otherwise
    // the test could not catch a regression to a hardcoded model.
    let with_default = dispatch(&raw, DEFAULT_MODEL);
    let with_body_model = dispatch(&raw, body_model);
    assert!(
        with_default.is_none(),
        "DEFAULT_MODEL (Claude estimator) must reject this borderline block -> NoChange"
    );
    assert!(
        with_body_model.is_some(),
        "the body's tiktoken model must accept this borderline block -> Modified"
    );
    assert_ne!(
        with_default, with_body_model,
        "fixture must be a genuine discriminator: output differs by model"
    );

    // The transform must read `body[\"model\"]` for the gate, so its output
    // matches a direct dispatch with the body's model (Some) and NOT a hardcoded
    // DEFAULT_MODEL dispatch (None). If the impl ignored body["model"] and used
    // DEFAULT_MODEL, this would be None and the unwrap below would panic.
    let t = HeadroomTransform {
        settings: HeadroomSettings { compression: true },
    };
    let via_transform = t
        .transform_bytes(&raw, &main_ctx())
        .expect("transform must be Ok")
        .expect("body declares a tiktoken model -> Some (a hardcoded DEFAULT_MODEL would yield None)");

    assert_eq!(
        Some(&via_transform),
        with_body_model.as_ref(),
        "transform_bytes must dispatch with body[\"model\"], not DEFAULT_MODEL"
    );
}
