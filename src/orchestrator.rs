//! Orchestrator: resolve the tail upstream, build the proxy chain back-to-front
//! with a race-free READY handshake, wire + signal-forward the agent, run it, and
//! tear the chain down (children survive parent death — see `teardown`).

use crate::config::ResolvedProxy;

/// Render a resolved chain as the `POVERTY_PROXY_CHAIN` value: lowercase proxy
/// names in chain order (head->tail), comma-separated. Empty chain -> "".
pub fn serialize_chain(chain: &[ResolvedProxy]) -> String {
    chain
        .iter()
        .map(|p| p.name.as_str())
        .collect::<Vec<_>>()
        .join(",")
}

/// Parse a `POVERTY_PROXY_CHAIN` CSV into ordered, trimmed, non-empty names.
/// (Validation/coercion is M2's `resolve_chain`; this is the raw tokenizer used
/// by the nested-reuse comparison and when threading the env value through.)
pub fn parse_chain(csv: &str) -> Vec<String> {
    csv.split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

use crate::central::{self, CentralInfo};
use crate::proxy::Upstream;

/// The default tail when the user has no pre-existing gateway and central is not
/// the tail.
const DEFAULT_ANTHROPIC_BASE_URL: &str = "https://api.anthropic.com";

/// Inputs to tail-upstream resolution, passed explicitly so the resolver is
/// pure and unit-testable (the caller reads `std::env` and central state).
#[derive(Clone, Debug, Default)]
pub struct TailInputs {
    /// `Some` when central is the chain tail (its started port+secret).
    pub central: Option<CentralInfo>,
    /// The user's pre-existing `ANTHROPIC_BASE_URL`, if any. Empty/whitespace is
    /// treated as unset.
    pub preexisting_base_url: Option<String>,
}

/// Resolve where the LAST hop forwards (design §6 / R17):
/// 1. central tail -> its wire URL,
/// 2. else a non-empty pre-existing `ANTHROPIC_BASE_URL` (stack in front of it),
/// 3. else `https://api.anthropic.com`.
pub fn resolve_tail_upstream(inputs: &TailInputs) -> anyhow::Result<Upstream> {
    if let Some(info) = inputs.central.as_ref() {
        return Ok(central::central_wire_upstream(info));
    }

    let preexisting = inputs
        .preexisting_base_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let raw = preexisting.unwrap_or(DEFAULT_ANTHROPIC_BASE_URL);
    let url = url::Url::parse(raw)
        .map_err(|e| anyhow::anyhow!("ANTHROPIC_BASE_URL is not a valid URL ({raw:?}): {e}"))?;
    Ok(Upstream { url })
}

/// Assemble the agent's `extra_env` (what M7's `Agent::build_command` mirrors
/// into both the process env and the inline `--settings` JSON).
///
/// `ANTHROPIC_BASE_URL` is intentionally NOT included: the agent sets it from
/// its `base_url` argument (the chain head). `central_is_tail` controls the
/// dummy `ANTHROPIC_AUTH_TOKEN=wire-proxy` (central injects the real JWT).
pub fn compute_agent_env(chain: &[ResolvedProxy], central_is_tail: bool) -> Vec<(String, String)> {
    let mut env = vec![
        ("POVERTY_PROXY_CHAIN".to_string(), serialize_chain(chain)),
        ("ENABLE_TOOL_SEARCH".to_string(), "true".to_string()),
    ];
    if central_is_tail {
        env.push(("ANTHROPIC_AUTH_TOKEN".to_string(), "wire-proxy".to_string()));
    }
    env
}

use std::path::PathBuf;

use crate::config::ProxySettings;
use crate::proxy::pino::TailTtl;

/// Everything needed to render one hop's `poverty-mode proxy <name>` argv.
pub struct ProxyHopSpec<'a> {
    pub proxy: &'a ResolvedProxy,
    /// Listen addr — `127.0.0.1:0` for an OS-assigned ephemeral port.
    pub listen: String,
    /// The next hop's URL (or the tail upstream for the last hop).
    pub upstream: String,
    /// Per-run ULID identity shared by all hops of this run (R10).
    pub run_id: String,
    pub log_file: PathBuf,
}

fn tail_ttl_str(t: TailTtl) -> &'static str {
    match t {
        TailTtl::FiveMin => "5m",
        TailTtl::OneHour => "1h",
    }
}

/// Build the exact argv for `current_exe proxy <name> ...` from a hop spec.
///
/// The flags are emitted so the argv re-parses through M1's actual `proxy` clap
/// parser (`orchestrator_tests::proxy_child_args_round_trips_through_clap`):
/// - the per-proxy body-tee sink is `--body-log-file` (the global `--log-file`
///   is a separate tracing arg);
/// - `--auto-cache` / `--strip-ansi` / `--compression` are PRESENCE flags with
///   `--no-*` companions, so a boolean is encoded by emitting the right flag (or
///   nothing when the value already matches the CLI default), never a value.
///
/// pino default: `auto_cache=false`, `strip_ansi=true`. headroom default:
/// `compression=false`. Optional flags (`--drop-tools`, `--model-override`) are
/// omitted when empty/unset.
pub fn proxy_child_args(spec: &ProxyHopSpec) -> Vec<String> {
    let mut args = vec![
        "proxy".to_string(),
        spec.proxy.name.as_str().to_string(),
        "--listen".to_string(),
        spec.listen.clone(),
        "--upstream".to_string(),
        spec.upstream.clone(),
        "--run-id".to_string(),
        spec.run_id.clone(),
        "--body-log-file".to_string(),
        spec.log_file.to_string_lossy().into_owned(),
    ];

    match &spec.proxy.settings {
        ProxySettings::Pino(p) => {
            // auto_cache default is false: emit the presence flag only when on.
            if p.auto_cache {
                args.push("--auto-cache".to_string());
            }
            args.push("--tail-ttl".to_string());
            args.push(tail_ttl_str(p.tail_ttl).to_string());
            if !p.drop_tools.is_empty() {
                args.push("--drop-tools".to_string());
                args.push(p.drop_tools.join(","));
            }
            // strip_ansi default is true: emit --no-strip-ansi only when off.
            if !p.strip_ansi {
                args.push("--no-strip-ansi".to_string());
            }
            if let Some(model) = p.model_override.as_ref() {
                args.push("--model-override".to_string());
                args.push(model.clone());
            }
        }
        ProxySettings::Headroom(h) => {
            // compression default is false: emit the explicit flag either way so
            // the child's resolved value is unambiguous regardless of defaults.
            if h.compression {
                args.push("--compression".to_string());
            } else {
                args.push("--no-compression".to_string());
            }
        }
        ProxySettings::Central(_) => {
            // Central is never spawned via `poverty-mode proxy`; it is the
            // external daemon. The chain builder never passes a Central hop here.
            debug_assert!(
                false,
                "proxy_child_args must never be called for a Central hop"
            );
        }
    }

    args
}

#[cfg(test)]
#[path = "orchestrator_tests.rs"]
mod orchestrator_tests;
