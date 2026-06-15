//! The agent seam: the AI agent process the chain fronts.
//!
//! The orchestrator (M6) drives any agent through this trait so the concrete
//! agent (v1: `claude`, filled in by M7's `agent::claude::ClaudeAgent`) is just
//! one impl. The trait is defined here, in M6, because `build_and_run` takes a
//! `&dyn Agent`; M7 adds the `claude` submodule and the inline `--settings`
//! wiring without re-typing the trait.

pub mod claude;
pub mod codex;

use url::Url;

/// An AI agent the proxy chain fronts.
///
/// `build_command` returns a fully-prepared, not-yet-spawned child command. Each
/// agent points ITSELF at `base_url` by its own mechanism (Claude:
/// `ANTHROPIC_BASE_URL` + inline `--settings`; codex: a `-c` model-provider
/// override). `base_url` is the composed agent base (chain head + the agent's
/// wire-client segment when central is the tail; see `orchestrator::agent_base_for`),
/// NOT carried in `extra_env`. Agents mirror the relevant `extra_env` pairs into
/// the child process environment. `argv` is the user's pass-through agent arguments.
pub trait Agent {
    /// A short, stable identifier for diagnostics (e.g. `"claude"`).
    fn name(&self) -> &str;

    /// The central-wire client/api path segment this agent's requests carry into
    /// the chain (C1), e.g. `"claude-code/anthropic"` or `"codex/openai"`. The
    /// orchestrator appends it to the agent-agnostic head when central is the tail.
    /// Default is Claude's segment (Claude was the only agent before codex).
    fn wire_client_path(&self) -> &str {
        "claude-code/anthropic"
    }

    /// True iff this agent only works with JetBrains Central as the chain tail
    /// (its wire client/api segment is a Central concept). Default false.
    fn requires_central(&self) -> bool {
        false
    }

    /// Build the child command for this agent, pointed at `base_url` with
    /// `extra_env` applied. Does not spawn — the caller runs it.
    fn build_command(
        &self,
        argv: &[String],
        base_url: &Url,
        extra_env: &[(String, String)],
    ) -> tokio::process::Command;
}

use crate::agent::claude::ClaudeAgent;
use crate::agent::codex::CodexAgent;

/// Pick the agent adapter from the user's pass-through program (`run -- <prog> …`).
/// Match on `argv[0]`'s basename, lowercased, with a trailing `.exe` stripped
/// (Windows is case-insensitive): `codex` → [`CodexAgent`], everything else
/// (including empty argv) → [`ClaudeAgent`].
pub fn select_agent(argv: &[String]) -> Box<dyn Agent> {
    let base = argv
        .first()
        .map(|p| {
            let stem = p.rsplit(['/', '\\']).next().unwrap_or(p.as_str());
            let lower = stem.to_ascii_lowercase();
            lower.strip_suffix(".exe").unwrap_or(&lower).to_string()
        })
        .unwrap_or_else(|| "claude".to_string());
    match base.as_str() {
        "codex" => Box::new(CodexAgent),
        _ => Box::new(ClaudeAgent),
    }
}

// Characterization guard (R12): the `Agent` trait already exists (M6 typed it so
// `build_and_run` could take `&dyn Agent`). These tests lock its object-safe
// shape — usable through a trait object, `build_command` reachable via `&dyn` —
// before `ClaudeAgent` is exercised by M7, so an accidental signature change that
// would break M6's call sites is caught here.
#[cfg(test)]
#[path = "agent_tests.rs"]
mod agent_tests;
