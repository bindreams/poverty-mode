//! Foundational proxy types shared by every later milestone (R9).
//!
//! These definitions are created once here; later milestones FILL behavior (the
//! async engine in M3, the pino/headroom transforms in M4/M5) but never redefine
//! these types. `proxy::pino` and `proxy::headroom` (this module's submodules)
//! hold per-proxy settings + transforms.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

pub mod headroom;
pub mod pino;

/// True for `/v1/messages`, `/v1/messages/count_tokens`, and either of those
/// followed by a `?query`. Mirrors the upstream pino `isMessagesPath`
/// (reference/pino/src/server.js:27-34).
pub fn is_messages_path(path: &str) -> bool {
    let base = path.split('?').next().unwrap_or("");
    base == "/v1/messages" || base == "/v1/messages/count_tokens"
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

/// Selects which body transform the engine applies. The concrete transform is
/// chosen from this tag (M3 wires the dispatch; M4/M5 fill the transforms).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransformKind {
    /// Byte-faithful pass-through (no transform).
    None,
    /// pino cache-breakpoint injection.
    Pino,
    /// headroom context compression.
    Headroom,
}

/// A request-body mutation applied to a transformed `POST /v1/messages`.
///
/// `transform` mutates the parsed JSON body in place. `apply_headers` is called
/// by the engine AFTER `transform()` and AFTER the Host/Content-Length rewrite,
/// only on a transformed POST `/v1/messages` (R6). The default is a no-op;
/// `PinoTransform` overrides it (the override is filled in M4).
pub trait BodyTransform: Send + Sync {
    /// Mutate the parsed JSON request body in place.
    fn transform(&self, body: &mut serde_json::Value) -> anyhow::Result<()>;

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

#[cfg(test)]
#[path = "proxy_tests.rs"]
mod proxy_tests;
