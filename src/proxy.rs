//! Foundational proxy types shared by every later milestone (R9).
//!
//! These definitions are created once here; later milestones FILL behavior (the
//! async engine in M3, the pino/headroom transforms in M4/M5) but never redefine
//! these types. `proxy::pino` and `proxy::headroom` (this module's submodules)
//! hold per-proxy settings + transforms.

use std::convert::Infallible;
use std::io::Write;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use bytes::Bytes;
use futures_util::{StreamExt, TryStreamExt};
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full, StreamBody};
use hyper::body::{Frame, Incoming};
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use hyper_util::server::graceful::GracefulShutdown;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;

pub mod headroom;
pub mod pino;

/// True for `/v1/messages` and `/v1/messages/count_tokens`, with an optional
/// `?query` and an optional opaque leading wire-client prefix (C1: the agent
/// carries `claude-code/anthropic` in its base URL, so the inbound path arrives as
/// `/claude-code/anthropic/v1/messages`). A bare `ends_with` suffix match suffices
/// and is agent-agnostic — codex's `/responses` never ends with a messages suffix.
///
/// This DELIBERATELY diverges from upstream pino's exact `isMessagesPath`
/// (reference/pino/src/server.js): poverty-mode's chain is agent-agnostic, so the
/// messages suffix may sit behind a client/api prefix the proxy must not interpret.
pub fn is_messages_path(path: &str) -> bool {
    let base = path.split('?').next().unwrap_or("");
    base.ends_with("/v1/messages") || base.ends_with("/v1/messages/count_tokens")
}

/// True when the `content-type` header contains `application/json`
/// (case-insensitive). Mirrors the reference `isJsonRequest`.
pub fn is_json_content_type(headers: &http::HeaderMap) -> bool {
    headers
        .get(http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_ascii_lowercase().contains("application/json"))
        .unwrap_or(false)
}

/// Compose the absolute upstream URI from the upstream (scheme/host/port +
/// path prefix) and the inbound request's path-and-query.
///
/// The inbound path is appended after the upstream's `path_prefix()` (trailing
/// slash already stripped, `""` for a bare upstream). The scheme and authority
/// come from `host_header()` (default ports elided, explicit ports preserved —
/// JS `URL.host` parity, reference/pino/src/config.js:30). The inbound query is
/// preserved.
///
/// Precondition (validated here): the upstream URL has NO userinfo and NO query.
/// `Upstream` only ever wraps a wire URL (`http://127.0.0.1:<port>/wire/<secret>/…`)
/// or `https://api.anthropic.com`; neither carries userinfo/query. Validating
/// here guarantees the string composition below cannot produce a malformed URI.
pub fn upstream_target_uri(
    upstream: &Upstream,
    inbound_path_and_query: &str,
) -> anyhow::Result<http::Uri> {
    if !upstream.url.username().is_empty() || upstream.url.password().is_some() {
        anyhow::bail!("upstream URL must not contain userinfo");
    }
    if upstream.url.query().is_some() {
        anyhow::bail!("upstream URL must not contain a query string");
    }
    debug_assert!(
        upstream.url.username().is_empty()
            && upstream.url.password().is_none()
            && upstream.url.query().is_none(),
        "Upstream invariant: no userinfo / no query"
    );

    let scheme = upstream.url.scheme();
    let authority = upstream.host_header();
    let prefix = upstream.path_prefix(); // trailing slash stripped, "" for bare
    let composed = format!("{scheme}://{authority}{prefix}{inbound_path_and_query}");
    composed
        .parse::<http::Uri>()
        .map_err(|e| anyhow::anyhow!("failed to build upstream URI: {e}"))
}

/// Build the reqwest client used for all upstream forwarding.
///
/// TLS uses the `rustls-tls-native-roots` feature pinned in Cargo.toml (R2),
/// which loads the OS / corporate trust store so the final HTTPS hop works
/// behind corporate CAs. We deliberately do NOT use rustls-platform-verifier or
/// `use_preconfigured_tls` (R2). Auto-redirect is disabled: we never want auth
/// headers (`x-api-key`, `authorization`) replayed to a redirected host.
pub fn build_upstream_client() -> anyhow::Result<reqwest::Client> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| anyhow::anyhow!("failed to build upstream client: {e}"))
}

/// First-party (compiled-in) vs external (downloaded) proxy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProxyKind {
    /// pino / headroom — implemented in this crate, run via `poverty-mode proxy`.
    FirstParty,
    /// JB Central — a downloaded third-party binary.
    External,
}

/// The v1 proxy identities.
///
/// Derives `Hash` (plus `Copy`/`Eq`) per R23l so M2 can use it as a `HashSet`
/// key / by value / `==`; `#[serde(rename_all = "lowercase")]` gives the
/// canonical `pino`/`headroom`/`central` wire spellings.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProxyName {
    Pino,
    Headroom,
    Central,
}

impl ProxyName {
    /// The canonical lowercase name used in config, CLI, and the chain spec.
    pub fn as_str(self) -> &'static str {
        match self {
            ProxyName::Pino => "pino",
            ProxyName::Headroom => "headroom",
            ProxyName::Central => "central",
        }
    }

    /// First-party (compiled-in) vs external (downloaded).
    pub fn kind(self) -> ProxyKind {
        match self {
            ProxyName::Pino | ProxyName::Headroom => ProxyKind::FirstParty,
            ProxyName::Central => ProxyKind::External,
        }
    }

    /// Central must always be the tail hop; first-party proxies may sit anywhere.
    pub fn must_be_last(self) -> bool {
        matches!(self, ProxyName::Central)
    }

    /// The health endpoint path: first-party proxies answer `/__pm/health`
    /// locally; central exposes `/health`.
    pub fn health_path(self) -> &'static str {
        match self.kind() {
            ProxyKind::FirstParty => "/__pm/health",
            ProxyKind::External => "/health",
        }
    }
}

impl std::fmt::Display for ProxyName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Error returned when a string does not name a known proxy.
#[derive(Debug, thiserror::Error)]
#[error("unknown proxy name: {0:?} (expected one of pino, headroom, central)")]
pub struct UnknownProxyName(pub String);

impl FromStr for ProxyName {
    type Err = UnknownProxyName;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "pino" => Ok(ProxyName::Pino),
            "headroom" => Ok(ProxyName::Headroom),
            "central" => Ok(ProxyName::Central),
            other => Err(UnknownProxyName(other.to_string())),
        }
    }
}

/// Where a proxy forwards: the next hop or the real upstream. Wraps a [`url::Url`]
/// and provides the helpers the engine needs to rewrite forwarded requests.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Upstream {
    /// The upstream URL (scheme, host, optional explicit port, path prefix).
    pub url: url::Url,
}

impl Upstream {
    /// Parse an upstream from a URL string.
    pub fn parse(s: &str) -> anyhow::Result<Self> {
        let url = url::Url::parse(s)?;
        Ok(Upstream { url })
    }

    /// The value for the forwarded `Host` header: host, plus an explicit
    /// non-default port. The default `:80` (http) / `:443` (https) is elided; an
    /// explicitly specified port (even if it is another scheme's default, e.g.
    /// `:443` on an http URL) is preserved.
    ///
    /// `url::Url::port()` already encodes exactly this distinction: it returns
    /// `None` when the port is absent or equals this scheme's default, and
    /// `Some(p)` only for an explicit non-default port.
    pub fn host_header(&self) -> String {
        let host = self.url.host_str().unwrap_or("");
        match self.url.port() {
            Some(port) => format!("{host}:{port}"),
            None => host.to_string(),
        }
    }

    /// The path prefix to prepend to inbound request paths, with a single
    /// trailing slash stripped. A root path (`/`) yields the empty string, so
    /// `prefix + inbound_path` never doubles the slash.
    pub fn path_prefix(&self) -> String {
        let path = self.url.path();
        let trimmed = path.strip_suffix('/').unwrap_or(path);
        trimmed.to_string()
    }
}

/// Selects which body transform the engine applies, carrying the chosen
/// transform's settings. The concrete `BodyTransform` is materialized from this
/// tag via [`TransformKind::as_body_transform`] (M3 wires the dispatch; M4/M5
/// fill the transforms).
#[derive(Clone, Debug, PartialEq)]
pub enum TransformKind {
    /// Byte-faithful pass-through (no transform).
    None,
    /// pino cache-breakpoint injection, configured by [`pino::PinoSettings`].
    Pino(pino::PinoSettings),
    /// headroom context compression, configured by [`headroom::HeadroomSettings`].
    Headroom(headroom::HeadroomSettings),
}

impl TransformKind {
    /// Construct the concrete `Arc`-shared transform for this kind, or `None`
    /// for `TransformKind::None`. The transform is stored as
    /// `Arc<dyn BodyTransform + Send + Sync>` (R22/R23d) so the engine can clone
    /// the `Arc` into a `spawn_blocking` closure (covers pino's cheap mutation
    /// and headroom's CPU-heavy compress, R20). The transform's `transform` may
    /// currently return `Err` (M1 fail-loud stubs until M4/M5); the engine
    /// forwards the original body and warns on `Err`, never silently corrupting it.
    pub fn as_body_transform(&self) -> Option<std::sync::Arc<dyn BodyTransform + Send + Sync>> {
        match self {
            TransformKind::None => None,
            TransformKind::Pino(settings) => Some(std::sync::Arc::new(pino::PinoTransform {
                settings: settings.clone(),
            })),
            TransformKind::Headroom(settings) => {
                Some(std::sync::Arc::new(headroom::HeadroomTransform {
                    settings: settings.clone(),
                }))
            }
        }
    }
}

/// A request-body mutation applied to a transformed `POST /v1/messages`.
///
/// `transform_bytes` is the byte-fidelity seam (FIX-B): it takes the ORIGINAL
/// request bytes and returns `None` when nothing changed (the engine forwards
/// the original bytes verbatim, preserving the prompt cache byte-for-byte) or
/// `Some(bytes)` to forward exactly those bytes. `transform` is the legacy
/// `Value`-in-place hook the default `transform_bytes` round-trips through;
/// transforms whose output must be byte-surgical (headroom) override
/// `transform_bytes` directly and never touch `serde_json::Value`.
///
/// `apply_headers` is called by the engine AFTER a transform ran (i.e. when
/// `transform_bytes` returned `Some`) and AFTER the Host/Content-Length rewrite,
/// only on a transformed POST `/v1/messages` (R6). The default is a no-op;
/// `PinoTransform` overrides it.
pub trait BodyTransform: Send + Sync {
    /// Mutate the parsed JSON request body in place.
    fn transform(&self, body: &mut serde_json::Value) -> anyhow::Result<()>;

    /// Transform the ORIGINAL request bytes (FIX-B).
    ///
    /// Returns `Ok(None)` when there is NO change — the engine forwards the
    /// original request bytes verbatim (byte-faithful, prompt-cache-preserving).
    /// Returns `Ok(Some(bytes))` to forward exactly `bytes`.
    ///
    /// The default implementation parses to a `serde_json::Value`, runs
    /// [`BodyTransform::transform`], and re-serializes — a canonicalizing
    /// round-trip that is acceptable where cross-turn consistency (not raw
    /// byte-fidelity) is what the prompt cache needs (pino). Transforms that
    /// must preserve the upstream's cache-hot bytes exactly (headroom's
    /// byte-surgical output) OVERRIDE this and never round-trip through `Value`.
    fn transform_bytes(&self, raw: &[u8]) -> anyhow::Result<Option<Vec<u8>>> {
        let mut value: serde_json::Value = serde_json::from_slice(raw)?;
        self.transform(&mut value)?;
        Ok(Some(serde_json::to_vec(&value)?))
    }

    /// Adjust outbound headers after a transform. Default: no-op (R6).
    fn apply_headers(&self, _headers: &mut http::HeaderMap) {}
}

/// Everything one proxy instance (engine) needs to run. `run_id` is the per-run
/// ULID (identity, reported by health + READY) — NOT the chain spec, which the
/// orchestrator carries separately in `POVERTY_PROXY_CHAIN` (R10).
#[derive(Clone, Debug)]
pub struct EngineConfig {
    /// Which proxy this instance is.
    pub name: ProxyName,
    /// The loopback bind address (`HOST:0` for an OS-assigned port).
    pub listen: SocketAddr,
    /// Where to forward.
    pub upstream: Upstream,
    /// Per-run ULID shared by all hops of one run (identity / staleness).
    pub run_id: String,
    /// Optional per-instance body log file.
    pub log_file: Option<PathBuf>,
    /// Which body transform to apply.
    pub transform: TransformKind,
}

/// The single structured line a `proxy` child prints to stdout once bound, read
/// by the orchestrator as a blocking pipe read (R10/§9).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReadyLine {
    /// Always `true` on a successful bind.
    pub ready: bool,
    /// The OS-assigned (or configured) bound port.
    pub port: u16,
    /// The proxy name (e.g. `"pino"`).
    pub proxy: String,
    /// The per-run ULID (identity).
    pub run_id: String,
}

/// The body of `GET /__pm/health`: readiness + identity + staleness (R10/§5.4).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HealthBody {
    /// The proxy name.
    pub proxy: String,
    /// The bound port.
    pub port: u16,
    /// The upstream `host:port` (never the wire secret/path — §15).
    pub upstream: String,
    /// The per-run ULID (identity).
    pub run_id: String,
}

/// Internal per-connection shared context.
struct EngineState {
    name: ProxyName,
    port: u16,
    upstream: Upstream,
    run_id: String,
    client: reqwest::Client,
    // R22/R23d: the engine stores `Arc<dyn BodyTransform + Send + Sync>` so the
    // forward path can clone the `Arc` into a `tokio::task::spawn_blocking`
    // closure (covers pino's cheap mutation AND headroom's CPU-heavy compress,
    // R20). M5 reuses this exact field and does NOT re-type it. A `Box` would
    // foreclose that design (it cannot be moved into a 'static blocking closure
    // and still be reused), so M3 authors `Arc` now.
    transform: Option<Arc<dyn BodyTransform + Send + Sync>>,
    // M3.11: the optional per-response log-tee destination (append mode).
    log_file: Option<std::path::PathBuf>,
}

/// A bound, serving engine. `local_addr` is the real bound address (ephemeral
/// port resolved); `handle` resolves when the serve loop drains and exits.
pub struct BoundEngine {
    pub local_addr: SocketAddr,
    pub handle: tokio::task::JoinHandle<anyhow::Result<()>>,
}

/// Bind the listener, print the ReadyLine to stdout, and spawn the serve loop.
/// Returns the real bound address immediately (so an in-process caller learns
/// the ephemeral port without polling). `shutdown` resolving begins a graceful
/// drain; the returned `handle` resolves once all in-flight connections finish.
pub async fn bind_engine(
    cfg: EngineConfig,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> anyhow::Result<BoundEngine> {
    let listener = TcpListener::bind(cfg.listen).await?;
    let local_addr = listener.local_addr()?;
    let port = local_addr.port();

    // Complete ALL fallible initialization BEFORE the ReadyLine: `ready:true` is
    // the orchestrator's blocking sync point (R10) and must mean the engine is
    // actually serving. Building the upstream client is the only fallible step
    // here (`as_body_transform` is infallible), so it precedes `print_ready_line`.
    let state = Arc::new(EngineState {
        name: cfg.name,
        port,
        upstream: cfg.upstream,
        run_id: cfg.run_id,
        client: build_upstream_client()?,
        transform: cfg.transform.as_body_transform(),
        log_file: resolve_log_file(cfg.log_file, port),
    });

    // Print exactly one ReadyLine to stdout (compact JSON + newline + flush)
    // AFTER successful bind + init and BEFORE accepting connections. This is the
    // synchronization point the orchestrator reads as a blocking pipe read.
    print_ready_line(&state.name, port, &state.run_id)?;

    let handle = tokio::spawn(async move { serve_loop(listener, state, shutdown).await });
    Ok(BoundEngine { local_addr, handle })
}

/// Deterministic transform for integration tests: sets `body["__pm_test"]=true`
/// in `transform`, and `x-pm-marker: applied` in `apply_headers`. Only compiled
/// with the `test-transforms` feature. Proves the engine runs both the body
/// transform and the R6 header hook on a transformed POST /v1/messages.
#[cfg(feature = "test-transforms")]
pub struct MarkerTransform;

#[cfg(feature = "test-transforms")]
impl BodyTransform for MarkerTransform {
    fn transform(&self, body: &mut serde_json::Value) -> anyhow::Result<()> {
        if let Some(obj) = body.as_object_mut() {
            obj.insert("__pm_test".to_string(), serde_json::Value::Bool(true));
        }
        Ok(())
    }

    fn apply_headers(&self, headers: &mut http::HeaderMap) {
        headers.insert("x-pm-marker", http::HeaderValue::from_static("applied"));
    }
}

/// Test seam: bind an engine whose transform is an explicitly-injected
/// `Arc`-shared transform, bypassing `TransformKind`. Only compiled with
/// `test-transforms`. Mirrors `bind_engine` exactly except it injects the
/// transform directly. The parameter is `Arc<dyn BodyTransform + Send + Sync>`
/// to match the engine field type (R22/R23d).
#[cfg(feature = "test-transforms")]
pub async fn bind_engine_with_boxed_transform(
    cfg: EngineConfig,
    transform: Arc<dyn BodyTransform + Send + Sync>,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> anyhow::Result<BoundEngine> {
    let listener = TcpListener::bind(cfg.listen).await?;
    let local_addr = listener.local_addr()?;
    let port = local_addr.port();
    print_ready_line(&cfg.name, port, &cfg.run_id)?;
    let state = Arc::new(EngineState {
        name: cfg.name,
        port,
        upstream: cfg.upstream,
        run_id: cfg.run_id,
        client: build_upstream_client()?,
        transform: Some(transform),
        log_file: resolve_log_file(cfg.log_file, port),
    });
    let handle = tokio::spawn(async move { serve_loop(listener, state, shutdown).await });
    Ok(BoundEngine { local_addr, handle })
}

/// Resolve a body-log path that may carry the `{port}` placeholder.
///
/// The orchestrator builds each hop's body-log path BEFORE the OS assigns the
/// ephemeral port, so it embeds the literal token `{port}`; only the engine knows
/// the real bound port. Substituting here makes the on-disk file land at the
/// design-spec §5.11 name `<state>/runs/<run-id>/<proxy>-<port>.log` that
/// `status::enumerate_runs` parses. A literal path (no token — e.g. standalone
/// `poverty-mode proxy ... --body-log-file FILE`) passes through unchanged.
fn resolve_log_file(path: Option<PathBuf>, port: u16) -> Option<PathBuf> {
    path.map(|p| {
        let s = p.to_string_lossy();
        if s.contains("{port}") {
            PathBuf::from(s.replace("{port}", &port.to_string()))
        } else {
            p
        }
    })
}

fn print_ready_line(name: &ProxyName, port: u16, run_id: &str) -> anyhow::Result<()> {
    let ready = ReadyLine {
        ready: true,
        port,
        proxy: name.as_str().to_string(),
        run_id: run_id.to_string(),
    };
    let stdout = std::io::stdout();
    let mut lock = stdout.lock();
    serde_json::to_writer(&mut lock, &ready)?;
    lock.write_all(b"\n")?;
    lock.flush()?;
    Ok(())
}

async fn serve_loop(
    listener: TcpListener,
    state: Arc<EngineState>,
    shutdown: impl std::future::Future<Output = ()> + Send,
) -> anyhow::Result<()> {
    let graceful = GracefulShutdown::new();
    // http1-only Builder: the locked manifest (R2/R23a) enables hyper
    // ["server","http1"] and hyper-util ["tokio","server","server-graceful"] —
    // NOT server-auto/http2 — so we use the http1 connection server (matching the
    // canonical stub). `serve_connection` returns an owned `http1::Connection`
    // (no borrow of the builder), which directly implements `GracefulConnection`,
    // so `graceful.watch(conn)` wraps it without an `into_owned()` step.
    let builder = hyper::server::conn::http1::Builder::new();
    tokio::pin!(shutdown);

    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, _peer) = match accepted {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let io = TokioIo::new(stream);
                let st = state.clone();
                let svc = service_fn(move |req: Request<Incoming>| {
                    let st = st.clone();
                    async move { handle_request(st, req).await }
                });
                let conn = builder.serve_connection(io, svc);
                let watched = graceful.watch(conn);
                tokio::spawn(async move {
                    let _ = watched.await;
                });
            }
            _ = &mut shutdown => {
                // Stop accepting; drain in-flight connections to completion.
                break;
            }
        }
    }
    // Await all in-flight connection futures. No timeout: the only human-surfaced
    // numeric bound lives on the orchestrator's readiness deadline (M6), never here.
    graceful.shutdown().await;
    Ok(())
}

/// The engine's response/forward body type: a boxed body streaming `Bytes`
/// with `std::io::Error` as its error, so we can mix buffered (`Full`) and
/// streamed (`StreamBody`) bodies behind one type.
type EngineBody = BoxBody<Bytes, std::io::Error>;

fn full_body(bytes: Bytes) -> EngineBody {
    Full::new(bytes).map_err(|never| match never {}).boxed()
}

/// Stream a request body (`Incoming`) through untouched as a `reqwest::Body`,
/// without buffering. Used on the non-transform path (R11).
fn stream_request_body(body: Incoming) -> reqwest::Body {
    let stream = body.into_data_stream().map_err(std::io::Error::other);
    reqwest::Body::wrap_stream(stream)
}

/// Request handler: answer the local health probe; otherwise forward upstream.
///
/// Returns `Result<_, Infallible>` (R23e): hyper-util's `serve_connection`
/// requires `S::Error: Into<Box<dyn std::error::Error + Send + Sync>>`, which
/// `anyhow::Error` does NOT satisfy. Every internal failure is converted to a
/// local 502 response, never propagated as the service error.
async fn handle_request(
    state: Arc<EngineState>,
    req: Request<Incoming>,
) -> Result<Response<EngineBody>, Infallible> {
    if req.method() == hyper::Method::GET && is_health_path(req.uri().path(), state.name) {
        return Ok(health_response(&state));
    }
    forward(state, req).await
}

/// True when `path` is this proxy's local health path (`/__pm/health` for
/// first-party, `/health` for central). Named for what it checks (not "messages").
fn is_health_path(path: &str, name: ProxyName) -> bool {
    path == name.health_path()
}

fn health_response(state: &EngineState) -> Response<EngineBody> {
    let body = HealthBody {
        proxy: state.name.as_str().to_string(),
        port: state.port,
        upstream: state.upstream.host_header(),
        run_id: state.run_id.clone(),
    };
    let json = serde_json::to_vec(&body).unwrap_or_else(|_| b"{}".to_vec());
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "application/json")
        .header("x-pm-proxy", state.name.as_str())
        .body(full_body(Bytes::from(json)))
        .unwrap()
}

/// Build a local 502 response carrying an explanatory plaintext body. Used to
/// convert EVERY internal `forward` error into a response (R23e) — the hyper
/// service error type is `Infallible`, so we never `?`-propagate.
fn bad_gateway(detail: impl std::fmt::Display) -> Response<EngineBody> {
    Response::builder()
        .status(StatusCode::BAD_GATEWAY)
        .header("content-type", "text/plain")
        .body(full_body(Bytes::from(format!(
            "proxy upstream error: {detail}"
        ))))
        .unwrap()
}

/// Forward an inbound request to the upstream. Non-transform requests stream the
/// body through untouched (R11); only `should_transform` (POST + messages path +
/// JSON) buffers, runs the transform on a `spawn_blocking` worker (R22/R23d/R20),
/// and recomputes Content-Length. Host is rewritten and auth headers pass through
/// verbatim. Every internal error becomes a local 502 (R23e) — nothing is
/// `?`-propagated as the hyper service error.
async fn forward(
    state: Arc<EngineState>,
    req: Request<Incoming>,
) -> Result<Response<EngineBody>, Infallible> {
    let method = req.method().clone();
    let path_and_query = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str().to_string())
        .unwrap_or_else(|| req.uri().path().to_string());
    let inbound_headers = req.headers().clone();

    let should_transform = method == hyper::Method::POST
        && is_messages_path(&path_and_query)
        && is_json_content_type(&inbound_headers);

    // Convert each fallible step into a local 502 (R23e); never `?`-propagate.
    let target = match upstream_target_uri(&state.upstream, &path_and_query) {
        Ok(t) => t,
        Err(e) => return Ok(bad_gateway(format!("bad upstream URI: {e}"))),
    };
    let reqwest_method = match reqwest::Method::from_bytes(method.as_str().as_bytes()) {
        Ok(m) => m,
        Err(e) => return Ok(bad_gateway(format!("bad method: {e}"))),
    };

    // Build upstream headers: copy inbound verbatim, then rewrite Host. Auth
    // headers (x-api-key, authorization, anthropic-beta) are copied as-is.
    let mut headers = inbound_headers.clone();
    headers.remove(http::header::HOST);
    let host_value = match http::HeaderValue::from_str(&state.upstream.host_header()) {
        Ok(v) => v,
        Err(e) => return Ok(bad_gateway(format!("bad host header: {e}"))),
    };
    headers.insert(http::header::HOST, host_value);

    let req_builder = if should_transform {
        // Buffer + (optionally) transform, then recompute Content-Length exactly.
        let body_bytes = match req.into_body().collect().await {
            Ok(b) => b.to_bytes(),
            Err(e) => return Ok(bad_gateway(format!("reading request body: {e}"))),
        };
        // FIX-B byte-fidelity seam: keep the ORIGINAL request bytes; only
        // replace them when `transform_bytes` returns `Some`. `None` forwards
        // the original bytes VERBATIM (headroom byte-surgical output and pino's
        // true passthrough both ride this), so the cache-hot zone is never
        // canonicalized through `serde_json::Value`.
        let mut out_body: Vec<u8> = body_bytes.to_vec();
        let mut did_transform = false;

        if !out_body.is_empty() {
            if let Some(transform) = state.transform.as_ref() {
                // R22/R23d/R20: run the transform on a blocking worker via an
                // `Arc` clone so CPU-heavy transforms (headroom compress) never
                // block the executor. The closure returns the transform's
                // byte-level decision (None = no change, Some = new bytes) or
                // its error.
                let transform = transform.clone();
                let raw = out_body.clone();
                let outcome =
                    tokio::task::spawn_blocking(move || transform.transform_bytes(&raw)).await;
                match outcome {
                    Ok(Ok(Some(bytes))) => {
                        out_body = bytes;
                        did_transform = true;
                    }
                    Ok(Ok(None)) => {
                        // No change: forward the original bytes verbatim.
                    }
                    Ok(Err(e)) => {
                        // Fail-loud-in-logs, byte-faithful body (matches
                        // reference "WARN transform threw, skipping"). Never
                        // forwards a half-transformed body, never `?`-propagates
                        // a transform error to the client.
                        tracing::warn!("transform failed, forwarding original body: {e}");
                    }
                    Err(e) => {
                        tracing::warn!("transform task join failed, forwarding original body: {e}");
                    }
                }
            }
        }

        headers.remove(http::header::CONTENT_LENGTH);
        let cl = match http::HeaderValue::from_str(&out_body.len().to_string()) {
            Ok(v) => v,
            Err(e) => return Ok(bad_gateway(format!("bad content-length: {e}"))),
        };
        headers.insert(http::header::CONTENT_LENGTH, cl);

        // R6 header hook: only on a transformed POST /v1/messages, AFTER the
        // body transform and AFTER the Host/Content-Length rewrite.
        if did_transform {
            if let Some(transform) = state.transform.as_ref() {
                transform.apply_headers(&mut headers);
            }
        }

        let mut b = state
            .client
            .request(reqwest_method, target.to_string())
            .body(out_body);
        for (name, value) in headers.iter() {
            b = b.header(name.as_str(), value.as_bytes());
        }
        b
    } else {
        // Stream-through (R11): forward the request body untouched, no buffering.
        // Drop any inbound content-length so reqwest sets the correct framing for
        // the streamed body (it may be chunked).
        headers.remove(http::header::CONTENT_LENGTH);
        let streamed = stream_request_body(req.into_body());
        let mut b = state
            .client
            .request(reqwest_method, target.to_string())
            .body(streamed);
        for (name, value) in headers.iter() {
            b = b.header(name.as_str(), value.as_bytes());
        }
        b
    };

    match req_builder.send().await {
        Ok(up_resp) => Ok(build_downstream_response(state, up_resp).await),
        Err(e) => {
            tracing::warn!("upstream error: {e}");
            Ok(bad_gateway(e))
        }
    }
}

/// Stream the upstream response back to the client, forwarding ALL upstream
/// response headers verbatim (reference server.js:149) and streaming the body
/// (SSE-safe). Infallible: a malformed status/header is logged and skipped, never
/// `?`-propagated (R23e). Async so M3.11 can open the optional log-tee file here.
async fn build_downstream_response(
    state: Arc<EngineState>,
    up_resp: reqwest::Response,
) -> Response<EngineBody> {
    let status = up_resp.status();
    let resp_headers = up_resp.headers().clone();

    // Open the tee file (append) once per response if configured.
    let tee = match state.log_file.as_ref() {
        Some(path) => {
            if let Some(parent) = path.parent() {
                if let Err(e) = tokio::fs::create_dir_all(parent).await {
                    tracing::warn!("tee: cannot create log dir {}: {e}", parent.display());
                }
            }
            // This file holds full request/response bodies (the most sensitive
            // on-disk artifact), so on POSIX it must be owner-only (0600) like
            // every other file we write (paths::atomic_write / ensure_run_dir).
            // `.mode(0o600)` is applied AT creation so a freshly-created file is
            // born owner-only — no world-readable TOCTOU window between create
            // and chmod. It is ignored when the file already exists; the
            // `harden_file_perms` call below tightens that pre-existing/append
            // case. No-op on Windows (no POSIX mode bits).
            let mut opts = tokio::fs::OpenOptions::new();
            opts.create(true).append(true);
            #[cfg(unix)]
            opts.mode(0o600);
            match opts.open(path).await {
                Ok(file) => {
                    // Warn (never fail the response) if hardening fails.
                    if let Err(e) = crate::paths::harden_file_perms(path) {
                        tracing::warn!("tee: cannot harden log file {}: {e}", path.display());
                    }
                    Some(Arc::new(tokio::sync::Mutex::new(file)))
                }
                Err(e) => {
                    tracing::warn!("tee: cannot open log file {}: {e}", path.display());
                    None
                }
            }
        }
        None => None,
    };

    let tee_for_stream = tee.clone();
    let framed = up_resp.bytes_stream().then(move |chunk| {
        let tee = tee_for_stream.clone();
        async move {
            match chunk {
                Ok(bytes) => {
                    if let Some(tee) = tee.as_ref() {
                        let mut f = tee.lock().await;
                        // Observable failures (MINOR finding): warn, never panic,
                        // never fail the response on a log error.
                        if let Err(e) = f.write_all(&bytes).await {
                            tracing::warn!("tee: write failed: {e}");
                        } else if let Err(e) = f.flush().await {
                            tracing::warn!("tee: flush failed: {e}");
                        }
                    }
                    Ok(Frame::data(bytes))
                }
                Err(e) => Err(std::io::Error::other(e)),
            }
        }
    });
    // `BodyExt::boxed` (not `StreamExt::boxed`): `StreamBody` is both a `Body`
    // and a `Stream`, and importing `StreamExt` (for `.then`) makes the bare
    // `.boxed()` ambiguous. We want the `Body` → `BoxBody<Bytes, io::Error>`.
    let body = BodyExt::boxed(StreamBody::new(framed));

    // Infallible (R23e): a malformed status/header is logged-skipped, never
    // `?`-propagated to the hyper service error.
    let status = StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let mut builder = Response::builder().status(status);
    for (name, value) in resp_headers.iter() {
        // Forward upstream response headers verbatim. We intentionally do NOT
        // selectively drop content-length/transfer-encoding (R11): hyper
        // re-frames the streamed body and ignores a stale content-length we
        // pass, and forwarding verbatim matches the reference pino. Strip only
        // the genuinely connection-scoped `connection` header.
        if name.as_str().eq_ignore_ascii_case("connection") {
            continue;
        }
        builder = builder.header(name.as_str(), value.as_bytes());
    }
    builder
        .body(body)
        .unwrap_or_else(|_| bad_gateway("failed to build downstream response"))
}

/// Run the engine to completion, draining on OS signal. Public entry per contract.
pub async fn run_proxy(cfg: EngineConfig) -> anyhow::Result<()> {
    run_proxy_with_shutdown(cfg, default_shutdown_signal()).await
}

/// Test/orchestrator seam: run the engine, draining when `shutdown` resolves.
pub async fn run_proxy_with_shutdown(
    cfg: EngineConfig,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> anyhow::Result<()> {
    let bound = bind_engine(cfg, shutdown).await?;
    bound.handle.await??;
    Ok(())
}

/// The real OS shutdown signal: Ctrl-C everywhere; SIGTERM additionally on Unix.
async fn default_shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut term = signal(SignalKind::terminate()).expect("install SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

#[cfg(test)]
#[path = "proxy_tests.rs"]
mod proxy_tests;
