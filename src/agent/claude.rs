//! The v1 `claude` agent: the AI coding agent the proxy chain fronts.
//!
//! M6 needs a concrete [`Agent`] so the orchestrator's `run_command` can spawn
//! and signal-forward the agent process. This module provides exactly that — it
//! builds the child command from the user's `agent_argv`, points it at the chain
//! head via `ANTHROPIC_BASE_URL`, and mirrors the orchestrator's `extra_env`
//! (chain signature, `ENABLE_TOOL_SEARCH`, the central wire-proxy auth token)
//! into the process environment.
//!
//! M7 (the claude adapter milestone) ENRICHES this with the inline `--settings`
//! JSON and the `ENABLE_TOOL_SEARCH`-origin cross-check; it does not re-type the
//! [`Agent`] trait or replace the process-env wiring this milestone establishes.

use url::Url;

use crate::agent::Agent;

/// The v1 agent implementation. Zero-sized: all per-run state arrives through
/// `build_command`'s arguments (the resolved chain head and `extra_env`).
pub struct ClaudeAgent;

impl Agent for ClaudeAgent {
    fn name(&self) -> &str {
        "claude"
    }

    /// Build the not-yet-spawned child command for the agent.
    ///
    /// `argv` is the user's pass-through agent invocation (`run -- <prog> args…`):
    /// `argv[0]` is the program, `argv[1..]` its arguments. `base_url` is the
    /// chain head (or the reused live chain / tail upstream) and is exported as
    /// `ANTHROPIC_BASE_URL`; every `extra_env` pair is mirrored into the process
    /// environment (`ANTHROPIC_BASE_URL` is deliberately NOT in `extra_env` — it
    /// comes from `base_url`, per `orchestrator::compute_agent_env`).
    fn build_command(
        &self,
        argv: &[String],
        base_url: &Url,
        extra_env: &[(String, String)],
    ) -> tokio::process::Command {
        // Generic model (M6): the program is argv[0]; argv[1..] are its args.
        // Belt 2 (M7.2): a single `--settings <json>` pair is inserted between the
        // program and argv[1..], so it lands at CLI-arg precedence ahead of the
        // user's own flags. The JSON's `{"env":{...}}` contents — mirroring belt 1
        // (ANTHROPIC_BASE_URL + extra_env) — are filled in M7.4; M7.2 only fixes
        // the program/argv ordering and the presence/position of `--settings`.
        let (program, rest): (&str, &[String]) = match argv.split_first() {
            Some((program, rest)) => (program.as_str(), rest),
            // Empty argv: invoke the default agent binary, still emitting belt 2.
            None => (self.name(), &[]),
        };
        let mut cmd = tokio::process::Command::new(program);

        // Belt 2: inline --settings env block, inserted BEFORE the user's args.
        cmd.arg("--settings");
        cmd.arg("{}"); // placeholder JSON; real env block added in M7.4.

        // User args (argv[1..]) last.
        cmd.args(rest);

        // Belt 1: process environment. ANTHROPIC_BASE_URL first, then every
        // orchestrator env entry (POVERTY_PROXY_CHAIN, ENABLE_TOOL_SEARCH, and the
        // central-tail ANTHROPIC_AUTH_TOKEN). The same values land in belt 2's
        // JSON, so the two belts cannot disagree.
        cmd.env("ANTHROPIC_BASE_URL", base_url.as_str());
        for (k, v) in extra_env {
            cmd.env(k, v);
        }
        cmd
    }
}

#[cfg(test)]
#[path = "claude_tests.rs"]
mod claude_tests;
