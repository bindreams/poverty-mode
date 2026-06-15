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

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context};
use percent_encoding::{utf8_percent_encode, AsciiSet, CONTROLS};

use crate::download;
use crate::paths;
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

/// `…/jbcentral/latest/version.txt` — where the live latest version is published (R4).
pub fn latest_version_url() -> String {
    format!(
        "{}/jbcentral/latest/version.txt",
        crate::download::JBCENTRAL_S3_BASE
    )
}

/// Pure config-or-default version resolver (no network, R4): the trimmed `cfg_pinned` if non-blank,
/// else `DEFAULT_JBCENTRAL_VERSION`.
pub fn pinned_version(cfg_pinned: Option<&str>) -> String {
    match cfg_pinned.map(str::trim) {
        Some(v) if !v.is_empty() => v.to_string(),
        _ => DEFAULT_JBCENTRAL_VERSION.to_string(),
    }
}

/// Parse a `version.txt` body: the first non-blank, trimmed line, which must look like a dotted
/// version (digits and dots, at least one dot). Anything else is an error so the caller falls back.
pub fn parse_version_txt(body: &str) -> anyhow::Result<String> {
    let line = body
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .ok_or_else(|| anyhow!("version.txt is empty"))?;
    let looks_versiony = line.contains('.') && line.chars().all(|c| c.is_ascii_digit() || c == '.');
    if !looks_versiony {
        bail!("version.txt does not contain a dotted version: {line:?}");
    }
    Ok(line.to_string())
}

/// Resolve the jbcentral version to use (R4). If `cfg_pinned` is set (non-blank), use it. Otherwise
/// GET `<base>/jbcentral/latest/version.txt`, parse the first dotted-version line, and fall back to
/// `DEFAULT_JBCENTRAL_VERSION` on ANY failure (network, status, parse). `base` is parameterized for
/// testing; production calls [`resolve_version`].
///
/// **R5 contract:** synchronous `reqwest::blocking` GET — call via `spawn_blocking` from async code.
pub fn resolve_version_from(cfg_pinned: Option<&str>, base: &str) -> String {
    if let Some(v) = cfg_pinned.map(str::trim) {
        if !v.is_empty() {
            return v.to_string();
        }
    }
    let url = format!("{base}/jbcentral/latest/version.txt");
    let fetch = || -> anyhow::Result<String> {
        let client = reqwest::blocking::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .context("building reqwest blocking client")?;
        let body = client
            .get(&url)
            .send()
            .with_context(|| format!("GET {url}"))?
            .error_for_status()
            .with_context(|| format!("non-success status from {url}"))?
            .text()
            .with_context(|| format!("reading body of {url}"))?;
        parse_version_txt(&body)
    };
    fetch().unwrap_or_else(|_| DEFAULT_JBCENTRAL_VERSION.to_string())
}

/// Production version resolver: [`resolve_version_from`] against JetBrains' real S3 base.
///
/// **R5 contract:** synchronous — call via `spawn_blocking` from async code.
pub fn resolve_version(cfg_pinned: Option<&str>) -> String {
    resolve_version_from(cfg_pinned, crate::download::JBCENTRAL_S3_BASE)
}

// install layout =====

/// The on-disk name of the jbcentral binary for the host OS.
pub fn jbcentral_binary_name() -> &'static str {
    if cfg!(windows) {
        "jbcentral.exe"
    } else {
        "jbcentral"
    }
}

/// `<cache_root>/bin/{INSTALL_TOOL_DIR}/<version>` — the directory an asset extracts into (R4).
pub fn install_dir_in(cache_root: &Path, version: &str) -> PathBuf {
    cache_root.join("bin").join(INSTALL_TOOL_DIR).join(version)
}

/// Recursively find the jbcentral binary under `dir`. Handles assets that nest the binary one or more
/// levels deep. Deterministic: directory entries are sorted before descent so the result is stable.
pub fn find_binary_under(dir: &Path) -> Option<PathBuf> {
    let target = jbcentral_binary_name();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let mut entries: Vec<PathBuf> = match std::fs::read_dir(&d) {
            Ok(rd) => rd.flatten().map(|e| e.path()).collect(),
            Err(_) => continue,
        };
        entries.sort();
        for path in entries {
            if path.is_dir() {
                stack.push(path);
            } else if path.file_name().and_then(|s| s.to_str()) == Some(target) {
                return Some(path);
            }
        }
    }
    None
}

/// Resolve the installed jbcentral binary path for `version` under `cache_root`, scanning the version
/// dir so BOTH flat (`<dir>/jbcentral`) and nested (`<dir>/jbcentral-<ver>/jbcentral`) layouts return
/// the real path. `None` if the version dir does not contain the binary. This is the canonical
/// resolver shared with M10 status/clean.
pub fn installed_binary_path_in(cache_root: &Path, version: &str) -> Option<PathBuf> {
    let dir = install_dir_in(cache_root, version);
    if !dir.is_dir() {
        return None;
    }
    find_binary_under(&dir)
}

/// True iff the jbcentral binary for `version` is installed under `cache_root` (flat or nested).
pub fn is_installed_in(cache_root: &Path, version: &str) -> bool {
    installed_binary_path_in(cache_root, version).is_some()
}

/// Ensure `jbcentral` of `version` is installed in the managed bin cache; return the path to the
/// binary. Idempotent: if already present (flat or nested), returns its resolved path without
/// downloading. The download is sha256-verified against the per-version pin (R14, fail-closed): a
/// known target with no pin is an error.
///
/// **R5 contract:** synchronous (network + filesystem). Call via `spawn_blocking` from async code.
pub fn ensure_installed(version: &str) -> anyhow::Result<PathBuf> {
    let cache_root = paths::cache_dir().context("resolving cache dir")?;
    if let Some(bin) = installed_binary_path_in(&cache_root, version) {
        return Ok(bin);
    }

    let os = download::host_os()?;
    let arch = download::host_arch()?;
    let url = download::jbcentral_asset_url(version, os, arch)?;

    // Mandatory checksum (R14): a known target MUST have a pin. Missing pin => fail closed.
    let sha = download::pinned_sha256(version, os, arch).ok_or_else(|| {
        anyhow!(
            "no pinned sha256 for jbcentral {version} ({os}/{arch}); refusing to download \
             unverified (populate the pin per Task M8.12)"
        )
    })?;

    let dest = install_dir_in(&cache_root, version);
    download::download_verify_extract(&url, Some(sha), &dest)
        .with_context(|| format!("downloading jbcentral {version} for {os}/{arch}"))?;

    let bin = installed_binary_path_in(&cache_root, version)
        .ok_or_else(|| anyhow!("jbcentral binary not found after extracting {url}"))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&bin)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&bin, perms)?;
    }

    Ok(bin)
}

// login state =====

/// Result of inspecting `jbcentral status` (R20: login truth from status parsing, not "secret present").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CentralLoginState {
    LoggedIn,
    LoggedOut,
    Unknown,
}

/// Classify a `jbcentral status` run. `code` is the process exit code (`None` if the process was
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

/// Run `<bin> status` and return its captured stdout (R23c). Used by M10's `status`/`doctor` to render
/// the human-readable login line and by `ensure_logged_in` for classification. Errors if the process
/// cannot be spawned; a non-zero exit is NOT an error here (the stdout is still returned so the caller
/// can classify it via `classify_login_status`).
///
/// **R5 contract:** synchronous (spawns a child process). Call via `spawn_blocking` from async code.
pub fn run_status_text(bin: &Path) -> anyhow::Result<String> {
    let output = std::process::Command::new(bin)
        .arg("status")
        .output()
        .with_context(|| format!("running {} status", bin.display()))?;
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Run `<bin> status` and classify the login state from the real exit code AND output (R20).
///
/// `run_status_text` discards the exit code, but `classify_login_status` needs it: with a `None`
/// code it short-circuits to `Unknown` and can never report logged-in/out. The `status`/`doctor`
/// login line must therefore go through this helper so a logged-in central (exit 0 + banner) renders
/// as such. Errors if the process cannot be spawned; a non-zero exit is classified, not an error.
///
/// **R5 contract:** synchronous (spawns a child process). Call via `spawn_blocking` from async code.
pub fn run_status_classified(bin: &Path) -> anyhow::Result<CentralLoginState> {
    let output = std::process::Command::new(bin)
        .arg("status")
        .output()
        .with_context(|| format!("running {} status", bin.display()))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    Ok(classify_login_status(
        output.status.code(),
        &stdout,
        &stderr,
    ))
}

/// Detect login state by running `<bin> status`; if logged out, surface and run the interactive
/// `<bin> login` (inheriting stdio so the browser-OAuth flow reaches the user). Never bypasses.
///
/// **R5 contract:** synchronous (spawns child processes). Call via `spawn_blocking` from async code.
pub fn ensure_logged_in(bin: &Path) -> anyhow::Result<()> {
    let output = std::process::Command::new(bin)
        .arg("status")
        .output()
        .with_context(|| format!("running {} status", bin.display()))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    match classify_login_status(output.status.code(), &stdout, &stderr) {
        CentralLoginState::LoggedIn => Ok(()),
        CentralLoginState::Unknown => {
            bail!("`jbcentral status` was terminated abnormally; cannot determine login state")
        }
        CentralLoginState::LoggedOut => {
            eprintln!(
                "JB Central is not logged in. Launching `jbcentral login` (browser OAuth; requires a JetBrains AI Pro subscription)."
            );
            let status = std::process::Command::new(bin)
                .arg("login")
                .stdin(std::process::Stdio::inherit())
                .stdout(std::process::Stdio::inherit())
                .stderr(std::process::Stdio::inherit())
                .status()
                .with_context(|| format!("running {} login", bin.display()))?;
            if !status.success() {
                bail!(
                    "`jbcentral login` failed (exit {:?}); cannot use JB Central",
                    status.code()
                );
            }
            // Re-check: login must have actually taken effect.
            let recheck = std::process::Command::new(bin)
                .arg("status")
                .output()
                .with_context(|| format!("re-running {} status after login", bin.display()))?;
            let so = String::from_utf8_lossy(&recheck.stdout);
            let se = String::from_utf8_lossy(&recheck.stderr);
            match classify_login_status(recheck.status.code(), &so, &se) {
                CentralLoginState::LoggedIn => Ok(()),
                _ => bail!("still not logged in to JB Central after `jbcentral login`"),
            }
        }
    }
}

// start / health / stop =====

/// The `jbcentral config set ...` argv lines we apply before starting: analytics off + the RESOLVED
/// pinned version (threaded in by the orchestrator, R4 — never re-derived from a const here), so the
/// singleton stays on the same version we installed.
pub fn configure_commands(version: &str) -> Vec<Vec<String>> {
    vec![
        vec![
            "config".to_string(),
            "set".to_string(),
            "google-analytics".to_string(),
            "off".to_string(),
        ],
        vec![
            "config".to_string(),
            "set".to_string(),
            "pinned-version".to_string(),
            version.to_string(),
        ],
    ]
}

/// argv for starting the proxy daemon.
pub fn proxy_start_argv() -> Vec<String> {
    vec!["proxy".to_string(), "start".to_string()]
}

/// argv for stopping the proxy daemon.
pub fn proxy_stop_argv() -> Vec<String> {
    vec!["proxy".to_string(), "stop".to_string()]
}

/// Environment overlay for the start command. When a port is requested we set `WIRE_PROXY_PORT` so
/// jbcentral binds it; otherwise we leave it to jbcentral's default/config.
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

/// Path to jbcentral's daemon PID file: `~/.wire/proxy.pid` (spec 5.7).
pub fn proxy_pid_path() -> anyhow::Result<PathBuf> {
    let home = directories::BaseDirs::new()
        .ok_or_else(|| anyhow!("cannot determine home directory"))?
        .home_dir()
        .to_path_buf();
    Ok(home.join(".wire").join("proxy.pid"))
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
/// `~/.wire/config.json`, which may differ from a caller's requested port — see [`start`]'s reuse
/// note. Returns `None` when there is nothing healthy to reuse.
fn reuse_if_healthy() -> Option<CentralInfo> {
    let info = read_wire_config().ok()?;
    let healthy = health(info.port);
    reuse_decision(Some(info), healthy)
}

/// Configure jbcentral (analytics off + the threaded pinned `version`, R4), set the requested port,
/// start the daemon, then read `~/.wire/config.json` for the actual `proxy_port` + `proxy_secret`.
///
/// **Idempotent singleton (spec 5.7):** if `~/.wire/config.json` already describes a daemon that
/// answers `/health`, that `CentralInfo` is returned immediately without re-configuring or restarting.
///
/// **Port semantics on reuse:** `port` is a REQUEST honored only when we actually start a new daemon.
/// JB Central is a shared singleton, so when an existing healthy daemon is reused, the live daemon's
/// already-bound port wins and the requested `port` is intentionally ignored (we never rebind a daemon
/// other sessions may be using). Callers must use the returned `CentralInfo.port`, not the requested
/// one. This is asserted by `start_reuse_keeps_live_daemon_port` in the unit tests.
///
/// **R5 contract:** synchronous (spawns child processes + blocking health GET). Call via
/// `spawn_blocking` from async code.
pub fn start(bin: &Path, port: Option<u16>, version: &str) -> anyhow::Result<CentralInfo> {
    if let Some(info) = reuse_if_healthy() {
        return Ok(info);
    }

    for argv in configure_commands(version) {
        let status = std::process::Command::new(bin)
            .args(&argv)
            .status()
            .with_context(|| format!("running {} {}", bin.display(), argv.join(" ")))?;
        if !status.success() {
            bail!(
                "`jbcentral {}` failed (exit {:?})",
                argv.join(" "),
                status.code()
            );
        }
    }

    let mut cmd = std::process::Command::new(bin);
    cmd.args(proxy_start_argv());
    for (k, v) in start_env(port) {
        cmd.env(k, v);
    }
    let status = cmd
        .status()
        .with_context(|| format!("running {} proxy start", bin.display()))?;
    if !status.success() {
        bail!("`jbcentral proxy start` failed (exit {:?})", status.code());
    }

    // jbcentral writes the actual port+secret here after the daemon binds; read it (do not guess).
    let info =
        read_wire_config().context("reading ~/.wire/config.json after jbcentral proxy start")?;
    Ok(info)
}

/// Stop the central singleton daemon (`jbcentral proxy stop`). Best-effort: a not-running daemon is
/// treated as already stopped (jbcentral returns non-zero in that case, which is still "stopped").
///
/// **R5 contract:** synchronous (spawns a child process). Call via `spawn_blocking` from async code.
pub fn stop(bin: &Path) -> anyhow::Result<()> {
    let status = std::process::Command::new(bin)
        .args(proxy_stop_argv())
        .status()
        .with_context(|| format!("running {} proxy stop", bin.display()))?;
    let _ = status;
    Ok(())
}

// `central` subcommand support =====

/// A focused snapshot for `central <status>`: install presence, daemon run state,
/// and login truth. Kept independent of `crate::status`'s full report so the
/// `central status` command renders only central facts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CentralCommandStatus {
    /// Installed versions under `<cache>/bin/jbcentral/`, newest last; empty when none.
    pub versions: Vec<String>,
    /// The live daemon port (from `~/.wire/config.json`) when `/health` answers.
    pub running_port: Option<u16>,
    /// Login truth parsed from `jbcentral status` (R20).
    pub login: CentralLoginState,
}

/// Render a [`CentralCommandStatus`] as the human-facing text for `central status`.
pub fn render_central_command_status(status: &CentralCommandStatus) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    if status.versions.is_empty() {
        let _ = writeln!(out, "install: not installed");
    } else {
        let _ = writeln!(out, "install: {}", status.versions.join(", "));
    }
    match status.running_port {
        Some(port) => {
            let _ = writeln!(out, "state: running on port {port}");
        }
        None => {
            let _ = writeln!(out, "state: stopped");
        }
    }
    // Login is only meaningful with an install; absent one it is Unknown.
    let login = if status.versions.is_empty() {
        CentralLoginState::Unknown
    } else {
        status.login
    };
    let login_str = match login {
        CentralLoginState::LoggedIn => "logged in",
        CentralLoginState::LoggedOut => "logged out",
        CentralLoginState::Unknown => "unknown",
    };
    let _ = writeln!(out, "login: {login_str}");
    out
}

#[cfg(test)]
#[path = "central_tests.rs"]
mod central_tests;
