//! JB Central: the downloaded shared singleton that always runs last in the
//! chain. M8 fills install / login / start / health / stop; this module currently
//! provides only the two items the orchestrator (M6) consumes — the started
//! `CentralInfo` (port + wire secret) and `central_wire_upstream`, which renders
//! the JetBrains wire URL the pre-central hop (or a central-only agent) targets.

use crate::proxy::Upstream;
use percent_encoding::{utf8_percent_encode, AsciiSet, CONTROLS};

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

/// The wire upstream the chain forwards to when central is the tail:
/// `http://127.0.0.1:<port>/wire/<secret>/claude-code/anthropic` (design §6).
/// The pre-central hop carries this as its `--upstream`; in a central-only chain
/// the agent's `ANTHROPIC_BASE_URL` points here directly.
pub fn central_wire_upstream(info: &CentralInfo) -> Upstream {
    let secret = utf8_percent_encode(&info.secret, WIRE_SECRET_SET);
    let url = url::Url::parse(&format!(
        "http://127.0.0.1:{}/wire/{secret}/claude-code/anthropic",
        info.port
    ))
    .expect("central wire URL is well-formed");
    Upstream { url }
}

#[cfg(test)]
#[path = "central_tests.rs"]
mod central_tests;
