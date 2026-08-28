//! JB Central: the shared singleton that always runs last in the chain, always an externally
//! installed `central` found on `PATH`. This module covers resolve / start / health / stop
//! (login is assumed, not driven); it currently
//! provides the items the orchestrator (M6) consumes — the started `CentralInfo`
//! (port + wire secret) and `central_wire_upstream`, which renders the JetBrains
//! wire URL the pre-central hop (or a central-only agent) targets — plus the M8.5
//! constants (R4) and central `config.json` parsing.
//!
//! **R5 contract:** every function here that does filesystem I/O (`read_wire_config`)
//! — and, as later M8 tasks fill them, every function that shells out or hits the
//! network — is synchronous/blocking. Callers in an async context (the orchestrator,
//! M6) MUST invoke them via `tokio::task::spawn_blocking`.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context};
use percent_encoding::{utf8_percent_encode, AsciiSet, CONTROLS};

use crate::proxy::Upstream;

/// Characters that must be percent-encoded so the wire secret stays a single,
/// faithful path segment (R20). Beyond the C0 controls, this encodes the path
/// terminators (`?`, `#`), the segment separator (`/`), space, and every other
/// generic-URI delimiter — so an arbitrary secret from central's `config.json`
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
/// bound and the wire secret read from central's `config.json` (design §6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CentralInfo {
    /// The loopback port central bound.
    pub port: u16,
    /// The wire secret central injects into its path prefix.
    pub secret: String,
}

/// The wire ENVELOPE URL that fronts JB Central (C1):
/// `http://127.0.0.1:<port>/wire/<percent-encoded-secret>` (design §6). The
/// agent-specific client/api segment (`claude-code/anthropic`, `codex/openai`) is
/// appended by the agent's base URL, NOT here, so a single chain serves every
/// agent. This is the upstream the hop before central uses (or the agent base
/// prefix for a central-only chain). The externally-sourced secret is
/// percent-encoded as one path segment. Never logged.
pub fn central_wire_envelope_url(info: &CentralInfo) -> String {
    let secret = utf8_percent_encode(&info.secret, WIRE_SECRET_SET);
    format!("http://127.0.0.1:{}/wire/{secret}", info.port)
}

/// The wire envelope URL the chain forwards to when central is the tail, as a parsed [`Upstream`]
/// for direct use as a proxy upstream. The pre-central hop carries this as its `--upstream`; in a
/// central-only chain the agent's `ANTHROPIC_BASE_URL` points here directly. Returns an error
/// (never panics) if, against expectation, the encoded URL fails to parse.
pub fn central_wire_upstream(info: &CentralInfo) -> anyhow::Result<Upstream> {
    let s = central_wire_envelope_url(info);
    let url = url::Url::parse(&s).with_context(|| "constructing the JB Central wire upstream URL")?;
    Ok(Upstream { url })
}

/// Parse the contents of central's `config.json` into a [`CentralInfo`].
///
/// Fails closed (error, never a default) when the file is unparseable or missing fields, so the
/// caller never silently bypasses wire. The error message never echoes the raw JSON (it may carry the
/// secret): on a parse failure we emit a fixed string and do NOT interpolate the serde error, which
/// could contain a fragment of the input. Some central builds write `proxy_port` as a string, so a
/// numeric-string port is coerced.
pub fn parse_wire_config(contents: &str) -> anyhow::Result<CentralInfo> {
    let value: serde_json::Value =
        serde_json::from_str(contents).map_err(|_| anyhow!("central's config.json is not valid JSON"))?;

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
        None => bail!("central's config.json is missing \"proxy_port\""),
    };

    let secret = match value.get("proxy_secret") {
        Some(serde_json::Value::String(s)) if !s.is_empty() => s.clone(),
        Some(_) => bail!("proxy_secret has an unexpected type or is empty"),
        None => bail!("central's config.json is missing \"proxy_secret\""),
    };

    Ok(CentralInfo { port, secret })
}

/// The executable name poverty-mode spawns when the config does not name one.
///
/// `jbcentral` survives only as a pre-1.0 compat symlink, so a fresh install may not have it.
pub const DEFAULT_CENTRAL_EXECUTABLE: &str = "central";

/// The executable to spawn for central: trimmed-non-empty `configured`, else
/// [`DEFAULT_CENTRAL_EXECUTABLE`]. Blank/unset is the default, not an error.
///
/// A bare name is returned AS a bare name on purpose: `Command` resolves it through `execvp`, whose
/// lookup is the authority. Pre-resolving it here would diverge from what actually executes.
pub fn central_executable(configured: Option<&str>) -> PathBuf {
    match configured.map(str::trim).filter(|s| !s.is_empty()) {
        Some(exe) => PathBuf::from(exe),
        None => PathBuf::from(DEFAULT_CENTRAL_EXECUTABLE),
    }
}

/// The shared "central is not installed" error, naming what was looked for.
pub fn missing_central_error(exe: &Path) -> anyhow::Error {
    if is_explicit_path(exe) {
        return anyhow!(
            "configured central executable `{}` does not exist — install JetBrains Central or fix `central.executable`",
            exe.display()
        );
    }
    anyhow!(
        "central executable `{}` not found on PATH — install JetBrains Central and make sure it is on your PATH",
        exe.display()
    )
}

/// True when `exe` is a path rather than a bare file name, i.e. `Command` will NOT search `PATH`
/// for it. Used only to word errors correctly.
fn is_explicit_path(exe: &Path) -> bool {
    if exe.is_absolute() {
        return true;
    }
    // Checked on the raw string, not via `components()`: `Path::new("central/").components()`
    // yields one component, so a count would call a trailing-slash name bare when the OS will
    // not search PATH for it.
    let raw = exe.as_os_str().to_string_lossy();
    raw.contains('/') || (cfg!(windows) && raw.contains('\\'))
}

/// Whether a central binary can actually be spawned, and how to label it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Presence {
    /// The process was created. `display` is `--version`'s first non-empty stdout line, or the path
    /// when it ran but printed nothing usable.
    Present { display: String },
    /// The process could NOT be created. `reason` says why, in the user's terms.
    ///
    /// Not only "missing": a directory or a file without the execute bit fails `PermissionDenied`,
    /// not `NotFound`. Calling those present would make `status` disagree with every run.
    Unavailable { reason: String },
}

/// Ask the OS whether central can be run, by spawning `<bin> --version`.
///
/// The ONLY presence check in this crate. There is deliberately no second mechanism: an `is_file`
/// walk cannot reproduce `execvp`/`CreateProcessW` (it matches a non-executable file that `execvp`
/// skips, and its `.exe` rules differ), so any such check drifts from what a run does. Everything
/// that reports on central — `status`, `doctor` — goes through here.
///
/// **R5 contract:** spawns a child process — call via `spawn_blocking` from async code.
pub fn probe_presence(bin: &Path) -> Presence {
    match std::process::Command::new(bin).arg("--version").output() {
        Err(e) => Presence::Unavailable {
            reason: unspawnable_reason(bin, &e),
        },
        Ok(output) => Presence::Present {
            display: version_line(bin, &output),
        },
    }
}

/// Why `bin` could not be spawned, phrased for a human.
fn unspawnable_reason(bin: &Path, e: &std::io::Error) -> String {
    match e.kind() {
        std::io::ErrorKind::NotFound if is_explicit_path(bin) => "does not exist".to_string(),
        std::io::ErrorKind::NotFound => "not found on PATH".to_string(),
        std::io::ErrorKind::PermissionDenied => "not executable".to_string(),
        _ => e.to_string(),
    }
}

/// `--version`'s first non-empty stdout line, falling back to the path when the binary ran but said
/// nothing usable (or exited non-zero).
fn version_line(bin: &Path, output: &std::process::Output) -> String {
    let fallback = || bin.display().to_string();
    if !output.status.success() {
        return fallback();
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(str::to_string)
        .unwrap_or_else(fallback)
}

/// What [`stop`] did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StopOutcome {
    /// `central proxy stop` ran and reported success.
    Stopped,
    /// central could not be spawned at all, so there was nothing to stop. `reason` says why.
    Unavailable { reason: String },
    /// central ran but reported failure. The exit code is passed through UNINTERPRETED: which code
    /// central uses for a not-running daemon is unverified, and guessing would swallow a genuine
    /// stop failure.
    Failed { code: Option<i32> },
}

/// Map a failure to SPAWN `bin` into a useful error: a `NotFound` means central is not installed,
/// which gets [`missing_central_error`]; anything else (permissions, a broken interpreter) is
/// reported as-is, since it is a real problem with a binary that DOES exist.
fn spawn_error(bin: &Path, e: std::io::Error, subcommand: &str) -> anyhow::Error {
    if e.kind() == std::io::ErrorKind::NotFound {
        return missing_central_error(bin);
    }
    anyhow::Error::new(e).context(format!("running {} {subcommand}", bin.display()))
}

/// The directory name central keeps its state in, under `$HOME`.
///
/// central moved here from `~/.wire` when it was renamed from `jbcentral` at 1.0. The legacy
/// directory is never consulted: a compat install symlinks this name to it, and central 1.0+
/// writes here directly.
pub const STATE_DIR: &str = ".jetbrains-central";

/// `$HOME`, or an error naming the failure (never a guess).
pub(crate) fn home_dir() -> anyhow::Result<PathBuf> {
    Ok(directories::BaseDirs::new()
        .ok_or_else(|| anyhow!("cannot determine home directory"))?
        .home_dir()
        .to_path_buf())
}

/// Central's state directory under `home` (see [`STATE_DIR`]).
pub fn state_dir_in(home: &Path) -> PathBuf {
    home.join(STATE_DIR)
}

/// Location of the wire config under `home`.
pub fn wire_config_path_in(home: &Path) -> PathBuf {
    state_dir_in(home).join("config.json")
}

/// Location of the wire config: `~/.jetbrains-central/config.json`.
pub fn wire_config_path() -> anyhow::Result<PathBuf> {
    Ok(wire_config_path_in(&home_dir()?))
}

/// Read + parse the wire config. Blocking filesystem I/O (R5).
pub fn read_wire_config() -> anyhow::Result<CentralInfo> {
    read_wire_config_in(&home_dir()?)
}

/// Read + parse the wire config under `home`. Both failure modes name the file, so a corrupt
/// config is never mistaken for a missing one in the surfaced error.
pub fn read_wire_config_in(home: &Path) -> anyhow::Result<CentralInfo> {
    let path = wire_config_path_in(home);
    let contents = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    parse_wire_config(&contents).with_context(|| format!("parsing {}", path.display()))
}

/// The existing wire config, or `None` when central has never written one.
///
/// A config that exists but does not parse is an Err, NOT a `None`: treating a corrupt state dir
/// as "nothing to reuse" would start a second daemon over it and bury the real cause.
pub fn existing_config_in(home: &Path) -> anyhow::Result<Option<CentralInfo>> {
    let path = wire_config_path_in(home);
    match std::fs::read_to_string(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        Ok(contents) => Ok(Some(
            parse_wire_config(&contents).with_context(|| format!("parsing {}", path.display()))?,
        )),
    }
}

// install layout ======================================================================================================

// login state =========================================================================================================

/// Result of inspecting `central status` (R20: login truth from status parsing, not "secret present").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CentralLoginState {
    LoggedIn,
    LoggedOut,
    Unknown,
}

/// Classify a `central status` run. `code` is the process exit code (`None` if the process was
/// killed by a signal). Logged-out is detected by a non-zero exit OR by an authentication-negative
/// phrase in the output, so we never silently route to Anthropic when login is actually required.
pub fn classify_login_status(code: Option<i32>, stdout: &str, stderr: &str) -> CentralLoginState {
    let combined = format!("{stdout}\n{stderr}").to_ascii_lowercase();
    let says_logged_out = combined.contains("not logged in")
        || combined.contains("not authenticated")
        || combined.contains("logged out")
        || combined.contains("please log in")
        || combined.contains("jbcentral login");
    match code {
        Some(0) if says_logged_out => CentralLoginState::LoggedOut,
        Some(0) => CentralLoginState::LoggedIn,
        Some(_) => CentralLoginState::LoggedOut,
        None => CentralLoginState::Unknown,
    }
}

/// Run `<bin> status` and classify the login state from the real exit code AND output (R20).
///
/// `classify_login_status` needs the exit code: with a `None` code it short-circuits to `Unknown`
/// and can never report logged-in/out. The `status`/`doctor` login line goes through this helper
/// so a logged-in central (exit 0 + banner) renders as such. Errors if the process cannot be
/// spawned; a non-zero exit is classified, not an error.
///
/// **R5 contract:** synchronous (spawns a child process). Call via `spawn_blocking` from async code.
pub fn run_status_classified(bin: &Path) -> anyhow::Result<CentralLoginState> {
    let output = std::process::Command::new(bin)
        .arg("status")
        .output()
        .map_err(|e| spawn_error(bin, e, "status"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    Ok(classify_login_status(output.status.code(), &stdout, &stderr))
}

// start / health / stop ===============================================================================================

/// argv for starting the proxy daemon.
pub fn proxy_start_argv() -> Vec<String> {
    vec!["proxy".to_string(), "start".to_string()]
}

/// argv for stopping the proxy daemon.
pub fn proxy_stop_argv() -> Vec<String> {
    vec!["proxy".to_string(), "stop".to_string()]
}

/// Environment overlay for the start command. When a port is requested we set `WIRE_PROXY_PORT` so
/// central binds it; otherwise we leave it to central's default/config.
pub fn start_env(port: Option<u16>) -> Vec<(String, String)> {
    match port {
        Some(p) => vec![("WIRE_PROXY_PORT".to_string(), p.to_string())],
        None => Vec::new(),
    }
}

/// The local health-probe URL for a running central daemon.
pub fn health_url(port: u16) -> String {
    format!("http://127.0.0.1:{port}/health")
}

/// Path to central's daemon PID file under `home`.
pub fn proxy_pid_path_in(home: &Path) -> PathBuf {
    state_dir_in(home).join("proxy.pid")
}

/// Path to central's daemon PID file: `~/.jetbrains-central/proxy.pid` (spec 5.7).
pub fn proxy_pid_path() -> anyhow::Result<PathBuf> {
    Ok(proxy_pid_path_in(&home_dir()?))
}

/// Per-request bound for the blocking central health probe (see [`health`]). Bounds
/// an external event (a daemon that accepts the TCP connection but never answers
/// `/health`) so a detached `spawn_blocking` probe cannot outlive a cancelled
/// caller future and leak a blocking-pool thread. Mirrors
/// `orchestrator::HEALTH_PROBE_TIMEOUT`.
const HEALTH_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// True iff `GET http://127.0.0.1:<port>/health` returns a success status.
///
/// **R5 contract:** synchronous `reqwest::blocking` GET — call via `spawn_blocking` from async code.
///
/// The client carries a bounded per-request timeout. This is the sanctioned
/// human-surfaced failure bound on an EXTERNAL event (a central daemon that
/// accepts the connection but never answers `/health`), NOT a sync-by-sleep. It
/// guarantees an unresponsive daemon fails the probe instead of hanging, so a
/// detached `spawn_blocking` probe cannot outlive a cancelled caller future and
/// leak a blocking-pool thread.
pub fn health(port: u16) -> bool {
    let url = health_url(port);
    let client = match reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(HEALTH_PROBE_TIMEOUT)
        .build()
    {
        Ok(c) => c,
        Err(_) => return false,
    };
    client
        .get(&url)
        .send()
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

/// Pure decision (testable): given the parsed wire-config `existing` and whether its daemon is
/// `healthy`, decide whether to reuse it. Reuse iff a config exists AND it is healthy. When reused,
/// the LIVE daemon's port (inside `existing`) is what the caller gets — the caller's requested port is
/// intentionally NOT consulted here (a shared singleton is never rebound). Asserted by
/// `start_reuse_keeps_live_daemon_port`.
fn reuse_decision(existing: Option<CentralInfo>, healthy: bool) -> Option<CentralInfo> {
    match existing {
        Some(info) if healthy => Some(info),
        _ => None,
    }
}

/// If a wire config already exists AND its daemon answers `/health`, return that `CentralInfo`
/// (singleton reuse — spec 5.7/§9). The returned `port` is the LIVE daemon's port read from
/// central's `config.json`, which may differ from a caller's requested port — see [`start`]'s reuse
/// note. Returns `None` when there is nothing healthy to reuse.
fn reuse_if_healthy() -> anyhow::Result<Option<CentralInfo>> {
    let existing = existing_config_in(&home_dir()?)?;
    let healthy = existing.as_ref().is_some_and(|info| health(info.port));
    Ok(reuse_decision(existing, healthy))
}

/// Start (or reuse) the central singleton. Idempotent: a healthy daemon described
/// by central's `config.json` is reused without spawning `bin`. poverty-mode never
/// runs `config set` (that would mutate the global state dir shared with the user's
/// own central). Login is assumed.
///
/// **Port semantics on reuse:** `port` is a REQUEST honored only when we actually start a new daemon.
/// JB Central is a shared singleton, so when an existing healthy daemon is reused, the live daemon's
/// already-bound port wins and the requested `port` is intentionally ignored (we never rebind a daemon
/// other sessions may be using). Callers must use the returned `CentralInfo.port`, not the requested
/// one. This is asserted by `start_reuse_keeps_live_daemon_port` in the unit tests.
///
/// **R5 contract:** synchronous (spawns a child process + blocking health GET). Call
/// via `spawn_blocking` from async code.
pub fn start(bin: &Path, port: Option<u16>) -> anyhow::Result<CentralInfo> {
    if let Some(info) = reuse_if_healthy()? {
        return Ok(info);
    }
    let mut cmd = std::process::Command::new(bin);
    cmd.args(proxy_start_argv());
    for (k, v) in start_env(port) {
        cmd.env(k, v);
    }
    let status = cmd.status().map_err(|e| spawn_error(bin, e, "proxy start"))?;
    if !status.success() {
        bail!("`central proxy start` failed (exit {:?})", status.code());
    }

    // central writes the actual port+secret here after the daemon binds; read it (do not guess).
    let info = read_wire_config().context("reading central's config.json after `central proxy start`")?;
    Ok(info)
}

/// Stop the central singleton daemon (`central proxy stop`).
///
/// Reports what happened rather than deciding: an unspawnable central is `Unavailable` (nothing to
/// stop), a non-zero exit is `Failed` with the code passed through. central's stderr is inherited,
/// so its own message reaches the user either way.
///
/// **R5 contract:** synchronous (spawns a child process). Call via `spawn_blocking` from async code.
pub fn stop(bin: &Path) -> anyhow::Result<StopOutcome> {
    match std::process::Command::new(bin).args(proxy_stop_argv()).status() {
        Err(e) => Ok(StopOutcome::Unavailable {
            reason: unspawnable_reason(bin, &e),
        }),
        Ok(status) if status.success() => Ok(StopOutcome::Stopped),
        Ok(status) => Ok(StopOutcome::Failed { code: status.code() }),
    }
}

#[cfg(test)]
#[path = "central_tests.rs"]
mod central_tests;
