//! Shared test fixtures for raw-byte serving and archive building (R3 — single copy, reused via
//! `mod common;`). The canonical request-capturing JSON stub lives in `tests/common/stub.rs`; this
//! module is for binary payloads (archives, `version.txt`) that the JSON stub cannot represent.
//!
//! `allow(dead_code)`: each integration-test crate (`download_http`, `central_version_http`) includes
//! the whole `common` module but uses only a subset of these helpers; the unused-in-this-crate ones
//! must not trip a `dead_code` warning (which `-D warnings` would turn into a build failure).
#![allow(dead_code)]

use std::convert::Infallible;
use std::io::Write as _;
use std::net::SocketAddr;
use std::sync::Arc;

use http_body_util::Full;
use hyper::body::Bytes;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;

/// Build a small `.tar.gz` in memory containing one file `file_rel` with known contents.
pub fn make_tar_gz_fixture(file_rel: &str, contents: &[u8]) -> Vec<u8> {
    use flate2::write::GzEncoder;
    use flate2::Compression;

    let mut tar_bytes: Vec<u8> = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut tar_bytes);
        let mut header = tar::Header::new_gnu();
        header.set_size(contents.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        builder.append_data(&mut header, file_rel, contents).unwrap();
        builder.finish().unwrap();
    }
    let mut gz = GzEncoder::new(Vec::new(), Compression::default());
    gz.write_all(&tar_bytes).unwrap();
    gz.finish().unwrap()
}

/// Lowercase hex sha256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    let d = h.finalize();
    let mut s = String::with_capacity(d.len() * 2);
    for b in d {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Start a hyper server on 127.0.0.1:0 that serves `body` (raw bytes) for EVERY request. Returns the
/// bound port. The port is learned from `local_addr()` after `bind` returns — a real readiness
/// primitive, no sleep/poll.
pub async fn serve_bytes(body: Vec<u8>) -> u16 {
    let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0))).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let body = Arc::new(body);
    tokio::spawn(async move {
        loop {
            let (stream, _) = match listener.accept().await {
                Ok(v) => v,
                Err(_) => break,
            };
            let io = TokioIo::new(stream);
            let body = body.clone();
            tokio::spawn(async move {
                let svc = service_fn(move |_req: Request<hyper::body::Incoming>| {
                    let body = body.clone();
                    async move { Ok::<_, Infallible>(Response::new(Full::new(Bytes::from((*body).clone())))) }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(io, svc)
                    .await;
            });
        }
    });
    port
}
