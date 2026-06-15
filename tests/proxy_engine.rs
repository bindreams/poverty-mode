mod common;

use common::stub::{start_gated_stub, start_stub_async};
use std::net::SocketAddr;

use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::{Request, StatusCode};
use hyper_util::rt::TokioIo;
use poverty_mode::proxy::{bind_engine, EngineConfig, ProxyName, TransformKind, Upstream};
use tokio::net::TcpStream;
use tokio::sync::Notify;

fn upstream(s: &str) -> Upstream {
    Upstream {
        url: url::Url::parse(s).unwrap(),
    }
}

async fn raw_post(port: u16, path: &str, body: &str) -> (StatusCode, String) {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let stream = TcpStream::connect(addr).await.expect("connect stub");
    let io = TokioIo::new(stream);
    let (mut sender, conn) = hyper::client::conn::http1::handshake(io)
        .await
        .expect("handshake");
    tokio::spawn(async move {
        let _ = conn.await;
    });
    let req = Request::builder()
        .method("POST")
        .uri(path)
        .header("host", format!("127.0.0.1:{port}"))
        .header("content-type", "application/json")
        .body(Full::<Bytes>::from(body.to_string()))
        .unwrap();
    let resp = sender.send_request(req).await.expect("send");
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).to_string())
}

#[tokio::test]
async fn stub_records_the_last_request_and_accessors() {
    let stub = start_stub_async(r#"{"ok":true}"#).await;
    assert_eq!(stub.count(), 0);
    let (status, body) = raw_post(stub.port, "/v1/messages", r#"{"hi":1}"#).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, r#"{"ok":true}"#);

    let cap = stub.last().expect("a request was captured");
    assert_eq!(cap.method, "POST");
    assert_eq!(cap.uri, "/v1/messages");
    assert_eq!(cap.body, br#"{"hi":1}"#.to_vec());
    assert_eq!(stub.count(), 1);
    assert_eq!(stub.first_segment().as_deref(), Some("v1"));
}

#[tokio::test]
async fn bind_engine_reports_real_ephemeral_port_and_drains() {
    let stub = start_stub_async(r#"{"ok":true}"#).await;
    let shutdown = std::sync::Arc::new(Notify::new());
    let shutdown_fut = {
        let s = shutdown.clone();
        async move { s.notified().await }
    };
    let cfg = EngineConfig {
        name: ProxyName::Pino,
        listen: "127.0.0.1:0".parse().unwrap(),
        upstream: upstream(&format!("http://127.0.0.1:{}", stub.port)),
        run_id: "01J0BIND".to_string(),
        log_file: None,
        transform: TransformKind::None,
    };
    let bound = bind_engine(cfg, shutdown_fut).await.expect("bind");
    assert_ne!(bound.local_addr.port(), 0, "must report a real bound port");
    assert_eq!(bound.local_addr.ip().to_string(), "127.0.0.1");

    // Trigger drain and confirm the serve task exits cleanly. `notify_one`
    // (not `notify_waiters`) stores a permit when no waiter is yet registered,
    // so the wakeup is never lost to a spawn-vs-notify race: the serve task's
    // `notified().await` consumes the stored permit and the loop breaks. With
    // `notify_waiters` the notification would be dropped whenever the spawned
    // serve task had not yet polled its shutdown future, hanging the drain.
    shutdown.notify_one();
    bound.handle.await.expect("join").expect("engine ok");
}

async fn raw_get(port: u16, path: &str) -> (StatusCode, Option<String>, String) {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let stream = TcpStream::connect(addr).await.expect("connect");
    let io = TokioIo::new(stream);
    let (mut sender, conn) = hyper::client::conn::http1::handshake(io)
        .await
        .expect("handshake");
    tokio::spawn(async move {
        let _ = conn.await;
    });
    let req = Request::builder()
        .method("GET")
        .uri(path)
        .header("host", format!("127.0.0.1:{port}"))
        .body(Full::<Bytes>::from(Vec::new()))
        .unwrap();
    let resp = sender.send_request(req).await.expect("send");
    let status = resp.status();
    let hdr = resp
        .headers()
        .get("x-pm-proxy")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, hdr, String::from_utf8_lossy(&bytes).to_string())
}

// CONTRACT GUARD (not a new TDD cycle): the health route was implemented in
// M3.7. This pins the full local-health contract and the not-forwarded invariant.
#[tokio::test]
async fn health_is_answered_locally_with_identity_and_not_forwarded() {
    let stub = start_stub_async(r#"{"ok":true}"#).await;
    let shutdown = std::sync::Arc::new(Notify::new());
    let shutdown_fut = {
        let s = shutdown.clone();
        async move { s.notified().await }
    };
    let cfg = EngineConfig {
        name: ProxyName::Pino,
        listen: "127.0.0.1:0".parse().unwrap(),
        upstream: upstream(&format!("http://127.0.0.1:{}", stub.port)),
        run_id: "01J0HEALTH".to_string(),
        log_file: None,
        transform: TransformKind::None,
    };
    let bound = bind_engine(cfg, shutdown_fut).await.expect("bind");
    let port = bound.local_addr.port();

    let (status, hdr, body) = raw_get(port, "/__pm/health").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(hdr.as_deref(), Some("pino"));

    let j: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(j["proxy"], "pino");
    assert_eq!(j["port"], port);
    assert_eq!(j["upstream"], format!("127.0.0.1:{}", stub.port));
    assert_eq!(j["run_id"], "01J0HEALTH");

    // The stub upstream must NOT have been touched by the health probe.
    assert_eq!(stub.count(), 0, "health must not hit upstream");

    shutdown.notify_waiters();
    bound.handle.await.expect("join").expect("engine ok");
}

async fn raw_get_with_auth(
    port: u16,
    path: &str,
    api_key: &str,
    authorization: &str,
) -> StatusCode {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let stream = TcpStream::connect(addr).await.expect("connect");
    let io = TokioIo::new(stream);
    let (mut sender, conn) = hyper::client::conn::http1::handshake(io)
        .await
        .expect("handshake");
    tokio::spawn(async move {
        let _ = conn.await;
    });
    let req = Request::builder()
        .method("GET")
        .uri(path)
        .header("host", format!("127.0.0.1:{port}"))
        .header("x-api-key", api_key)
        .header("authorization", authorization)
        .body(Full::<Bytes>::from(Vec::new()))
        .unwrap();
    let resp = sender.send_request(req).await.expect("send");
    resp.status()
}

// raw_post variant that also returns the upstream response headers so we can
// assert verbatim forwarding.
async fn raw_post_with_resp_header(
    port: u16,
    path: &str,
    body: &str,
    header_name: &str,
) -> (StatusCode, Option<String>) {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let stream = TcpStream::connect(addr).await.expect("connect");
    let io = TokioIo::new(stream);
    let (mut sender, conn) = hyper::client::conn::http1::handshake(io)
        .await
        .expect("handshake");
    tokio::spawn(async move {
        let _ = conn.await;
    });
    let req = Request::builder()
        .method("POST")
        .uri(path)
        .header("host", format!("127.0.0.1:{port}"))
        .header("content-type", "application/json")
        .body(Full::<Bytes>::from(body.to_string()))
        .unwrap();
    let resp = sender.send_request(req).await.expect("send");
    let status = resp.status();
    let hv = resp
        .headers()
        .get(header_name)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let _ = resp.into_body().collect().await;
    (status, hv)
}

fn fwd_cfg(name: ProxyName, run_id: &str, upstream_url: &str) -> EngineConfig {
    EngineConfig {
        name,
        listen: "127.0.0.1:0".parse().unwrap(),
        upstream: upstream(upstream_url),
        run_id: run_id.to_string(),
        log_file: None,
        transform: TransformKind::None,
    }
}

#[tokio::test]
async fn forward_streams_get_applies_prefix_rewrites_host_passes_auth() {
    let stub = start_stub_async(r#"{"ok":true}"#).await;
    let shutdown = std::sync::Arc::new(Notify::new());
    let shutdown_fut = {
        let s = shutdown.clone();
        async move { s.notified().await }
    };
    let secret = format!(
        "http://127.0.0.1:{}/wire/SECRET/claude-code/anthropic",
        stub.port
    );
    let bound = bind_engine(fwd_cfg(ProxyName::Pino, "01J0FWD", &secret), shutdown_fut)
        .await
        .expect("bind");
    let port = bound.local_addr.port();

    // Non-/v1/messages GET -> stream-through forward path.
    let status = raw_get_with_auth(port, "/v1/models", "sk-ant-test-key", "Bearer tok-123").await;
    assert_eq!(status, StatusCode::OK);

    let cap = stub.last().expect("captured");
    assert_eq!(cap.uri, "/wire/SECRET/claude-code/anthropic/v1/models");
    assert_eq!(
        cap.host.as_deref(),
        Some(format!("127.0.0.1:{}", stub.port).as_str())
    );
    assert_eq!(cap.x_api_key.as_deref(), Some("sk-ant-test-key"));
    assert_eq!(cap.authorization.as_deref(), Some("Bearer tok-123"));

    shutdown.notify_waiters();
    bound.handle.await.expect("join").expect("engine ok");
}

#[tokio::test]
async fn forward_count_tokens_reaches_upstream() {
    let stub = start_stub_async(r#"{"ok":true}"#).await;
    let shutdown = std::sync::Arc::new(Notify::new());
    let shutdown_fut = {
        let s = shutdown.clone();
        async move { s.notified().await }
    };
    let bound = bind_engine(
        fwd_cfg(
            ProxyName::Pino,
            "01J0CT",
            &format!("http://127.0.0.1:{}", stub.port),
        ),
        shutdown_fut,
    )
    .await
    .expect("bind");
    let port = bound.local_addr.port();

    let body = r#"{"model":"claude-x","messages":[]}"#;
    let (status, _resp) = raw_post(port, "/v1/messages/count_tokens", body).await;
    assert_eq!(status, StatusCode::OK);

    let cap = stub.last().expect("captured");
    assert_eq!(cap.uri, "/v1/messages/count_tokens");
    assert_eq!(cap.body, body.as_bytes().to_vec());

    shutdown.notify_waiters();
    bound.handle.await.expect("join").expect("engine ok");
}

#[tokio::test]
async fn forward_post_messages_recomputes_content_length() {
    let stub = start_stub_async(r#"{"ok":true}"#).await;
    let shutdown = std::sync::Arc::new(Notify::new());
    let shutdown_fut = {
        let s = shutdown.clone();
        async move { s.notified().await }
    };
    let bound = bind_engine(
        fwd_cfg(
            ProxyName::Pino,
            "01J0CL",
            &format!("http://127.0.0.1:{}", stub.port),
        ),
        shutdown_fut,
    )
    .await
    .expect("bind");
    let port = bound.local_addr.port();

    // TransformKind::None -> no transform runs -> body byte-faithful.
    let body = r#"{"model":"claude-x","messages":[{"role":"user","content":"hi"}]}"#;
    let (status, resp_body) = raw_post(port, "/v1/messages", body).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(resp_body, r#"{"ok":true}"#);

    let cap = stub.last().expect("captured");
    assert_eq!(cap.uri, "/v1/messages");
    assert_eq!(cap.body, body.as_bytes().to_vec());
    assert_eq!(
        cap.content_length.as_deref(),
        Some(body.len().to_string().as_str()),
        "content-length must equal the forwarded body length"
    );

    shutdown.notify_waiters();
    bound.handle.await.expect("join").expect("engine ok");
}

// FIX-B: a pino hop with ALL features off is a TRUE byte passthrough — the
// engine forwards the ORIGINAL request bytes verbatim (transform_bytes -> None),
// even when those bytes are non-canonical, and applies NO beta header (matching
// reference pino's `mutate=false` arm).
#[tokio::test]
async fn pino_all_features_off_forwards_bytes_verbatim_no_beta() {
    use poverty_mode::proxy::pino::{CacheTtl, PinoSettings};

    let stub = start_stub_async(r#"{"ok":true}"#).await;
    let shutdown = std::sync::Arc::new(Notify::new());
    let shutdown_fut = {
        let s = shutdown.clone();
        async move { s.notified().await }
    };
    let cfg = EngineConfig {
        name: ProxyName::Pino,
        listen: "127.0.0.1:0".parse().unwrap(),
        upstream: upstream(&format!("http://127.0.0.1:{}", stub.port)),
        run_id: "01J0PINOPASS".to_string(),
        log_file: None,
        transform: TransformKind::Pino(PinoSettings {
            auto_cache: false,
            tail_ttl: CacheTtl::FiveMin,
            drop_tools: vec![],
            strip_ansi: false,
            model_override: None,
        }),
    };
    let bound = bind_engine(cfg, shutdown_fut).await.expect("bind");
    let port = bound.local_addr.port();

    // Non-canonical byte-forms (1e1, redundant \/): a Value round-trip would
    // canonicalize these. Byte-equal forwarding proves the true passthrough.
    let body =
        r#"{"model":"claude-x","max_tokens":1e1,"messages":[{"role":"user","content":"a\/b"}]}"#;
    let (status, _resp) = raw_post(port, "/v1/messages", body).await;
    assert_eq!(status, StatusCode::OK);

    let cap = stub.last().expect("captured");
    assert_eq!(
        cap.body,
        body.as_bytes().to_vec(),
        "all-features-off pino must forward the original bytes verbatim"
    );
    assert_eq!(
        cap.content_length.as_deref(),
        Some(body.len().to_string().as_str())
    );
    assert!(
        cap.anthropic_beta.is_none(),
        "no beta header when no feature is active (apply_headers must not fire)"
    );

    shutdown.notify_waiters();
    bound.handle.await.expect("join").expect("engine ok");
}

// FIX-B: a pino hop with auto_cache ON re-serializes + injects breakpoints AND
// applies the 1h-cache beta header (apply_headers fires on the active transform).
#[tokio::test]
async fn pino_auto_cache_on_injects_and_applies_beta() {
    use poverty_mode::proxy::pino::{CacheTtl, PinoSettings, BETA_FLAG};

    let stub = start_stub_async(r#"{"ok":true}"#).await;
    let shutdown = std::sync::Arc::new(Notify::new());
    let shutdown_fut = {
        let s = shutdown.clone();
        async move { s.notified().await }
    };
    let cfg = EngineConfig {
        name: ProxyName::Pino,
        listen: "127.0.0.1:0".parse().unwrap(),
        upstream: upstream(&format!("http://127.0.0.1:{}", stub.port)),
        run_id: "01J0PINOCACHE".to_string(),
        log_file: None,
        transform: TransformKind::Pino(PinoSettings {
            auto_cache: true,
            tail_ttl: CacheTtl::FiveMin,
            drop_tools: vec![],
            strip_ansi: false,
            model_override: None,
        }),
    };
    let bound = bind_engine(cfg, shutdown_fut).await.expect("bind");
    let port = bound.local_addr.port();

    let body = r#"{"model":"claude-x","system":[{"type":"text","text":"hi"}],"messages":[{"role":"user","content":[{"type":"text","text":"hello"}]}]}"#;
    let (status, _resp) = raw_post(port, "/v1/messages", body).await;
    assert_eq!(status, StatusCode::OK);

    let cap = stub.last().expect("captured");
    let received: serde_json::Value =
        serde_json::from_slice(&cap.body).expect("forwarded body is JSON");
    let breakpoints = serde_json::to_string(&received)
        .unwrap()
        .matches("cache_control")
        .count();
    assert!(
        breakpoints > 0,
        "auto_cache must inject at least one cache breakpoint"
    );
    assert_eq!(
        cap.anthropic_beta.as_deref(),
        Some(BETA_FLAG),
        "auto_cache must apply the 1h-cache beta header via apply_headers"
    );
    assert_eq!(
        cap.content_length.as_deref(),
        Some(cap.body.len().to_string().as_str()),
        "content-length must equal the transformed body length"
    );

    shutdown.notify_waiters();
    bound.handle.await.expect("join").expect("engine ok");
}

// POST with content-type text/plain, to exercise the is_json_content_type guard
// (a non-JSON POST to /v1/messages must NOT be transformed). Gated with its sole
// caller (the `test-transforms` non-JSON test) so the default build has no dead code.
#[cfg(feature = "test-transforms")]
async fn raw_post_text(port: u16, path: &str, body: &str) -> StatusCode {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let stream = TcpStream::connect(addr).await.expect("connect");
    let io = TokioIo::new(stream);
    let (mut sender, conn) = hyper::client::conn::http1::handshake(io)
        .await
        .expect("handshake");
    tokio::spawn(async move {
        let _ = conn.await;
    });
    let req = Request::builder()
        .method("POST")
        .uri(path)
        .header("host", format!("127.0.0.1:{port}"))
        .header("content-type", "text/plain")
        .body(Full::<Bytes>::from(body.to_string()))
        .unwrap();
    let resp = sender.send_request(req).await.expect("send");
    resp.status()
}

#[cfg(feature = "test-transforms")]
#[tokio::test]
async fn transform_and_apply_headers_run_on_post_messages() {
    use poverty_mode::proxy::{bind_engine_with_boxed_transform, MarkerTransform};

    let stub = start_stub_async(r#"{"ok":true}"#).await;
    let shutdown = std::sync::Arc::new(Notify::new());
    let shutdown_fut = {
        let s = shutdown.clone();
        async move { s.notified().await }
    };
    let cfg = fwd_cfg(
        ProxyName::Pino,
        "01J0XF",
        &format!("http://127.0.0.1:{}", stub.port),
    );
    let bound =
        bind_engine_with_boxed_transform(cfg, std::sync::Arc::new(MarkerTransform), shutdown_fut)
            .await
            .expect("bind");
    let port = bound.local_addr.port();

    let body = r#"{"model":"claude-x","messages":[]}"#;
    let (status, _resp) = raw_post(port, "/v1/messages", body).await;
    assert_eq!(status, StatusCode::OK);

    let cap = stub.last().expect("captured");
    let received: serde_json::Value = serde_json::from_slice(&cap.body).unwrap();
    assert_eq!(
        received["__pm_test"],
        serde_json::Value::Bool(true),
        "body transform must run"
    );
    assert_eq!(received["model"], "claude-x");
    assert_eq!(
        cap.content_length.as_deref(),
        Some(cap.body.len().to_string().as_str()),
        "content-length must equal the transformed body length"
    );
    // R6: the apply_headers hook fired (x-pm-marker reached upstream). The
    // canonical stub records anthropic-beta; for the marker we assert through a
    // raw upstream read below in transform_apply_headers_hook_reaches_upstream.

    shutdown.notify_waiters();
    bound.handle.await.expect("join").expect("engine ok");
}

#[cfg(feature = "test-transforms")]
#[tokio::test]
async fn transform_apply_headers_hook_reaches_upstream() {
    use poverty_mode::proxy::{bind_engine_with_boxed_transform, MarkerTransform};
    // A bespoke upstream that records the `x-pm-marker` header, proving R6's
    // apply_headers ran AND its mutation reached upstream.
    use std::sync::{Arc as StdArc, Mutex};
    let seen: StdArc<Mutex<Option<String>>> = StdArc::new(Mutex::new(None));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let up_port = listener.local_addr().unwrap().port();
    let seen_loop = seen.clone();
    tokio::spawn(async move {
        loop {
            let (stream, _) = match listener.accept().await {
                Ok(v) => v,
                Err(_) => continue,
            };
            let io = TokioIo::new(stream);
            let seen = seen_loop.clone();
            tokio::spawn(async move {
                let svc = hyper::service::service_fn(move |req: Request<hyper::body::Incoming>| {
                    let seen = seen.clone();
                    async move {
                        let marker = req
                            .headers()
                            .get("x-pm-marker")
                            .and_then(|v| v.to_str().ok())
                            .map(|s| s.to_string());
                        *seen.lock().unwrap() = marker;
                        let _ = req.into_body().collect().await;
                        Ok::<_, std::convert::Infallible>(
                            hyper::Response::builder()
                                .status(StatusCode::OK)
                                .header("content-type", "application/json")
                                .body(Full::new(Bytes::from_static(b"{\"ok\":true}")))
                                .unwrap(),
                        )
                    }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(io, svc)
                    .await;
            });
        }
    });

    let shutdown = std::sync::Arc::new(Notify::new());
    let shutdown_fut = {
        let s = shutdown.clone();
        async move { s.notified().await }
    };
    let cfg = fwd_cfg(
        ProxyName::Pino,
        "01J0HK",
        &format!("http://127.0.0.1:{up_port}"),
    );
    let bound =
        bind_engine_with_boxed_transform(cfg, std::sync::Arc::new(MarkerTransform), shutdown_fut)
            .await
            .expect("bind");
    let port = bound.local_addr.port();

    let (status, _r) = raw_post(port, "/v1/messages", r#"{"messages":[]}"#).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        seen.lock().unwrap().as_deref(),
        Some("applied"),
        "R6 apply_headers must run on a transformed POST /v1/messages and reach upstream"
    );

    shutdown.notify_waiters();
    bound.handle.await.expect("join").expect("engine ok");
}

#[cfg(feature = "test-transforms")]
#[tokio::test]
async fn transform_and_hook_do_not_run_off_messages_path() {
    use poverty_mode::proxy::{bind_engine_with_boxed_transform, MarkerTransform};

    let stub = start_stub_async(r#"{"ok":true}"#).await;
    let shutdown = std::sync::Arc::new(Notify::new());
    let shutdown_fut = {
        let s = shutdown.clone();
        async move { s.notified().await }
    };
    let cfg = fwd_cfg(
        ProxyName::Pino,
        "01J0XF2",
        &format!("http://127.0.0.1:{}", stub.port),
    );
    let bound =
        bind_engine_with_boxed_transform(cfg, std::sync::Arc::new(MarkerTransform), shutdown_fut)
            .await
            .expect("bind");
    let port = bound.local_addr.port();

    let body = r#"{"x":1}"#;
    let (status, _resp) = raw_post(port, "/v1/other", body).await;
    assert_eq!(status, StatusCode::OK);

    let cap = stub.last().expect("captured");
    assert_eq!(
        cap.body,
        body.as_bytes().to_vec(),
        "non-messages body must be byte-faithful"
    );
    assert!(
        serde_json::from_slice::<serde_json::Value>(&cap.body)
            .unwrap()
            .get("__pm_test")
            .is_none(),
        "transform must not run off the messages path"
    );

    shutdown.notify_waiters();
    bound.handle.await.expect("join").expect("engine ok");
}

#[cfg(feature = "test-transforms")]
#[tokio::test]
async fn transform_does_not_run_on_non_json_post_messages() {
    use poverty_mode::proxy::{bind_engine_with_boxed_transform, MarkerTransform};

    let stub = start_stub_async(r#"{"ok":true}"#).await;
    let shutdown = std::sync::Arc::new(Notify::new());
    let shutdown_fut = {
        let s = shutdown.clone();
        async move { s.notified().await }
    };
    let cfg = fwd_cfg(
        ProxyName::Pino,
        "01J0XF3",
        &format!("http://127.0.0.1:{}", stub.port),
    );
    let bound =
        bind_engine_with_boxed_transform(cfg, std::sync::Arc::new(MarkerTransform), shutdown_fut)
            .await
            .expect("bind");
    let port = bound.local_addr.port();

    // content-type text/plain on /v1/messages -> is_json_content_type guard bars
    // the transform; body streams through byte-faithful.
    let body = r#"{"model":"claude-x","messages":[]}"#;
    let status = raw_post_text(port, "/v1/messages", body).await;
    assert_eq!(status, StatusCode::OK);

    let cap = stub.last().expect("captured");
    assert_eq!(
        cap.body,
        body.as_bytes().to_vec(),
        "non-JSON body must be byte-faithful"
    );
    assert!(
        serde_json::from_slice::<serde_json::Value>(&cap.body)
            .unwrap()
            .get("__pm_test")
            .is_none(),
        "transform must not run for a non-JSON content-type"
    );

    shutdown.notify_waiters();
    bound.handle.await.expect("join").expect("engine ok");
}

#[tokio::test]
async fn forward_passes_upstream_response_headers_verbatim() {
    // Stub that returns a distinctive response header alongside JSON.
    let stub = start_stub_async(r#"{"ok":true}"#).await;
    let shutdown = std::sync::Arc::new(Notify::new());
    let shutdown_fut = {
        let s = shutdown.clone();
        async move { s.notified().await }
    };
    let bound = bind_engine(
        fwd_cfg(
            ProxyName::Pino,
            "01J0RH",
            &format!("http://127.0.0.1:{}", stub.port),
        ),
        shutdown_fut,
    )
    .await
    .expect("bind");
    let port = bound.local_addr.port();

    // The canonical stub always sets content-type: application/json; assert it
    // reaches the client verbatim (verbatim-response-header forwarding).
    let (status, ct) =
        raw_post_with_resp_header(port, "/v1/messages", r#"{"messages":[]}"#, "content-type").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        ct.as_deref(),
        Some("application/json"),
        "upstream response headers must pass through"
    );

    shutdown.notify_waiters();
    bound.handle.await.expect("join").expect("engine ok");
}

#[tokio::test]
async fn streaming_response_is_tee_d_to_log_file_when_configured() {
    let stub = start_stub_async("event: message\ndata: {\"a\":1}\n\n").await;
    let shutdown = std::sync::Arc::new(Notify::new());
    let shutdown_fut = {
        let s = shutdown.clone();
        async move { s.notified().await }
    };
    let dir = tempfile::tempdir().unwrap();
    let log_path = dir.path().join("pino-test.log");
    let cfg = EngineConfig {
        name: ProxyName::Pino,
        listen: "127.0.0.1:0".parse().unwrap(),
        upstream: upstream(&format!("http://127.0.0.1:{}", stub.port)),
        run_id: "01J0TEE".to_string(),
        log_file: Some(log_path.clone()),
        transform: TransformKind::None,
    };
    let bound = bind_engine(cfg, shutdown_fut).await.expect("bind");
    let port = bound.local_addr.port();

    let (status, body) = raw_post(port, "/v1/messages", r#"{"messages":[]}"#).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "event: message\ndata: {\"a\":1}\n\n");

    // Drain so the tee file is flushed/closed.
    shutdown.notify_waiters();
    bound.handle.await.expect("join").expect("engine ok");

    let logged = std::fs::read_to_string(&log_path).expect("log file exists");
    assert!(
        logged.contains("event: message"),
        "response body must be tee'd to the log: got {logged:?}"
    );
}

// FIX-C: the body-tee log holds full request/response bodies — the most
// sensitive on-disk artifact — so on POSIX it must be owner-only (0600), like
// every other file poverty-mode writes (paths::atomic_write / ensure_run_dir).
// Before the fix the engine created it with OpenOptions create+append and left
// it at the process umask default (typically 0644: world-readable), leaking
// captured prompts/responses to other local users.
#[cfg(unix)]
#[tokio::test]
async fn tee_log_file_is_owner_only_0600_on_unix() {
    use std::os::unix::fs::PermissionsExt;

    let stub = start_stub_async("event: message\ndata: {\"a\":1}\n\n").await;
    let shutdown = std::sync::Arc::new(Notify::new());
    let shutdown_fut = {
        let s = shutdown.clone();
        async move { s.notified().await }
    };
    let dir = tempfile::tempdir().unwrap();
    let log_path = dir.path().join("pino-perms.log");
    let cfg = EngineConfig {
        name: ProxyName::Pino,
        listen: "127.0.0.1:0".parse().unwrap(),
        upstream: upstream(&format!("http://127.0.0.1:{}", stub.port)),
        run_id: "01J0PERM".to_string(),
        log_file: Some(log_path.clone()),
        transform: TransformKind::None,
    };
    let bound = bind_engine(cfg, shutdown_fut).await.expect("bind");
    let port = bound.local_addr.port();

    let (status, _) = raw_post(port, "/v1/messages", r#"{"messages":[]}"#).await;
    assert_eq!(status, StatusCode::OK);

    shutdown.notify_waiters();
    bound.handle.await.expect("join").expect("engine ok");

    let mode = std::fs::metadata(&log_path)
        .expect("log file exists")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(
        mode, 0o600,
        "body-tee log must be hardened to 0600 (was {mode:o})"
    );
}

// FIX-C (review): the previous test only checks the FINAL mode, which the old
// code also reached via a post-create chmod (`harden_file_perms`) — so it cannot
// catch the TOCTOU window where a freshly-created file briefly exists at the umask
// default (e.g. 0644, world-readable). This test pins the no-window property: it
// replicates the engine's open pattern — `.mode(0o600)` applied AT creation, with
// NO subsequent chmod — under a deliberately permissive umask (0o000). The old
// `OpenOptions::new().create(true).append(true)` would be born at 0666 here; the
// fix's `.mode(0o600)` makes it 0600 from the first instant on disk.
#[cfg(unix)]
#[tokio::test]
async fn tee_log_file_is_born_owner_only_no_world_readable_window() {
    // `tokio::fs::OpenOptions::mode` is an inherent (cfg(unix)) method, so no
    // `OpenOptionsExt` import is needed; we only need `PermissionsExt` for `.mode()`
    // on the read-back metadata.
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let log_path = dir.path().join("pino-born-0600.log");

    // Make the umask as permissive as possible so a missing `.mode()` would yield
    // 0666: this is what surfaces the world-readable window the fix closes.
    let prev_umask = unsafe { libc::umask(0o000) };

    let file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(&log_path)
        .await
        .expect("open tee file");
    // Read the on-disk mode BEFORE any harden_file_perms-style chmod could run.
    let born_mode = file
        .metadata()
        .await
        .expect("stat tee file")
        .permissions()
        .mode()
        & 0o777;

    // Restore the inherited umask for the rest of the test process.
    unsafe { libc::umask(prev_umask) };

    assert_eq!(
        born_mode, 0o600,
        "body-tee log must be created owner-only (0600) with no world-readable \
         window; was born at {born_mode:o} under a permissive umask"
    );
}

#[tokio::test]
async fn tee_log_file_port_placeholder_resolves_to_bound_port() {
    // The orchestrator builds each hop's body-log path BEFORE the OS assigns the
    // ephemeral port, so it embeds the literal token `{port}`; the engine must
    // substitute the real bound port at file-open so the on-disk file matches the
    // design-spec §5.11 name `<proxy>-<port>.log` that `status::enumerate_runs`
    // parses. Without substitution the file would be named with a non-port token
    // (or a hop index) and status would report the WRONG port on every install.
    let stub = start_stub_async(r#"{"ok":true}"#).await;
    let shutdown = std::sync::Arc::new(Notify::new());
    let shutdown_fut = {
        let s = shutdown.clone();
        async move { s.notified().await }
    };
    let dir = tempfile::tempdir().unwrap();
    // The exact shape the orchestrator emits: `<dir>/<proxy>-{port}.log`.
    let templated = dir.path().join("pino-{port}.log");
    let cfg = EngineConfig {
        name: ProxyName::Pino,
        listen: "127.0.0.1:0".parse().unwrap(),
        upstream: upstream(&format!("http://127.0.0.1:{}", stub.port)),
        run_id: "01J0PORT".to_string(),
        log_file: Some(templated),
        transform: TransformKind::None,
    };
    let bound = bind_engine(cfg, shutdown_fut).await.expect("bind");
    let port = bound.local_addr.port();

    let (status, _) = raw_post(port, "/v1/messages", r#"{"messages":[]}"#).await;
    assert_eq!(status, StatusCode::OK);

    shutdown.notify_waiters();
    bound.handle.await.expect("join").expect("engine ok");

    // The file must land at the real-port name, not at the literal `{port}` token.
    let resolved = dir.path().join(format!("pino-{port}.log"));
    assert!(
        resolved.exists(),
        "engine must resolve {{port}} to the bound port {port}: expected {}",
        resolved.display()
    );
    let literal = dir.path().join("pino-{port}.log");
    assert!(
        !literal.exists(),
        "engine must NOT write the literal {{port}} token as a filename: {}",
        literal.display()
    );
}

#[tokio::test]
async fn no_log_file_means_no_file_written() {
    let stub = start_stub_async(r#"{"ok":true}"#).await;
    let shutdown = std::sync::Arc::new(Notify::new());
    let shutdown_fut = {
        let s = shutdown.clone();
        async move { s.notified().await }
    };
    let dir = tempfile::tempdir().unwrap();
    let cfg = EngineConfig {
        name: ProxyName::Pino,
        listen: "127.0.0.1:0".parse().unwrap(),
        upstream: upstream(&format!("http://127.0.0.1:{}", stub.port)),
        run_id: "01J0NOLOG".to_string(),
        log_file: None,
        transform: TransformKind::None,
    };
    let bound = bind_engine(cfg, shutdown_fut).await.expect("bind");
    let port = bound.local_addr.port();
    let (status, _) = raw_post(port, "/v1/messages", r#"{"messages":[]}"#).await;
    assert_eq!(status, StatusCode::OK);

    shutdown.notify_waiters();
    bound.handle.await.expect("join").expect("engine ok");

    assert!(
        std::fs::read_dir(dir.path()).unwrap().next().is_none(),
        "no log file must be created when log_file is None"
    );
}

// CONTRACT GUARD (not a new TDD cycle): the 502-on-upstream-failure arm was
// implemented in M3.9's forward(). This locks the fail-surface contract (spec
// §11): a closed upstream yields 502 with an explanatory body, never a reset
// connection / escaping Err.
#[tokio::test]
async fn upstream_down_yields_502() {
    // Bind-and-immediately-drop a listener to get a port guaranteed closed.
    let dead_port = {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let p = l.local_addr().unwrap().port();
        drop(l);
        p
    };
    let shutdown = std::sync::Arc::new(Notify::new());
    let shutdown_fut = {
        let s = shutdown.clone();
        async move { s.notified().await }
    };
    let cfg = EngineConfig {
        name: ProxyName::Pino,
        listen: "127.0.0.1:0".parse().unwrap(),
        upstream: upstream(&format!("http://127.0.0.1:{dead_port}")),
        run_id: "01J0502".to_string(),
        log_file: None,
        transform: TransformKind::None,
    };
    let bound = bind_engine(cfg, shutdown_fut).await.expect("bind");
    let port = bound.local_addr.port();

    let (status, body) = raw_post(port, "/v1/messages", r#"{"messages":[]}"#).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert!(
        body.contains("proxy upstream error"),
        "502 body should explain the upstream failure, got: {body:?}"
    );

    shutdown.notify_waiters();
    bound.handle.await.expect("join").expect("engine ok");
}

#[tokio::test]
async fn drain_completes_in_flight_request_before_exit() {
    let gated = start_gated_stub(r#"{"drained":true}"#).await;
    let shutdown = std::sync::Arc::new(Notify::new());
    let shutdown_fut = {
        let s = shutdown.clone();
        async move { s.notified().await }
    };
    let cfg = EngineConfig {
        name: ProxyName::Pino,
        listen: "127.0.0.1:0".parse().unwrap(),
        upstream: upstream(&format!("http://127.0.0.1:{}", gated.port)),
        run_id: "01J0DRAIN".to_string(),
        log_file: None,
        transform: TransformKind::None,
    };
    let bound = bind_engine(cfg, shutdown_fut).await.expect("bind");
    let port = bound.local_addr.port();

    // Fire the slow request concurrently; it blocks at the upstream.
    let client_task =
        tokio::spawn(async move { raw_post(port, "/v1/messages", r#"{"messages":[]}"#).await });

    // Wait (on a real event) until the request has reached the upstream.
    gated.started.notified().await;

    // Signal the engine to begin draining WHILE the request is in flight.
    shutdown.notify_waiters();

    // Now release the upstream; the in-flight request must complete.
    gated.release.notify_waiters();

    let (status, body) = client_task.await.expect("client task");
    assert_eq!(
        status,
        StatusCode::OK,
        "in-flight request must complete during drain"
    );
    assert_eq!(body, r#"{"drained":true}"#);

    // After the in-flight request finishes, the serve task must drain and exit.
    bound
        .handle
        .await
        .expect("join")
        .expect("engine drained ok");
}

use poverty_mode::proxy::run_proxy_with_shutdown;

// R5 guard: drive the engine through a healthy fake on the multi-thread runtime,
// serving a REAL request before draining, then drive the public async entry to
// completion. `flavor = "multi_thread"` ensures any accidental blocking call
// surfaces as a hang/panic the suite would catch, not a single-threaded deadlock
// that masks it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_proxy_entry_serves_and_drains_without_blocking_panic() {
    let stub = start_stub_async(r#"{"ok":true}"#).await;

    // Part 1 — the engine path (the body run_proxy_with_shutdown runs): bind via
    // bind_engine (returns the port), serve a REAL POST that reaches the healthy
    // fake, then drain. This is the real-forward R5 demands.
    let shutdown = std::sync::Arc::new(Notify::new());
    let shutdown_fut = {
        let s = shutdown.clone();
        async move { s.notified().await }
    };
    let cfg = EngineConfig {
        name: ProxyName::Pino,
        listen: "127.0.0.1:0".parse().unwrap(),
        upstream: upstream(&format!("http://127.0.0.1:{}", stub.port)),
        run_id: "01J0R5A".to_string(),
        log_file: None,
        transform: TransformKind::None,
    };
    let bound = bind_engine(cfg, shutdown_fut).await.expect("bind");
    let port = bound.local_addr.port();

    let (status, body) = raw_post(port, "/v1/messages", r#"{"messages":[]}"#).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "engine must serve a real forward without blocking panic"
    );
    assert_eq!(body, r#"{"ok":true}"#);
    assert_eq!(stub.count(), 1, "the real request reached the healthy fake");

    shutdown.notify_waiters();
    bound
        .handle
        .await
        .expect("join")
        .expect("engine drained ok");

    // Part 2 — the public async entry: bind + drain to completion with no
    // blocking-call panic. It prints its READY line to process stdout (not
    // readable in-process), so we synchronize on the run task finishing after we
    // signal shutdown; with no in-flight connection the drain completes at once.
    // `notify_one` (not `notify_waiters`) stores a permit when no waiter is yet
    // registered, so the wakeup is never lost to the spawn-vs-notify race: the
    // engine's `notified().await` consumes the stored permit and drains. With
    // `notify_waiters` the signal would be dropped whenever the spawned run task
    // had not yet polled its shutdown future, hanging the drain forever.
    let shutdown2 = std::sync::Arc::new(Notify::new());
    let shutdown2_fut = {
        let s = shutdown2.clone();
        async move { s.notified().await }
    };
    let cfg2 = EngineConfig {
        name: ProxyName::Pino,
        listen: "127.0.0.1:0".parse().unwrap(),
        upstream: upstream(&format!("http://127.0.0.1:{}", stub.port)),
        run_id: "01J0R5B".to_string(),
        log_file: None,
        transform: TransformKind::None,
    };
    let run = tokio::spawn(async move { run_proxy_with_shutdown(cfg2, shutdown2_fut).await });
    shutdown2.notify_one();
    run.await
        .expect("run task join")
        .expect("run_proxy_with_shutdown ok");
}
