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
