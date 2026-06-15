use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use http_body_util::{BodyExt, Full};
use hyper::body::{Bytes, Incoming};
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tokio::sync::Notify as TokioNotify;

/// What the stub upstream captured from a request it received (R3 shape plus the
/// extra fields M3's content-length / host assertions need).
#[derive(Clone, Default, Debug)]
pub struct Captured {
    pub method: String,
    pub uri: String,
    pub host: Option<String>,
    pub authorization: Option<String>,
    pub x_api_key: Option<String>,
    pub anthropic_beta: Option<String>,
    pub content_length: Option<String>,
    pub body: Vec<u8>,
}

#[derive(Default, Debug)]
struct CaptureState {
    last: Option<Captured>,
    count: usize,
}

/// A running in-process stub upstream (R3).
pub struct Stub {
    pub port: u16,
    state: Arc<Mutex<CaptureState>>,
}

impl Stub {
    /// The most recently captured request, if any (R3).
    pub fn last(&self) -> Option<Captured> {
        self.state.lock().unwrap().last.clone()
    }

    /// How many requests the stub has received (R3).
    pub fn count(&self) -> usize {
        self.state.lock().unwrap().count
    }

    /// The first path segment of the last request's URI (R3), e.g. `/v1/messages`
    /// → `v1`. `None` when there is no last request or no segment.
    pub fn first_segment(&self) -> Option<String> {
        let last = self.state.lock().unwrap().last.clone()?;
        let path = last.uri.split('?').next().unwrap_or("");
        path.trim_start_matches('/')
            .split('/')
            .next()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
    }
}

/// Start a canonical stub upstream on 127.0.0.1:0 from any context (R3): records
/// each request and answers every request with the given canned JSON and status
/// 200. Returns its bound port + capture handle synchronously. The server runs
/// until the test process exits.
///
/// Setup runs on a dedicated OS thread that owns a leaked multi-thread runtime
/// (kept alive for the process lifetime so the accept loop keeps serving). Doing
/// the `block_on` on a separate thread means this is safe to call BOTH from a
/// plain `#[test]` (no caller runtime) AND from inside a `#[tokio::test]` (where
/// nesting `block_on` on a runtime thread would otherwise panic). Tests already
/// inside a runtime may also use `start_stub_async` directly.
pub fn start_stub(canned_json: &'static str) -> Stub {
    let (tx, rx) = std::sync::mpsc::channel::<Stub>();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("stub runtime");
        let stub = rt.block_on(start_stub_async(canned_json));
        tx.send(stub).expect("send stub handle");
        // Keep the runtime alive for the process lifetime so the accept loop
        // (spawned onto it by `start_stub_async`) keeps serving.
        std::mem::forget(rt);
        loop {
            std::thread::park();
        }
    });
    rx.recv().expect("stub setup thread produced a handle")
}

/// Async constructor used by tests already inside a tokio runtime (the common
/// case for the engine integration tests).
pub async fn start_stub_async(canned_json: &'static str) -> Stub {
    let state = Arc::new(Mutex::new(CaptureState::default()));
    let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .expect("stub binds");
    let port = listener.local_addr().expect("stub addr").port();
    spawn_accept_loop(listener, state.clone(), canned_json);
    Stub { port, state }
}

fn spawn_accept_loop(listener: TcpListener, state: Arc<Mutex<CaptureState>>, canned_json: &'static str) {
    tokio::spawn(async move {
        loop {
            let (stream, _) = match listener.accept().await {
                Ok(v) => v,
                Err(_) => continue,
            };
            let io = TokioIo::new(stream);
            let state = state.clone();
            tokio::spawn(async move {
                let svc = service_fn(move |req: Request<Incoming>| {
                    let state = state.clone();
                    async move {
                        let captured = capture_request(req).await;
                        {
                            let mut g = state.lock().unwrap();
                            g.last = Some(captured);
                            g.count += 1;
                        }
                        Ok::<_, Infallible>(
                            Response::builder()
                                .status(StatusCode::OK)
                                .header("content-type", "application/json")
                                .body(Full::new(Bytes::from_static(canned_json.as_bytes())))
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
}

/// A stub upstream whose response is held until `release` is notified. `started`
/// fires once the upstream has RECEIVED a request, so the test knows the request
/// is genuinely in-flight before it signals shutdown.
pub struct GatedStub {
    pub port: u16,
    pub started: Arc<TokioNotify>,
    pub release: Arc<TokioNotify>,
}

pub async fn start_gated_stub(canned_json: &'static str) -> GatedStub {
    let started = Arc::new(TokioNotify::new());
    let release = Arc::new(TokioNotify::new());
    let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .expect("gated stub binds");
    let port = listener.local_addr().expect("addr").port();

    let started_loop = started.clone();
    let release_loop = release.clone();
    tokio::spawn(async move {
        loop {
            let (stream, _) = match listener.accept().await {
                Ok(v) => v,
                Err(_) => continue,
            };
            let io = TokioIo::new(stream);
            let started = started_loop.clone();
            let release = release_loop.clone();
            tokio::spawn(async move {
                let svc = service_fn(move |_req: Request<Incoming>| {
                    let started = started.clone();
                    let release = release.clone();
                    async move {
                        // Signal that the request reached upstream, then block on
                        // the caller-controlled release event.
                        started.notify_waiters();
                        release.notified().await;
                        Ok::<_, Infallible>(
                            Response::builder()
                                .status(StatusCode::OK)
                                .header("content-type", "application/json")
                                .body(Full::new(Bytes::from_static(canned_json.as_bytes())))
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

    GatedStub { port, started, release }
}

async fn capture_request(req: Request<Incoming>) -> Captured {
    let method = req.method().to_string();
    let uri = req.uri().to_string();
    let hget = |name: &str| {
        req.headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
    };
    let host = hget("host");
    let authorization = hget("authorization");
    let x_api_key = hget("x-api-key");
    let anthropic_beta = hget("anthropic-beta");
    let content_length = hget("content-length");
    let body = req
        .into_body()
        .collect()
        .await
        .map(|b| b.to_bytes().to_vec())
        .unwrap_or_default();
    Captured {
        method,
        uri,
        host,
        authorization,
        x_api_key,
        anthropic_beta,
        content_length,
        body,
    }
}
