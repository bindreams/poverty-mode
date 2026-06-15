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
        return central::central_wire_upstream(info);
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
///
/// The orchestrator ORIGINATES `ENABLE_TOOL_SEARCH` (M7 contract) and emits the
/// caller's `enable_tool_search` value verbatim (`config.defaults.enable_tool_search`,
/// default `true`). Setting it `false` disables Claude Code MCP tool search through
/// the proxy: a non-first-party base URL otherwise turns tool search off, so the
/// key is always emitted, never omitted.
pub fn compute_agent_env(
    chain: &[ResolvedProxy],
    central_is_tail: bool,
    enable_tool_search: bool,
) -> Vec<(String, String)> {
    let mut env = vec![
        ("POVERTY_PROXY_CHAIN".to_string(), serialize_chain(chain)),
        (
            "ENABLE_TOOL_SEARCH".to_string(),
            enable_tool_search.to_string(),
        ),
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

/// The bounded readiness deadline: the single human-surfaced failure timeout the
/// house rules permit (a bound on an external event — a child that never becomes
/// ready — NOT a sleep to synchronize code we control). If the chain is not ready
/// within this, the run is aborted and torn down. The success path never waits on
/// this timer: `start_hops` resolves the instant the last hop is ready.
pub const READINESS_DEADLINE: std::time::Duration = std::time::Duration::from_secs(30);

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
    enable_tool_search: bool,
) -> anyhow::Result<std::process::ExitStatus> {
    build_and_run_with_fault(chain, tail_upstream, agent, argv, enable_tool_search, false).await
}

/// Like [`build_and_run`], but constructs an [`manager::EphemeralManager`] with a
/// fault knob (test-only: forces the tail-most hop to fail readiness). Inert when
/// `fault` is false; [`build_and_run`] calls this with `false`.
#[doc(hidden)]
pub async fn build_and_run_with_fault(
    chain: Vec<ResolvedProxy>,
    tail_upstream: Upstream,
    agent: &dyn Agent,
    argv: &[String],
    enable_tool_search: bool,
    fault: bool,
) -> anyhow::Result<std::process::ExitStatus> {
    let (hops, central_tail) = split_trailing_central(&chain);

    if hops.is_empty() {
        // No first-party proxies to spawn: the agent talks directly to the tail
        // upstream (for central-only, that is the wire URL). agent_env still
        // reflects central_tail for the auth override.
        let env = compute_agent_env(&chain, central_tail, enable_tool_search);
        let cmd = agent.build_command(argv, &tail_upstream.url, &env);
        let status = run_agent_forwarding_signals(cmd, agent.name()).await?;
        return Ok(status);
    }

    let exe = self_spawn_exe()?;
    let mut manager = manager::EphemeralManager::new_with_fault(exe, fault)?;

    build_via_manager(
        &mut manager,
        &chain,
        &hops,
        central_tail,
        &tail_upstream,
        agent,
        argv,
        enable_tool_search,
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
/// Body-log filename for a hop (design §5.11: `<proxy>-<port>.log`). The port is
/// OS-assigned at bind time, so we emit the literal `{port}` token here; the engine
/// substitutes the real bound port at file-open (`proxy::resolve_log_file`), and
/// `status::enumerate_runs` parses it back. Kept a named fn so the producer side is
/// test-pinned (a regression to a hop-index name like `pino-0.log` fails loudly).
fn hop_log_file(run_dir: &std::path::Path, name: ProxyName) -> std::path::PathBuf {
    run_dir.join(format!("{}-{{port}}.log", name.as_str()))
}

#[allow(clippy::too_many_arguments)]
async fn build_via_manager(
    manager: &mut dyn manager::ProxyManager,
    chain: &[ResolvedProxy],
    hops: &[ResolvedProxy],
    central_tail: bool,
    tail_upstream: &Upstream,
    agent: &dyn Agent,
    argv: &[String],
    enable_tool_search: bool,
) -> anyhow::Result<std::process::ExitStatus> {
    let run_id = crate::paths::new_run_id();
    // `ensure_run_dir` hardens the dir to 0700 on POSIX (the run dir holds proxy
    // body-log files with full request/response bodies), unlike a raw create.
    let run_dir = crate::paths::ensure_run_dir(&run_id)?;

    // Carry STRUCTURED hop fields; the manager renders the exact argv via the
    // single source of truth `proxy_child_args` once it knows each hop's real
    // back-to-front --upstream. No placeholder/strip/re-append dance.
    //
    // The body-log file is named per design spec §5.11 as
    // `<state>/runs/<run-id>/<proxy>-<port>.log`, where `<port>` is the hop's REAL
    // listening port — the value `status::enumerate_runs` parses back out. The port
    // is OS-assigned and unknown here (hops bind `127.0.0.1:0` later), so we embed
    // the `{port}` token; the engine substitutes its bound port at file-open
    // (`proxy::resolve_log_file`). This keeps producer and consumer on the same
    // contract instead of writing a hop index the consumer would misread as a port.
    let hop_specs: Vec<manager::HopSpec> = hops
        .iter()
        .map(|hop| manager::HopSpec {
            proxy: hop.clone(),
            run_id: run_id.clone(),
            log_file: hop_log_file(&run_dir, hop.name),
        })
        .collect();

    let running = match tokio::time::timeout(
        READINESS_DEADLINE,
        manager.start_hops(&hop_specs, tail_upstream),
    )
    .await
    {
        Ok(Ok(running)) => running,
        Ok(Err(e)) => {
            // start_hops already tore down on internal error; surface it.
            return Err(e);
        }
        Err(_elapsed) => {
            // Timeout: dropping the `start_hops` future cancels it mid-flight, so
            // we run teardown explicitly to reap whatever children it spawned. Any
            // in-flight `spawn_blocking` health probe it left detached cannot run
            // forever — `health_probe` carries `HEALTH_PROBE_TIMEOUT`, so the
            // stranded blocking task self-terminates rather than leaking.
            let _ = manager.shutdown().await;
            anyhow::bail!(
                "chain not ready within {}s; torn down all started proxies",
                READINESS_DEADLINE.as_secs()
            );
        }
    };
    let head = running
        .first()
        .ok_or_else(|| anyhow::anyhow!("manager returned no running hops"))?;
    let head_base_url = head.base_url.clone();

    let agent_env = compute_agent_env(chain, central_tail, enable_tool_search);
    let agent_cmd = agent.build_command(argv, &head_base_url, &agent_env);
    let status_result = run_agent_forwarding_signals(agent_cmd, agent.name()).await;

    // Drain on agent exit (R17): tear down the proxy hops, await reaping.
    let _ = manager.shutdown().await;

    status_result
}

/// Spawn the agent child and wait for it, forwarding SIGINT/SIGTERM/Ctrl-C to it
/// (R17). Returns the agent's exit status. Does NOT tear down proxies — the
/// caller does that after this returns (drain on agent exit).
async fn run_agent_forwarding_signals(
    mut cmd: tokio::process::Command,
    agent_name: &str,
) -> anyhow::Result<std::process::ExitStatus> {
    let mut child = cmd
        .spawn()
        .map_err(|e| anyhow::anyhow!("spawning agent '{agent_name}': {e}"))?;

    #[cfg(unix)]
    {
        let child_pid = child.id();
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigint = signal(SignalKind::interrupt())
            .map_err(|e| anyhow::anyhow!("installing SIGINT handler: {e}"))?;
        let mut sigterm = signal(SignalKind::terminate())
            .map_err(|e| anyhow::anyhow!("installing SIGTERM handler: {e}"))?;
        loop {
            tokio::select! {
                status = child.wait() => {
                    return status.map_err(|e| anyhow::anyhow!("waiting on agent '{agent_name}': {e}"));
                }
                _ = sigint.recv() => {
                    forward_signal_unix(child_pid, libc::SIGINT);
                }
                _ = sigterm.recv() => {
                    forward_signal_unix(child_pid, libc::SIGTERM);
                }
            }
        }
    }

    #[cfg(windows)]
    {
        loop {
            tokio::select! {
                status = child.wait() => {
                    return status.map_err(|e| anyhow::anyhow!("waiting on agent '{agent_name}': {e}"));
                }
                _ = tokio::signal::ctrl_c() => {
                    // Windows delivers a console CTRL_C_EVENT to every process
                    // attached to the same console. The agent is spawned via
                    // `tokio::process::Command` with default flags, so it shares our
                    // console and the OS already delivered Ctrl-C to it directly —
                    // our handler is the backstop. We additionally run the
                    // TerminateProcess backstop so a child that ignores Ctrl-C is
                    // still reaped, then keep waiting for its real exit. (A non-Ctrl-C
                    // termination of the orchestrator itself — e.g. `taskkill /F`
                    // without `/T` — is not a signal we can intercept; the proxy HOPS
                    // are still reaped by the kill-on-job-close Job Object regardless,
                    // per R16, so no proxy is orphaned. Only the agent child is then
                    // left to the OS, the documented limit of cooperative shutdown on
                    // Windows.)
                    windows_ctrlc_backstop(&mut child);
                }
            }
        }
    }
}

/// Windows Ctrl-C backstop: TerminateProcess the agent child if it ignored the
/// console Ctrl-C the OS already delivered. Factored out so it is testable from an
/// integration test (the select arm above is not directly drivable in a hermetic
/// test, but its OS effect — reaping the agent — is). `pub` so
/// `tests/chain_signals_windows.rs` can call it as an external crate.
#[doc(hidden)]
#[cfg(windows)]
pub fn windows_ctrlc_backstop(child: &mut tokio::process::Child) {
    let _ = child.start_kill();
}

#[cfg(unix)]
fn forward_signal_unix(child_pid: Option<u32>, sig: libc::c_int) {
    if let Some(pid) = child_pid {
        // SAFETY: kill is always safe; ESRCH (already exited) is ignored.
        unsafe {
            libc::kill(pid as libc::pid_t, sig);
        }
    }
}

use std::path::PathBuf;

use crate::config::ProxySettings;
use crate::proxy::pino::CacheTtl;

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

fn ttl_str(t: CacheTtl) -> &'static str {
    match t {
        CacheTtl::FiveMin => "5m",
        CacheTtl::OneHour => "1h",
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
/// `compression=true`. Optional flags (`--drop-tools`, `--model-override`) are
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
            args.push("--main-ttl".to_string());
            args.push(ttl_str(p.main_ttl).to_string());
            args.push("--sub-ttl".to_string());
            args.push(ttl_str(p.sub_ttl).to_string());
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
            // compression default is true: emit the explicit flag either way so
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

use crate::agent::claude::ClaudeAgent;

/// High-level `run` orchestration (R5-safe): nested-reuse short-circuit, central
/// daemon start (when central is tail), tail resolution, then `build_and_run`
/// with the v1 ClaudeAgent. `chain` is the already-resolved chain (caller applies
/// cli>env>file precedence + optional TUI). `enable_tool_search` is the resolved
/// `config.defaults.enable_tool_search` (default `true`), threaded into the agent
/// env. Returns the agent's exit status.
///
/// R5: this is `async` and awaited from the runtime, so the two BLOCKING probes
/// (`nested_reuse_check`, which uses `reqwest::blocking`, and `ensure_central_started`,
/// which calls the synchronous central install/login/start/health machinery) are
/// dispatched off the executor via `tokio::task::spawn_blocking`. Calling either
/// synchronously here would panic ("Cannot start a runtime from within a runtime").
pub async fn run_command(
    chain: Vec<ResolvedProxy>,
    argv: &[String],
    enable_tool_search: bool,
) -> anyhow::Result<std::process::ExitStatus> {
    let agent = ClaudeAgent;

    // Design §7 nested-reuse: run the BLOCKING health probe off the executor (R5).
    let chain_for_probe = chain.clone();
    let reuse = tokio::task::spawn_blocking(move || nested_reuse_check(&chain_for_probe))
        .await
        .map_err(|e| anyhow::anyhow!("nested-reuse probe task join error: {e}"))?;
    if let Some(base) = reuse {
        let env = compute_agent_env(&chain, central_is_tail(&chain), enable_tool_search);
        let cmd = agent.build_command(argv, &base, &env);
        return run_agent_forwarding_signals(cmd, agent.name()).await;
    }

    // Resolve the tail upstream. Start central first if it is the tail — its
    // ensure/install/login/start/health calls are R5-blocking, so run them off
    // the executor via spawn_blocking.
    let inputs = if central_is_tail(&chain) {
        let chain_for_central = chain.clone();
        let info = tokio::task::spawn_blocking(move || ensure_central_started(&chain_for_central))
            .await
            .map_err(|e| anyhow::anyhow!("central start task join error: {e}"))??;
        TailInputs {
            central: Some(info),
            preexisting_base_url: None,
        }
    } else {
        TailInputs {
            central: None,
            preexisting_base_url: std::env::var("ANTHROPIC_BASE_URL").ok(),
        }
    };
    let tail = resolve_tail_upstream(&inputs)?;

    build_and_run(chain, tail, &agent, argv, enable_tool_search).await
}

/// The Central install/login/start/health operations the orchestrator drives,
/// behind a seam (the same injection style as [`manager::ProxyManager`], R15) so
/// the central-tail orchestration is unit-testable without a real `jbcentral`
/// daemon. The v1 impl is [`RealCentral`], which forwards to the live
/// `crate::central` functions (M8).
///
/// Every method is SYNCHRONOUS/blocking (R5): the real impl shells out, hits the
/// network, and does blocking health GETs — callers run the whole pipeline via
/// `tokio::task::spawn_blocking` (see `run_command`).
trait CentralOps {
    /// Resolve the jbcentral version to use (R4): the entry's pinned version if
    /// set, else the live `latest/version.txt` with fallback to the default.
    fn resolve_version(&self, cfg_pinned: Option<&str>) -> String;
    /// Ensure `version` is installed; return the binary path.
    fn ensure_installed(&self, version: &str) -> anyhow::Result<PathBuf>;
    /// Ensure the user is logged in (interactive login if needed).
    fn ensure_logged_in(&self, bin: &std::path::Path) -> anyhow::Result<()>;
    /// Configure + start the daemon for `version`, requesting `port`; return the
    /// live `CentralInfo`.
    fn start(
        &self,
        bin: &std::path::Path,
        port: Option<u16>,
        version: &str,
    ) -> anyhow::Result<CentralInfo>;
    /// True iff the daemon at `port` answers `/health`.
    fn health(&self, port: u16) -> bool;
}

/// Production [`CentralOps`]: forwards to the live `crate::central` pipeline (M8).
struct RealCentral;

impl CentralOps for RealCentral {
    fn resolve_version(&self, cfg_pinned: Option<&str>) -> String {
        central::resolve_version(cfg_pinned)
    }
    fn ensure_installed(&self, version: &str) -> anyhow::Result<PathBuf> {
        central::ensure_installed(version)
    }
    fn ensure_logged_in(&self, bin: &std::path::Path) -> anyhow::Result<()> {
        central::ensure_logged_in(bin)
    }
    fn start(
        &self,
        bin: &std::path::Path,
        port: Option<u16>,
        version: &str,
    ) -> anyhow::Result<CentralInfo> {
        central::start(bin, port, version)
    }
    fn health(&self, port: u16) -> bool {
        central::health(port)
    }
}

/// Ensure the Central singleton is installed, logged in, and started; return its
/// CentralInfo (port + secret). Central is always the tail and is never torn down
/// on session exit (design §5.7/§9). SYNCHRONOUS (R5-blocking calls, incl.
/// interactive login) — callers run it via `spawn_blocking`.
///
/// Thin production wrapper over [`ensure_central_started_with`] with the real
/// `crate::central` pipeline ([`RealCentral`]).
fn ensure_central_started(chain: &[ResolvedProxy]) -> anyhow::Result<CentralInfo> {
    ensure_central_started_with(chain, &RealCentral)
}

/// Drive the central-tail pipeline through a [`CentralOps`] seam (R4/R5): resolve
/// the version once (from the trailing Central entry's pinned version), install,
/// log in, start at the entry's requested port, then health-check the LIVE
/// daemon's port (fail-closed if it never reports healthy).
fn ensure_central_started_with(
    chain: &[ResolvedProxy],
    ops: &dyn CentralOps,
) -> anyhow::Result<CentralInfo> {
    // Caller invariant: only reached when central is the tail (see `run_command`).
    let central_settings = match chain.last().map(|p| &p.settings) {
        Some(ProxySettings::Central(c)) => c,
        _ => {
            debug_assert!(
                false,
                "ensure_central_started called without a central tail"
            );
            anyhow::bail!("internal error: ensure_central_started called without a central tail");
        }
    };
    let port = central_settings.port;
    let pinned = central_settings.pinned_version.clone();

    // R4: resolve the version ONCE (live latest-or-fallback unless pinned), then
    // thread the SAME value into install and start so the installed asset, the
    // `config set pinned-version`, and the running daemon all agree.
    let version = ops.resolve_version(pinned.as_deref());
    let bin = ops.ensure_installed(&version)?;
    ops.ensure_logged_in(&bin)?;
    let info = ops.start(&bin, port, &version)?;
    if !ops.health(info.port) {
        anyhow::bail!("JB Central started but /health did not report healthy");
    }
    Ok(info)
}

#[cfg(test)]
#[path = "orchestrator_tests.rs"]
mod orchestrator_tests;
