//! `poverty-mode status`: enumerate installed components, central state, and live runs.

use std::path::{Path, PathBuf};

use anyhow::Result;

#[cfg(test)]
#[path = "status_tests.rs"]
mod status_tests;

/// One proxy log file discovered inside a run directory: `<proxy>-<port>.log`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProxyLog {
    pub name: String,
    pub port: u16,
    pub log: PathBuf,
}

/// One run directory under `<log_dir>/<run_id>/`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunRecord {
    pub run_id: String,
    pub dir: PathBuf,
    pub proxies: Vec<ProxyLog>,
}

/// Parse a `<proxy>-<port>.log` file name into `(name, port)`.
///
/// Returns `None` for any name that does not end in `.log`, lacks a `-<port>`
/// segment, or whose port segment is not a valid `u16`.
fn parse_log_name(file_name: &str) -> Option<(String, u16)> {
    let stem = file_name.strip_suffix(".log")?;
    let (name, port_str) = stem.rsplit_once('-')?;
    if name.is_empty() {
        return None;
    }
    let port: u16 = port_str.parse().ok()?;
    Some((name.to_string(), port))
}

/// True iff `name` is a run directory name (carries a ULID; see `paths::run_ulid`).
fn is_run_id(name: &str) -> bool {
    crate::paths::run_ulid(name).is_some()
}

/// Enumerate run directories under `runs_root`, collecting their proxy logs.
///
/// - A missing `runs_root` is not an error; it yields an empty list.
/// - Non-directory entries directly under `runs_root` are ignored.
/// - A directory is treated as a run ONLY if `paths::run_ulid` accepts its name
///   (a bare ULID or a `<prefix>-<ULID>` session name); others are skipped so they
///   can never be enumerated (or pruned by `clean`).
/// - Within a run directory, only files matching `<proxy>-<port>.log` are collected.
/// - Runs are sorted by the embedded ULID (chronological). Within a run, proxy logs
///   are sorted ascending by `(name, port)` for deterministic output.
pub fn enumerate_runs(runs_root: &Path) -> Result<Vec<RunRecord>> {
    if !runs_root.exists() {
        return Ok(Vec::new());
    }

    let mut runs: Vec<RunRecord> = Vec::new();
    for entry in std::fs::read_dir(runs_root)? {
        let entry = entry?;
        let dir = entry.path();
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let run_id = match dir.file_name().and_then(|s| s.to_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };
        if !is_run_id(&run_id) {
            continue;
        }

        let mut proxies: Vec<ProxyLog> = Vec::new();
        for log_entry in std::fs::read_dir(&dir)? {
            let log_entry = log_entry?;
            if !log_entry.file_type()?.is_file() {
                continue;
            }
            let file_name = match log_entry.file_name().into_string() {
                Ok(s) => s,
                Err(_) => continue,
            };
            if let Some((name, port)) = parse_log_name(&file_name) {
                proxies.push(ProxyLog {
                    name,
                    port,
                    log: log_entry.path(),
                });
            }
        }
        proxies.sort_by(|a, b| (a.name.as_str(), a.port).cmp(&(b.name.as_str(), b.port)));

        runs.push(RunRecord { run_id, dir, proxies });
    }

    runs.sort_by(|a, b| crate::paths::run_ulid(&a.run_id).cmp(&crate::paths::run_ulid(&b.run_id)));
    Ok(runs)
}

/// Tri-state login, mirroring `crate::central::CentralLoginState`. Login truth is
/// parsed from `central status` (R20), never inferred from a secret's presence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CentralLogin {
    Unknown,
    LoggedOut,
    LoggedIn,
}

impl From<crate::central::CentralLoginState> for CentralLogin {
    fn from(value: crate::central::CentralLoginState) -> Self {
        match value {
            crate::central::CentralLoginState::LoggedIn => CentralLogin::LoggedIn,
            crate::central::CentralLoginState::LoggedOut => CentralLogin::LoggedOut,
            crate::central::CentralLoginState::Unknown => CentralLogin::Unknown,
        }
    }
}

/// Result of probing the central singleton, supplied by the caller so the report builder stays pure
/// and headless-testable. Not `Copy`: `install` carries owned strings resolved in the probe.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CentralProbe {
    /// `/health` on the configured port returned 200.
    pub running: bool,
    /// Login state parsed from `central status`.
    pub login: CentralLogin,
    /// The configured/actual proxy port, if known.
    pub port: Option<u16>,
    /// Whether central could be spawned. Decided in the blocking probe (which spawns
    /// `<exe> --version`) so the report builder stays pure.
    pub install: CentralInstall,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CentralInstall {
    /// central resolved; `display` is a best-effort human label (its `--version` first line,
    /// falling back to the path).
    Found { display: String },
    /// central could not be run; `looked_for` is the name tried and `reason` says why.
    NotFound { looked_for: String, reason: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CentralRun {
    Stopped,
    Running { port: u16 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CentralStatus {
    pub install: CentralInstall,
    pub run: CentralRun,
    pub login: CentralLogin,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusReport {
    /// First-party proxies are compiled into the binary; always present.
    pub first_party: Vec<String>,
    pub central: CentralStatus,
    pub runs: Vec<RunRecord>,
}

/// Assemble a full status report from explicit inputs (pure; no process spawning).
///
/// Install resolution lives on the probe (`probe.install`), not here: External mode
/// may spawn `<exe> --version` and Download mode scans the cache, both of which are
/// I/O that belongs in the blocking probe (see `run_status`).
pub fn build_status_report(runs_root: &Path, probe: &CentralProbe) -> Result<StatusReport> {
    let install = probe.install.clone();

    let run = match (probe.running, probe.port) {
        (true, Some(port)) => CentralRun::Running { port },
        _ => CentralRun::Stopped,
    };

    // Login state is only meaningful if central is installed. Absent an install we
    // report Unknown; otherwise we pass the probe's tri-state through verbatim --
    // there is no heuristic that could manufacture a false LoggedIn.
    let login = if matches!(install, CentralInstall::NotFound { .. }) {
        CentralLogin::Unknown
    } else {
        probe.login
    };

    Ok(StatusReport {
        first_party: vec!["pino".to_string(), "headroom".to_string()],
        central: CentralStatus { install, run, login },
        runs: enumerate_runs(runs_root)?,
    })
}

// rendering + live probe + dispatch (M10.3) ===========================================================================

use std::fmt::Write as _;

/// Render a status report as a human-facing multi-line string (pure).
pub fn render_status(report: &StatusReport) -> String {
    let mut out = String::new();

    let _ = writeln!(out, "components:");
    for fp in &report.first_party {
        let _ = writeln!(out, "  {fp} (built-in)");
    }
    match &report.central.install {
        CentralInstall::Found { display } => {
            let _ = writeln!(out, "  central: {display}");
        }
        CentralInstall::NotFound { looked_for, reason } => {
            let _ = writeln!(out, "  central: unavailable (`{looked_for}`: {reason})");
        }
    }

    let _ = writeln!(out, "central:");
    match &report.central.run {
        CentralRun::Stopped => {
            let _ = writeln!(out, "  state: stopped");
        }
        CentralRun::Running { port } => {
            let _ = writeln!(out, "  state: running on port {port}");
        }
    }
    let login = match report.central.login {
        CentralLogin::Unknown => "unknown",
        CentralLogin::LoggedOut => "logged out",
        CentralLogin::LoggedIn => "logged in",
    };
    let _ = writeln!(out, "  login: {login}");

    let _ = writeln!(out, "runs:");
    if report.runs.is_empty() {
        let _ = writeln!(out, "  no live runs");
    } else {
        for run in &report.runs {
            let proxies: Vec<String> = run.proxies.iter().map(|p| format!("{}:{}", p.name, p.port)).collect();
            let _ = writeln!(out, "  {}  [{}]", run.run_id, proxies.join(", "));
        }
    }

    out
}

/// Minimal parsed view of central's `config.json` for the live central probe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WireConfig {
    pub port: Option<u16>,
}

/// Build a `CentralProbe` from the independent sources (pure).
///
/// - `install`: whether central could be spawned (`Found` or `NotFound`).
/// - `wire`: the parsed central `config.json`, if any.
/// - `login`: the tri-state parsed from `central status` (Unknown if not probed).
///
/// `running` is left `false` here; the caller flips it to the real `/health` result for the carried
/// port (see `run_status`). The port is carried even for `NotFound`: a daemon started earlier can
/// still be live after its binary moves, so run-state is independent of presence.
pub fn assemble_probe(install: CentralInstall, wire: Option<WireConfig>, login: CentralLogin) -> CentralProbe {
    // A missing binary makes login unknowable, but says nothing about the daemon: one started
    // earlier can still be live, so the wire port is carried through and `/health` still decides.
    let login = match install {
        CentralInstall::NotFound { .. } => CentralLogin::Unknown,
        CentralInstall::Found { .. } => login,
    };
    CentralProbe {
        running: false,
        login,
        port: wire.and_then(|w| w.port),
        install,
    }
}

/// Run the blocking `/health` probe off the async runtime (R5: never block the
/// executor). Returns whether `http://127.0.0.1:<port>/health` answered healthy.
pub async fn probe_health_blocking(port: u16) -> Result<bool> {
    let running = tokio::task::spawn_blocking(move || crate::central::health(port)).await?;
    Ok(running)
}

/// Parse the live-probe port out of central's `config.json` text. Pure (no I/O).
///
/// Mirrors `central::parse_wire_config`'s port coercion (some jbcentral builds write
/// `proxy_port` as a string), but unlike that helper this never requires `proxy_secret`:
/// the status probe only needs the port to decide whether to `/health`-check (a port-only,
/// secretless wire config still yields a liveness verdict).
pub(crate) fn parse_wire_config_port(contents: &str) -> Option<u16> {
    let json: serde_json::Value = serde_json::from_str(contents).ok()?;
    match json.get("proxy_port") {
        Some(serde_json::Value::Number(n)) => n.as_u64().and_then(|v| u16::try_from(v).ok()),
        Some(serde_json::Value::String(s)) => s.trim().parse::<u16>().ok(),
        _ => None,
    }
}

/// Read central's `config.json` and return its live-probe port. Missing/invalid -> `None`.
/// Blocking filesystem I/O (R5). Secret-free by design (see [`parse_wire_config_port`]).
///
/// Delegates path resolution to `central::wire_config_path` so status can never disagree with
/// the start path about where central's state lives.
pub(crate) fn wire_config_port() -> Option<u16> {
    wire_config_port_in(&crate::central::home_dir().ok()?)
}

/// [`wire_config_port`] against an explicit `home`, so the resolution status actually uses is
/// testable without touching the real `$HOME` (which resolves either layout via a compat symlink).
pub(crate) fn wire_config_port_in(home: &std::path::Path) -> Option<u16> {
    let path = crate::central::wire_config_path_in(home);
    let text = std::fs::read_to_string(&path).ok()?;
    let port = parse_wire_config_port(&text);
    if port.is_none() {
        // The file exists but yields no port. Silence here would render as "stopped", which is a
        // claim we cannot support — say so instead.
        eprintln!(
            "warning: {} exists but has no usable proxy_port; central run-state is unknown",
            path.display()
        );
    }
    port
}

/// Read central's `config.json` for the live central probe. Missing/invalid -> port None.
fn read_wire_config() -> Option<WireConfig> {
    Some(WireConfig {
        port: wire_config_port(),
    })
}

/// The configured central `executable`, read from the trailing Central entry of the loaded config.
/// `None`/blank means the `central` default. Mirrors the orchestrator's resolution so status reports
/// the same binary the chain would run.
fn configured_central_executable() -> Result<Option<String>> {
    // Read-only: `status` is a diagnostic and must never create `poverty-mode.yaml`
    // as a side effect (load_or_create would write the default on first run).
    Ok(crate::config::Config::load_or_default()?.central_executable())
}

/// Blocking central probe (R5): resolve presence + login + run-state honoring the configured
/// `executable`.
///
/// Presence comes from `central::probe_presence`, i.e. an actual spawn — NOT from
/// any `is_file` lookup, which can disagree with what a run does. Login is
/// classified by running `<exe> status` with the SAME unresolved name the run would spawn. Run-state
/// (`/health` on the wire-config port) is independent of both: a daemon can be up even when the
/// binary has since been moved. Called via `spawn_blocking` from `run_status`.
fn probe_central() -> Result<CentralProbe> {
    let executable = configured_central_executable()?;
    let exe = crate::central::central_executable(executable.as_deref());
    let wire = read_wire_config();

    let install = match crate::central::probe_presence(&exe) {
        crate::central::Presence::Unavailable { reason } => CentralInstall::NotFound {
            looked_for: exe.display().to_string(),
            reason,
        },
        crate::central::Presence::Present { display } => CentralInstall::Found { display },
    };
    // Login truth from `<exe> status` (R20/R23c), only worth asking when central is present.
    // Login needs a SECOND spawn, so the two can disagree if the binary changes underneath. If the
    // login spawn cannot run central at all, downgrade the presence verdict to match: a report that
    // says both "found" and "could not run it" is incoherent.
    let (install, login) = match install {
        CentralInstall::NotFound { .. } => (install, CentralLogin::Unknown),
        CentralInstall::Found { .. } => match crate::central::run_status_classified(&exe) {
            Ok(state) => (install, CentralLogin::from(state)),
            Err(e) => match crate::central::probe_presence(&exe) {
                crate::central::Presence::Unavailable { reason } => (
                    CentralInstall::NotFound {
                        looked_for: exe.display().to_string(),
                        reason,
                    },
                    CentralLogin::Unknown,
                ),
                crate::central::Presence::Present { .. } => {
                    eprintln!("warning: could not determine central login state: {e:#}");
                    (install, CentralLogin::Unknown)
                }
            },
        },
    };

    let mut probe = assemble_probe(install, wire, login);
    if let Some(port) = probe.port {
        probe.running = crate::central::health(port);
    }
    Ok(probe)
}

/// Gather real inputs and print the status report. Side-effecting async entry point.
///
/// All blocking work (`central::health`, `central status` parsing) runs via
/// `spawn_blocking` so the tokio executor is never blocked (R5).
///
/// Note: `central`'s `/health` carries no identity (unlike the first-party
/// `/__pm/health`). We trust the configured port's `/health` here because central is
/// a forced singleton with a fixed JetBrains destination -- there is no port-squatter
/// identity concern that motivates the first-party hops' identity check.
pub async fn run_status() -> Result<()> {
    let runs_root = crate::paths::log_dir()?;

    // Off-runtime: config load, install scan / `<exe> --version`, wire-config read,
    // central status parse, health. Resolution honors the configured `executable`
    // (External-by-default), not just the managed download cache.
    let probe = tokio::task::spawn_blocking(probe_central).await??;

    let report = build_status_report(&runs_root, &probe)?;
    print!("{}", render_status(&report));
    Ok(())
}
