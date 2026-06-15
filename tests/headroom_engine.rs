//! R20 characterization: a compression-enabled headroom hop, bound through M3's
//! real engine seam (`bind_engine`), compresses the request body end-to-end. M3
//! already offloads the transform via `tokio::task::spawn_blocking` and catches
//! transform errors (warn + forward original) per R22/R23d; this test pins that
//! the headroom transform rides that offload and shrinks the body the upstream
//! receives. It is a characterization test (R12): the behavior already exists in
//! M3 (offload) and M5.3 (shrink), so it is green on first compile.

mod common;

use std::sync::Arc;

use common::stub::start_stub_async;
use poverty_mode::proxy::headroom::HeadroomSettings;
use poverty_mode::proxy::{bind_engine, EngineConfig, ProxyName, TransformKind, Upstream};
use serde_json::json;
use tokio::sync::Notify;

fn upstream(s: &str) -> Upstream {
    Upstream {
        url: url::Url::parse(s).expect("valid upstream url"),
    }
}

/// Large, highly compressible Anthropic body: a 200-dict JSON-array tool_result
/// well above the 512B JSON-array threshold, so the live-zone dispatcher rewrites
/// it (Modified path) and the body the upstream receives is strictly smaller.
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
            "content": [{ "type": "tool_result", "tool_use_id": "toolu_pm_engine", "content": payload }],
        }],
    })
}

/// Tiny sub-512B JSON-array tool_result: dispatcher returns NoChange, so the
/// offloaded transform must forward the body byte-equal.
fn tiny_body() -> serde_json::Value {
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
            "content": [{ "type": "tool_result", "tool_use_id": "toolu_pm_tiny", "content": payload }],
        }],
    })
}

/// FIX-B: a hand-authored COMPACT body whose cache-hot zone (top-level `system`,
/// `tools`, and a `1e1` number) uses byte-forms serde_json's canonical output
/// would rewrite, alongside a large compressible JSON-array tool_result in the
/// live zone the dispatcher WILL rewrite. Returns the raw bytes (NOT a Value)
/// because the whole point is to send bytes the engine must NOT canonicalize.
fn noncanonical_hotzone_bytes() -> Vec<u8> {
    let array: Vec<serde_json::Value> = (0..200)
        .map(|i| json!({ "id": i, "status": "ok", "value": format!("repeat-pattern-{}", i % 3) }))
        .collect();
    let payload = serde_json::to_string(&array).unwrap();
    let payload_literal = serde_json::to_string(&payload).unwrap();
    // 1e1 (serde would emit 10.0), \/ (serde drops it), é (serde collapses it).
    format!(
        r#"{{"model":"claude-sonnet-4-6","max_tokens":1e1,"system":[{{"type":"text","text":"café a\/b stable preamble"}}],"tools":[{{"name":"Bash","description":"run a\/b shell"}}],"messages":[{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"toolu_pm_bytes","content":{payload_literal}}}]}}]}}"#
    )
    .into_bytes()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn engine_forwards_headroom_cache_hot_zone_byte_for_byte() {
    let stub = start_stub_async(r#"{"ok":true}"#).await;
    let shutdown = Arc::new(Notify::new());
    let shutdown_fut = {
        let s = shutdown.clone();
        async move { s.notified().await }
    };
    let cfg = EngineConfig {
        name: ProxyName::Headroom,
        listen: "127.0.0.1:0".parse().unwrap(),
        upstream: upstream(&format!("http://127.0.0.1:{}", stub.port)),
        run_id: "01J0HEADROOMBYTES".to_string(),
        log_file: None,
        transform: TransformKind::Headroom(HeadroomSettings { compression: true }),
    };
    let bound = bind_engine(cfg, shutdown_fut).await.expect("bind");
    let port = bound.local_addr.port();
    let client = reqwest::Client::new();

    let sent = noncanonical_hotzone_bytes();
    // Sanity: the cache-hot byte-forms really are non-canonical, so a transform
    // that round-trips through serde_json::Value would corrupt them.
    let canonical =
        serde_json::to_vec(&serde_json::from_slice::<serde_json::Value>(&sent).expect("fixture valid")).unwrap();
    assert_ne!(sent, canonical, "fixture must use non-canonical cache-hot byte-forms");
    let sent_len = sent.len();

    let resp = client
        .post(format!("http://127.0.0.1:{port}/v1/messages"))
        .header("content-type", "application/json")
        .body(sent)
        .send()
        .await
        .expect("proxied request succeeds");
    assert!(resp.status().is_success());

    let received = stub.last().expect("stub captured the forwarded request");
    // The live-zone tool_result WAS compressed (body strictly smaller) ...
    assert!(
        received.body.len() < sent_len,
        "engine must forward a compressed body ({sent_len} -> {})",
        received.body.len()
    );
    // ... yet the cache-hot zone survived BYTE-FOR-BYTE (no canonicalization).
    let got = std::str::from_utf8(&received.body).expect("forwarded body is UTF-8");
    assert!(
        got.contains(r#""max_tokens":1e1"#),
        "1e1 must survive verbatim through the engine (Value round-trip would emit 10.0): {got}"
    );
    assert!(
        got.contains(r#""text":"café a\/b stable preamble""#),
        "non-ASCII + redundant-escape `system` text must survive byte-for-byte: {got}"
    );
    assert!(
        got.contains(r#""description":"run a\/b shell""#),
        "redundant \\/ escape in `tools` must survive byte-for-byte: {got}"
    );

    shutdown.notify_one();
    bound.handle.await.expect("engine task joins").expect("engine ok");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn engine_compresses_headroom_body_end_to_end() {
    // `start_stub_async` (NOT the sync `start_stub`) because we are inside a tokio
    // runtime: the sync constructor `block_on`s its own runtime and would panic
    // ("Cannot start a runtime from within a runtime") on a worker thread.
    let stub = start_stub_async(r#"{"ok":true}"#).await;

    // Bind the headroom engine via M3's seam: binds on 127.0.0.1:0 and returns
    // the real bound addr immediately (no port race, no readiness poll). The
    // engine serves through M3's offloaded forward path; the Headroom transform
    // is materialized from `cfg.transform` by `as_body_transform()`.
    let shutdown = Arc::new(Notify::new());
    let shutdown_fut = {
        let s = shutdown.clone();
        async move { s.notified().await }
    };
    let cfg = EngineConfig {
        name: ProxyName::Headroom,
        listen: "127.0.0.1:0".parse().unwrap(),
        upstream: upstream(&format!("http://127.0.0.1:{}", stub.port)),
        run_id: "01J0HEADROOM".to_string(),
        log_file: None,
        transform: TransformKind::Headroom(HeadroomSettings { compression: true }),
    };
    let bound = bind_engine(cfg, shutdown_fut).await.expect("bind");
    let port = bound.local_addr.port();
    let client = reqwest::Client::new();

    // (1) Large compressible body -> the upstream stub receives a SMALLER body,
    //     proving the headroom transform ran through M3's offloaded forward path.
    let sent = compressible_body();
    let sent_bytes = serde_json::to_vec(&sent).unwrap();
    let sent_len = sent_bytes.len();
    // `reqwest`'s `json` feature is intentionally not enabled (R2/R23a pin only
    // rustls-tls-native-roots/stream/blocking and forbid editing deps), so we set
    // the body bytes + `application/json` content-type by hand. That content-type
    // is exactly what trips the engine's `should_transform` gate.
    let resp = client
        .post(format!("http://127.0.0.1:{port}/v1/messages"))
        .header("content-type", "application/json")
        .body(sent_bytes)
        .send()
        .await
        .expect("proxied request succeeds");
    assert!(
        resp.status().is_success(),
        "engine forwarded and returned the stub response"
    );
    let received = stub.last().expect("stub captured the forwarded request");
    assert!(
        received.body.len() < sent_len,
        "engine must forward a compressed body ({sent_len} -> {})",
        received.body.len()
    );
    // The forwarded body is still valid JSON in the Anthropic shape.
    let received_json: serde_json::Value =
        serde_json::from_slice(&received.body).expect("forwarded body is valid JSON");
    assert_eq!(received_json["messages"][0]["content"][0]["type"], json!("tool_result"));

    // (2) Sub-512B body -> NoChange -> the upstream receives a byte-equal body,
    //     proving the offloaded transform is byte-faithful when nothing shrinks.
    let tiny = tiny_body();
    let tiny_bytes = serde_json::to_vec(&tiny).unwrap();
    let resp = client
        .post(format!("http://127.0.0.1:{port}/v1/messages"))
        .header("content-type", "application/json")
        .body(tiny_bytes.clone())
        .send()
        .await
        .expect("proxied tiny request succeeds");
    assert!(resp.status().is_success());
    let received = stub.last().expect("stub captured the tiny request");
    assert_eq!(received.body, tiny_bytes, "NoChange body must arrive byte-equal");

    // `notify_one` (not `notify_waiters`) stores a permit even if the serve task
    // has not yet re-registered on the shutdown future, so the drain cannot be
    // lost to a wake/register ordering.
    shutdown.notify_one();
    bound.handle.await.expect("engine task joins").expect("engine ok");
}
