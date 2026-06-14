//! JB Central: the downloaded shared singleton that always runs last in the
//! chain. M8 fills install / login / start / health / stop; this module currently
//! provides the items the orchestrator (M6) consumes — the started `CentralInfo`
//! (port + wire secret) and `central_wire_upstream`, which renders the JetBrains
//! wire URL the pre-central hop (or a central-only agent) targets — plus the M8.5
//! constants (R4) and `~/.wire/config.json` parsing.
//!
//! **R5 contract:** every function here that does filesystem I/O (`read_wire_config`)
//! — and, as later M8 tasks fill them, every function that shells out or hits the
//! network — is synchronous/blocking. Callers in an async context (the orchestrator,
//! M6) MUST invoke them via `tokio::task::spawn_blocking`.

use std::path::PathBuf;

use anyhow::{anyhow, bail, Context};
use percent_encoding::{utf8_percent_encode, AsciiSet, CONTROLS};

use crate::proxy::Upstream;

/// The default jbcentral version this build manages (R4). Single source of truth.
pub const DEFAULT_JBCENTRAL_VERSION: &str = "0.2.9";

/// The install-dir name under `<cache>/bin/` (R4). Shared with M10 status/clean — never `central`.
pub const INSTALL_TOOL_DIR: &str = "jbcentral";

/// Characters that must be percent-encoded so the wire secret stays a single,
/// faithful path segment (R20). Beyond the C0 controls, this encodes the path
/// terminators (`?`, `#`), the segment separator (`/`), space, and every other
/// generic-URI delimiter — so an arbitrary secret from `~/.wire/config.json`
/// cannot become a fragment, a query, or an extra path component.
const WIRE_SECRET_SET: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'/')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'`')
    .add(b'{')
    .add(b'}')
    .add(b':')
    .add(b'@')
    .add(b'[')
    .add(b']')
    .add(b'\\')
    .add(b'^')
    .add(b'|')
    .add(b'&')
    .add(b'=')
    .add(b'+')
    .add(b'$')
    .add(b',')
    .add(b';');

/// What `central::start` reports once central is running: the loopback port it
/// bound and the wire secret read from `~/.wire/config.json` (design §6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CentralInfo {
    /// The loopback port central bound.
    pub port: u16,
    /// The wire secret central injects into its path prefix.
    pub secret: String,
}

/// The wire URL string that fronts JB Central:
/// `http://127.0.0.1:<port>/wire/<percent-encoded-secret>/claude-code/anthropic` (design §6).
/// This is the upstream the hop before central uses (or the agent base for a central-only chain).
/// The externally-sourced secret is percent-encoded as one path segment so URL-significant
/// characters cannot escape the path into a query, fragment, or extra segment. Never logged.
pub fn central_wire_url(info: &CentralInfo) -> String {
    let secret = utf8_percent_encode(&info.secret, WIRE_SECRET_SET);
    format!(
        "http://127.0.0.1:{}/wire/{secret}/claude-code/anthropic",
        info.port
    )
}

/// The wire upstream the chain forwards to when central is the tail, as a parsed [`Upstream`]
/// for direct use as a proxy upstream. The pre-central hop carries this as its `--upstream`; in a
/// central-only chain the agent's `ANTHROPIC_BASE_URL` points here directly. Returns an error
/// (never panics) if, against expectation, the encoded URL fails to parse.
pub fn central_wire_upstream(info: &CentralInfo) -> anyhow::Result<Upstream> {
    let s = central_wire_url(info);
    let url =
        url::Url::parse(&s).with_context(|| "constructing the JB Central wire upstream URL")?;
    Ok(Upstream { url })
}

/// Parse the contents of `~/.wire/config.json` into a [`CentralInfo`].
///
/// Fails closed (error, never a default) when the file is unparseable or missing fields, so the
/// caller never silently bypasses wire. The error message never echoes the raw JSON (it may carry the
/// secret): on a parse failure we emit a fixed string and do NOT interpolate the serde error, which
/// could contain a fragment of the input. Some jbcentral builds write `proxy_port` as a string, so a
/// numeric-string port is coerced.
pub fn parse_wire_config(contents: &str) -> anyhow::Result<CentralInfo> {
    let value: serde_json::Value = serde_json::from_str(contents)
        .map_err(|_| anyhow!("~/.wire/config.json is not valid JSON"))?;

    let port = match value.get("proxy_port") {
        Some(serde_json::Value::Number(n)) => n
            .as_u64()
            .and_then(|v| u16::try_from(v).ok())
            .ok_or_else(|| anyhow!("proxy_port out of u16 range"))?,
        Some(serde_json::Value::String(s)) => s
            .trim()
            .parse::<u16>()
            .map_err(|_| anyhow!("proxy_port string is not a u16"))?,
        Some(_) => bail!("proxy_port has an unexpected type"),
        None => bail!("~/.wire/config.json missing \"proxy_port\""),
    };

    let secret = match value.get("proxy_secret") {
        Some(serde_json::Value::String(s)) if !s.is_empty() => s.clone(),
        Some(_) => bail!("proxy_secret has an unexpected type or is empty"),
        None => bail!("~/.wire/config.json missing \"proxy_secret\""),
    };

    Ok(CentralInfo { port, secret })
}

/// Default location of the wire config: `~/.wire/config.json`.
pub fn wire_config_path() -> anyhow::Result<PathBuf> {
    let home = directories::BaseDirs::new()
        .ok_or_else(|| anyhow!("cannot determine home directory"))?
        .home_dir()
        .to_path_buf();
    Ok(home.join(".wire").join("config.json"))
}

/// Read + parse `~/.wire/config.json`. Blocking filesystem I/O (R5).
pub fn read_wire_config() -> anyhow::Result<CentralInfo> {
    let path = wire_config_path()?;
    let contents =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    parse_wire_config(&contents)
}

#[cfg(test)]
#[path = "central_tests.rs"]
mod central_tests;
