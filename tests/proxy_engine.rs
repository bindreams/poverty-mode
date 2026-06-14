mod common;

use common::stub::start_stub_async;
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
