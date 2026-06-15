//! Integration test: orchestrator::health_probe / health_chain_id against an
//! in-process server serving /__pm/health, proving the real blocking probe works
//! and is driven off the runtime via spawn_blocking (R5).

use std::convert::Infallible;
use std::net::SocketAddr;

use http_body_util::Full;
use hyper::body::{Bytes, Incoming};
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;

use poverty_mode::orchestrator;

/// Serve a fixed HealthBody JSON at /__pm/health (200), 404 elsewhere. Returns
/// the bound port. The bind is awaited (no sleep); the server runs until exit.
async fn serve_health(run_id: &'static str) -> u16 {
    let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0))).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let body = format!(r#"{{"proxy":"pino","port":{port},"upstream":"api.anthropic.com","run_id":"{run_id}"}}"#);
    tokio::spawn(async move {
        loop {
            let (stream, _) = match listener.accept().await {
                Ok(v) => v,
                Err(_) => break,
            };
            let io = TokioIo::new(stream);
            let body = body.clone();
            tokio::spawn(async move {
                let svc = service_fn(move |req: Request<Incoming>| {
                    let body = body.clone();
                    async move {
                        if req.uri().path() == "/__pm/health" {
                            Ok::<_, Infallible>(
                                Response::builder()
                                    .status(StatusCode::OK)
                                    .header("content-type", "application/json")
                                    .header("x-pm-proxy", "pino")
                                    .body(Full::new(Bytes::from(body)))
                                    .unwrap(),
                            )
                        } else {
                            Ok::<_, Infallible>(
                                Response::builder()
                                    .status(StatusCode::NOT_FOUND)
                                    .body(Full::new(Bytes::new()))
                                    .unwrap(),
                            )
                        }
                    }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(io, svc)
                    .await;
            });
        }
    });
    port
}

#[tokio::test(flavor = "multi_thread")]
async fn health_chain_id_reads_live_run_id() {
    let port = serve_health("01HRUNX").await;
    let base = url::Url::parse(&format!("http://127.0.0.1:{port}")).unwrap();
    let id = tokio::task::spawn_blocking(move || orchestrator::health_chain_id(&base))
        .await
        .unwrap();
    assert_eq!(id, Some("01HRUNX".to_string()));
}

#[tokio::test(flavor = "multi_thread")]
async fn health_probe_none_when_nothing_listening() {
    // Bind+drop to obtain a definitely-closed port.
    let dead = {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let p = l.local_addr().unwrap().port();
        drop(l);
        p
    };
    let base = url::Url::parse(&format!("http://127.0.0.1:{dead}")).unwrap();
    let probe = tokio::task::spawn_blocking(move || orchestrator::health_probe(&base).is_some())
        .await
        .unwrap();
    assert!(!probe);
}
