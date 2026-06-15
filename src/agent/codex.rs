//! The `codex` agent: OpenAI's codex CLI fronted by the proxy chain.
//!
//! Codex points at a base URL via a config-provider override on the CLI
//! (`-c model_providers.<id>.base_url=…`), not via an env var — so this adapter's
//! belt is a self-contained `-c` provider injected before the user's args, the
//! direct analog of `ClaudeAgent`'s `--settings`. The base URL it receives is the
//! agent-agnostic head already suffixed with the codex wire-client segment
//! (`/codex/openai`, composed by `orchestrator::agent_base_for`); codex appends
//! `/responses` (its Responses-API wire path). No auth key is set: JB Central's
//! wire proxy injects the JWT, exactly as for the user's own keyless `wire` provider.

use url::Url;

use crate::agent::Agent;

/// The poverty-mode-owned codex provider id (a bare TOML key: no hyphen). Shared
/// with the in-repo `__codexpost` test stub so the producer and consumer of the
/// `-c …base_url=` override cannot drift.
pub const PROVIDER: &str = "povertymode";

/// The codex agent implementation. Zero-sized.
pub struct CodexAgent;

impl Agent for CodexAgent {
    fn name(&self) -> &str {
        "codex"
    }

    fn wire_client_path(&self) -> &str {
        "codex/openai"
    }

    fn requires_central(&self) -> bool {
        true
    }

    fn build_command(
        &self,
        argv: &[String],
        base_url: &Url,
        extra_env: &[(String, String)],
    ) -> tokio::process::Command {
        let (program, rest): (&str, &[String]) = match argv.split_first() {
            Some((program, rest)) => (program.as_str(), rest),
            None => (self.name(), &[]),
        };
        let mut cmd = tokio::process::Command::new(program);

        // Belt: a self-contained provider, injected at top level BEFORE any
        // subcommand (`exec`) or user args. `-c` values are TOML; emit quoted
        // strings so they parse deterministically (not via codex's raw fallback).
        cmd.arg("-c").arg(format!("model_provider=\"{PROVIDER}\""));
        cmd.arg("-c")
            .arg(format!("model_providers.{PROVIDER}.name=\"poverty-mode\""));
        cmd.arg("-c")
            .arg(format!("model_providers.{PROVIDER}.base_url=\"{}\"", base_url.as_str()));
        cmd.arg("-c")
            .arg(format!("model_providers.{PROVIDER}.wire_api=\"responses\""));

        cmd.args(rest);

        // Process env: mirror only the POVERTY_PROXY_* keys (so a nested codex can
        // still reuse the live chain). The Claude-specific keys the orchestrator
        // also emits (ENABLE_TOOL_SEARCH, ANTHROPIC_AUTH_TOKEN) are intentionally
        // NOT propagated to codex — codex uses our injected provider, not anthropic.
        for (k, v) in extra_env {
            if k.starts_with("POVERTY_PROXY_") {
                cmd.env(k, v);
            }
        }
        cmd
    }
}

#[cfg(test)]
#[path = "codex_tests.rs"]
mod codex_tests;
