mod common;

use common::stub::start_stub_async;
use std::net::SocketAddr;

use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::{Request, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::TcpStream;

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
