//! JB Central: the downloaded shared singleton that always runs last in the
//! chain. M8 fills install / login / start / health / stop; this module currently
//! provides only the two items the orchestrator (M6) consumes — the started
//! `CentralInfo` (port + wire secret) and `central_wire_upstream`, which renders
//! the JetBrains wire URL the pre-central hop (or a central-only agent) targets.

use crate::proxy::Upstream;

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
    let url = url::Url::parse(&format!(
        "http://127.0.0.1:{}/wire/{}/claude-code/anthropic",
        info.port, info.secret
    ))
    .expect("central wire URL is well-formed");
    Upstream { url }
}

#[cfg(test)]
#[path = "central_tests.rs"]
mod central_tests;
