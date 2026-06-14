//! Orchestrator: resolve the tail upstream, build the proxy chain back-to-front
//! with a race-free READY handshake, wire + signal-forward the agent, run it, and
//! tear the chain down (children survive parent death — see `teardown`).

pub mod manager;
pub mod teardown;

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

use crate::agent::Agent;

/// True iff the chain's last hop is the must-be-last (Central) proxy.
pub fn central_is_tail(chain: &[ResolvedProxy]) -> bool {
    chain.last().map(|p| p.name.must_be_last()).unwrap_or(false)
}

/// Split off a SINGLE trailing must-be-last (Central) entry, returning
/// (first_party_hops, central_is_tail). Asserts (debug) that no non-trailing
/// entry is must_be_last (the central-last contract; `resolve_chain` enforces it).
fn split_trailing_central(chain: &[ResolvedProxy]) -> (Vec<ResolvedProxy>, bool) {
    match chain.split_last() {
        Some((last, init)) if last.name.must_be_last() => {
            debug_assert!(
                init.iter().all(|p| !p.name.must_be_last()),
                "central-last contract violated: a non-trailing entry is must_be_last"
            );
            (init.to_vec(), true)
        }
        _ => {
            debug_assert!(
                chain.iter().all(|p| !p.name.must_be_last()),
                "central-last contract violated: a non-trailing entry is must_be_last"
            );
            (chain.to_vec(), false)
        }
    }
}

/// Build the chain (back-to-front, race-free, via the ProxyManager seam), run the
/// agent against the head, forward signals, wait, then tear the chain down.
/// Returns the agent's exit status.
///
/// No-first-party-hop cases (truly empty chain, OR central-only): exec the agent
/// unchanged, pointed straight at `tail_upstream` (design §6/§11 "easy to not
/// use"; for central-only, `tail_upstream` is the central wire URL).
pub async fn build_and_run(
    chain: Vec<ResolvedProxy>,
    tail_upstream: Upstream,
    agent: &dyn Agent,
    argv: &[String],
) -> anyhow::Result<std::process::ExitStatus> {
    let (hops, central_tail) = split_trailing_central(&chain);

    if hops.is_empty() {
        // No first-party proxies to spawn: the agent talks directly to the tail
        // upstream (for central-only, that is the wire URL). agent_env still
        // reflects central_tail for the auth override.
        let env = compute_agent_env(&chain, central_tail);
        let mut cmd = agent.build_command(argv, &tail_upstream.url, &env);
        let status = cmd
            .status()
            .await
            .map_err(|e| anyhow::anyhow!("spawning agent '{}': {e}", agent.name()))?;
        return Ok(status);
    }

    let exe = self_spawn_exe()?;
    let mut manager = manager::EphemeralManager::new(exe)?;

    build_via_manager(
        &mut manager,
        &chain,
        &hops,
        central_tail,
        &tail_upstream,
        agent,
        argv,
    )
    .await
}

/// Env var naming the `poverty-mode` binary to re-spawn for proxy hops. Honored
/// by `self_spawn_exe` ahead of `current_exe()`. This is a test seam: integration
/// tests drive `build_and_run` from the libtest harness binary, whose
/// `current_exe()` is NOT `poverty-mode` and cannot serve the `proxy` subcommand,
/// so they set this to the real `CARGO_BIN_EXE_poverty-mode`. In production it is
/// unset and `current_exe()` (the running `poverty-mode`) is used.
const SELF_SPAWN_EXE_ENV: &str = "POVERTY_PROXY_EXE";

/// Resolve the binary to self-spawn for proxy hops: `POVERTY_PROXY_EXE` if set and
/// non-empty (test seam), else `std::env::current_exe()` (the running binary).
fn self_spawn_exe() -> anyhow::Result<std::path::PathBuf> {
    if let Some(p) = std::env::var_os(SELF_SPAWN_EXE_ENV) {
        if !p.is_empty() {
            return Ok(std::path::PathBuf::from(p));
        }
    }
    std::env::current_exe()
        .map_err(|e| anyhow::anyhow!("resolving current_exe for self-spawn: {e}"))
}

/// Build the chain through a `&mut dyn ProxyManager` (R15 seam), run + signal-
/// forward the agent (signal forwarding added in M6.11), then shut the manager
/// down. Fail-closed deadline added in M6.12.
async fn build_via_manager(
    manager: &mut dyn manager::ProxyManager,
    chain: &[ResolvedProxy],
    hops: &[ResolvedProxy],
    central_tail: bool,
    tail_upstream: &Upstream,
    agent: &dyn Agent,
    argv: &[String],
) -> anyhow::Result<std::process::ExitStatus> {
    let run_id = crate::paths::new_run_id();
    let run_dir = crate::paths::run_dir(&run_id)?;
    std::fs::create_dir_all(&run_dir)
        .map_err(|e| anyhow::anyhow!("creating run dir {}: {e}", run_dir.display()))?;

    // Carry STRUCTURED hop fields; the manager renders the exact argv via the
    // single source of truth `proxy_child_args` once it knows each hop's real
    // back-to-front --upstream. No placeholder/strip/re-append dance.
    let hop_specs: Vec<manager::HopSpec> = hops
        .iter()
        .enumerate()
        .map(|(i, hop)| manager::HopSpec {
            proxy: hop.clone(),
            run_id: run_id.clone(),
            log_file: run_dir.join(format!("{}-{}.log", hop.name.as_str(), i)),
        })
        .collect();

    let running = manager.start_hops(&hop_specs, tail_upstream).await?;
    let head = running
        .first()
        .ok_or_else(|| anyhow::anyhow!("manager returned no running hops"))?;
    let head_base_url = head.base_url.clone();

    let agent_env = compute_agent_env(chain, central_tail);
    let mut agent_cmd = agent.build_command(argv, &head_base_url, &agent_env);
    let status_result = agent_cmd.status().await;

    // Teardown regardless of agent outcome (await reaping before returning).
    let _ = manager.shutdown().await;

    let status =
        status_result.map_err(|e| anyhow::anyhow!("spawning agent '{}': {e}", agent.name()))?;
    Ok(status)
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

use tokio::io::{AsyncBufRead, AsyncBufReadExt};

use crate::proxy::{ProxyName, ReadyLine};

/// Read the child's READY line from `reader` (a blocking pipe read = the real
/// synchronization primitive — no sleep/poll). Skips lines that are not JSON
/// objects (stray startup logs, emitted to `trace`). A JSON object that carries
/// a `"ready"` key but fails `ReadyLine` deserialization is surfaced as a parse
/// error (diagnosable), NOT silently skipped. Validates `ready == true`, the
/// proxy name, and the per-run `run_id` (R10). EOF before a valid READY, or any
/// mismatch, is an error (fail-closed).
pub async fn read_ready_line<R>(
    reader: &mut R,
    expected_name: ProxyName,
    expected_run_id: &str,
) -> anyhow::Result<ReadyLine>
where
    R: AsyncBufRead + Unpin,
{
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            anyhow::bail!(
                "proxy '{}' closed its stdout before emitting a READY line",
                expected_name.as_str()
            );
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Is this a JSON object at all? Non-objects are stray logs -> skip.
        let as_value: serde_json::Value = match serde_json::from_str(trimmed) {
            Ok(serde_json::Value::Object(map)) => serde_json::Value::Object(map),
            _ => {
                tracing::trace!(line = %trimmed, "skipping non-JSON-object stdout line before READY");
                continue;
            }
        };

        // A JSON object WITHOUT a "ready" key is some other structured log -> skip.
        let looks_like_ready = as_value.get("ready").is_some();
        if !looks_like_ready {
            tracing::trace!(line = %trimmed, "skipping JSON object without a 'ready' key");
            continue;
        }

        // It claims to be a READY line: it MUST deserialize, else diagnose.
        let parsed: ReadyLine = serde_json::from_value(as_value).map_err(|e| {
            anyhow::anyhow!(
                "malformed READY line from proxy '{}' (object had a 'ready' key but did not match the READY shape): {e}; line was: {trimmed}",
                expected_name.as_str()
            )
        })?;

        if !parsed.ready {
            anyhow::bail!("proxy '{}' reported ready=false", expected_name.as_str());
        }
        if parsed.proxy != expected_name.as_str() {
            anyhow::bail!(
                "proxy identity mismatch: expected '{}', READY line says '{}'",
                expected_name.as_str(),
                parsed.proxy
            );
        }
        if parsed.run_id != expected_run_id {
            anyhow::bail!(
                "run id mismatch for proxy '{}': expected '{}', READY line says '{}'",
                expected_name.as_str(),
                expected_run_id,
                parsed.run_id
            );
        }
        return Ok(parsed);
    }
}

use crate::proxy::HealthBody;

/// Per-request bound for the blocking health probe (see `health_probe`). Bounds an
/// external event (an unresponsive hop) so a detached `spawn_blocking` probe
/// cannot outlive a cancelled readiness deadline.
const HEALTH_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Blocking `GET <base>/__pm/health`, parsed into a `HealthBody`, or `None` on
/// any failure (not listening, non-200, wrong body). SYNCHRONOUS — callers in an
/// async context MUST run it via `tokio::task::spawn_blocking` (R5).
///
/// The client carries a bounded per-request timeout. This is the sanctioned
/// human-surfaced failure bound on an EXTERNAL event (a hop's HTTP server that
/// never answers), NOT a sync-by-sleep. It also guarantees that if the caller's
/// `start_hops` future is cancelled on the readiness deadline, the detached
/// `spawn_blocking` probe cannot run forever on the blocking pool — it
/// self-terminates at the timeout, so no leaked blocking task outlives the run.
pub fn health_probe(base: &url::Url) -> Option<HealthBody> {
    let url = base.join("/__pm/health").ok()?;
    let client = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(HEALTH_PROBE_TIMEOUT)
        .build()
        .ok()?;
    let resp = client.get(url).send().ok()?;
    if !resp.status().is_success() {
        return None;
    }
    // Parse the body via `text()` + `serde_json` rather than `Response::json` so
    // we do not require reqwest's `json` cargo feature (R2 owns the dep set:
    // features = ["rustls-tls-native-roots", "stream", "blocking"], no "json").
    let body = resp.text().ok()?;
    serde_json::from_str::<HealthBody>(&body).ok()
}

/// Convenience: the live per-run `run_id` reported by `/__pm/health` (R10), or
/// `None`. Used by `EphemeralManager` to confirm a hop's identity at startup.
/// SYNCHRONOUS — run via `spawn_blocking` from async (R5).
pub fn health_chain_id(base: &url::Url) -> Option<String> {
    health_probe(base).map(|h| h.run_id)
}

/// Pure nested-reuse decision (design §7 / R10). Reuse the live chain iff:
/// the desired chain signature is non-empty, the env `POVERTY_PROXY_CHAIN`
/// equals it, the env `ANTHROPIC_BASE_URL` parses, and that base is LIVE
/// (probe returns true). Returns `Some(base)` to reuse, else `None`.
///
/// The chain-signature match is against the env value (the live chain's
/// signature), NOT the `/__pm/health` body — the body carries the per-run
/// `run_id` (R10), so `is_live` is a pure liveness check.
pub fn nested_reuse_decision(
    desired_chain_sig: &str,
    env_chain: Option<String>,
    env_base: Option<String>,
    is_live: impl Fn(&url::Url) -> bool,
) -> Option<url::Url> {
    let desired = desired_chain_sig.trim();
    if desired.is_empty() {
        return None;
    }
    let env_chain = env_chain?;
    if env_chain.trim() != desired {
        return None;
    }
    let base_raw = env_base?;
    let base = url::Url::parse(base_raw.trim()).ok()?;
    if is_live(&base) {
        Some(base)
    } else {
        None
    }
}

/// Design §7 nested-invocation guard. If our env has `POVERTY_PROXY_CHAIN` equal
/// to `desired_chain`'s signature and `ANTHROPIC_BASE_URL` set to a LIVE
/// `/__pm/health`, return `Some(base)` so the caller execs the agent against the
/// live chain (no second chain). Else `None`. SYNCHRONOUS (calls `health_probe`)
/// — invoke via `spawn_blocking` from async (R5; see `run_command`).
pub fn nested_reuse_check(desired_chain: &[ResolvedProxy]) -> Option<url::Url> {
    let desired_sig = serialize_chain(desired_chain);
    let env_chain = std::env::var("POVERTY_PROXY_CHAIN").ok();
    let env_base = std::env::var("ANTHROPIC_BASE_URL").ok();
    nested_reuse_decision(&desired_sig, env_chain, env_base, |u| {
        health_probe(u).is_some()
    })
}

#[cfg(test)]
#[path = "orchestrator_tests.rs"]
mod orchestrator_tests;
