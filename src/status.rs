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

        runs.push(RunRecord {
            run_id,
            dir,
            proxies,
        });
    }

    runs.sort_by(|a, b| crate::paths::run_ulid(&a.run_id).cmp(&crate::paths::run_ulid(&b.run_id)));
    Ok(runs)
}

/// Tri-state login, mirroring `crate::central::CentralLoginState`. Login truth is
/// parsed from `jbcentral status` (R20), never inferred from a secret's presence.
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

/// Result of probing the central singleton, supplied by the caller so the report
/// builder stays pure and headless-testable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CentralProbe {
    /// `/health` on the configured port returned 200.
    pub running: bool,
    /// Login state parsed from `jbcentral status`.
    pub login: CentralLogin,
    /// The configured/actual proxy port, if known.
    pub port: Option<u16>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CentralInstall {
    NotInstalled,
    Installed { versions: Vec<String> },
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

/// Semantic sort key for a `major.minor.patch` version string (R23f). Components
/// that fail to parse fall back to `0`, so a malformed dir name sorts as oldest
/// rather than (lexicographically) jumping ahead of real versions. This guarantees
/// `0.2.10` sorts AFTER `0.2.9`, which a plain string sort gets wrong. Shared with
/// `crate::clean` so both modules agree on "newest installed version".
pub(crate) fn version_sort_key(version: &str) -> (u64, u64, u64) {
    let mut parts = version.split('.');
    let major = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let minor = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let patch = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    (major, minor, patch)
}

/// List installed central versions by reading `<cache>/bin/jbcentral/<version>/`
/// (the canonical install dir, R4: `crate::central::INSTALL_TOOL_DIR`). Sorted
/// SEMANTICALLY by `(major, minor, patch)` (R23f) so `0.2.10 > 0.2.9` — never
/// lexicographically.
pub(crate) fn central_versions(cache_dir: &Path) -> Result<Vec<String>> {
    let bin = cache_dir.join("bin").join(crate::central::INSTALL_TOOL_DIR);
    if !bin.exists() {
        return Ok(Vec::new());
    }
    let mut versions: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(&bin)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            if let Some(name) = entry.file_name().to_str() {
                versions.push(name.to_string());
            }
        }
    }
    versions.sort_by_key(|v| version_sort_key(v));
    Ok(versions)
}

/// Assemble a full status report from explicit inputs (pure; no process spawning).
pub fn build_status_report(
    cache_dir: &Path,
    runs_root: &Path,
    probe: &CentralProbe,
) -> Result<StatusReport> {
    let versions = central_versions(cache_dir)?;
    let install = if versions.is_empty() {
        CentralInstall::NotInstalled
    } else {
        CentralInstall::Installed { versions }
    };

    let run = match (probe.running, probe.port) {
        (true, Some(port)) => CentralRun::Running { port },
        _ => CentralRun::Stopped,
    };

    // Login state is only meaningful if central is installed. Absent an install we
    // report Unknown; otherwise we pass the probe's tri-state through verbatim --
    // there is no heuristic that could manufacture a false LoggedIn.
    let login = if install == CentralInstall::NotInstalled {
        CentralLogin::Unknown
    } else {
        probe.login
    };

    Ok(StatusReport {
        first_party: vec!["pino".to_string(), "headroom".to_string()],
        central: CentralStatus {
            install,
            run,
            login,
        },
        runs: enumerate_runs(runs_root)?,
    })
}

// rendering + live probe + dispatch (M10.3) =====

use std::fmt::Write as _;

/// Render a status report as a human-facing multi-line string (pure).
pub fn render_status(report: &StatusReport) -> String {
    let mut out = String::new();

    let _ = writeln!(out, "components:");
    for fp in &report.first_party {
        let _ = writeln!(out, "  {fp} (built-in)");
    }
    match &report.central.install {
        CentralInstall::NotInstalled => {
            let _ = writeln!(out, "  central: not installed");
        }
        CentralInstall::Installed { versions } => {
            let _ = writeln!(out, "  central: installed {}", versions.join(", "));
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
            let proxies: Vec<String> = run
                .proxies
                .iter()
                .map(|p| format!("{}:{}", p.name, p.port))
                .collect();
            let _ = writeln!(out, "  {}  [{}]", run.run_id, proxies.join(", "));
        }
    }

    out
}

/// Minimal parsed view of `~/.wire/config.json` for the live central probe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WireConfig {
    pub port: Option<u16>,
}

/// Build a `CentralProbe` from the two independent sources (pure).
///
/// - `versions_present`: an install exists under `<cache>/bin/jbcentral/`.
/// - `wire`: the parsed `~/.wire/config.json`, if any.
/// - `login`: the tri-state parsed from `jbcentral status` (Unknown if not probed).
///
/// `running` is left `false` here; the caller flips it to the real `/health` result
/// for the carried port (see `run_status`). With no install we emit a fully dead
/// probe so login is forced Unknown by `build_status_report`.
pub fn assemble_probe(
    versions_present: bool,
    wire: Option<WireConfig>,
    login: CentralLogin,
) -> CentralProbe {
    if !versions_present {
        return CentralProbe {
            running: false,
            login: CentralLogin::Unknown,
            port: None,
        };
    }
    CentralProbe {
        running: false,
        login,
        port: wire.and_then(|w| w.port),
    }
}

/// Run the blocking `/health` probe off the async runtime (R5: never block the
/// executor). Returns whether `http://127.0.0.1:<port>/health` answered healthy.
pub async fn probe_health_blocking(port: u16) -> Result<bool> {
    let running = tokio::task::spawn_blocking(move || crate::central::health(port)).await?;
    Ok(running)
}

/// Parse the live-probe port out of `~/.wire/config.json` text. Pure (no I/O).
///
/// Mirrors `central::parse_wire_config`'s port coercion (some jbcentral builds write
/// `proxy_port` as a string), but unlike that helper this never requires `proxy_secret`:
/// the status probe only needs the port to decide whether to `/health`-check. This is the
/// single source of truth shared by BOTH `poverty-mode status` and `poverty-mode central
/// status` so they cannot disagree about liveness for a port-only (secretless) wire config.
pub(crate) fn parse_wire_config_port(contents: &str) -> Option<u16> {
    let json: serde_json::Value = serde_json::from_str(contents).ok()?;
    match json.get("proxy_port") {
        Some(serde_json::Value::Number(n)) => n.as_u64().and_then(|v| u16::try_from(v).ok()),
        Some(serde_json::Value::String(s)) => s.trim().parse::<u16>().ok(),
        _ => None,
    }
}

/// Read `~/.wire/config.json` and return its live-probe port. Missing/invalid -> `None`.
/// Blocking filesystem I/O (R5). Secret-free by design (see [`parse_wire_config_port`]).
pub(crate) fn wire_config_port() -> Option<u16> {
    let home = directories::BaseDirs::new()?.home_dir().to_path_buf();
    let cfg = home.join(".wire").join("config.json");
    let text = std::fs::read_to_string(&cfg).ok()?;
    parse_wire_config_port(&text)
}

/// Read `~/.wire/config.json` for the live central probe. Missing/invalid -> port None.
fn read_wire_config() -> Option<WireConfig> {
    Some(WireConfig {
        port: wire_config_port(),
    })
}

/// Locate the newest installed central binary, delegating to the canonical
/// `central::installed_binary_path_in` so BOTH the flat
/// (`<cache>/bin/jbcentral/<ver>/jbcentral`) and nested (`.../jbcentral-<ver>/jbcentral`)
/// archive layouts resolve consistently with install/clean. A flat-only lookup here would
/// miss a nested install that `central_versions` still reports as present, forcing login to
/// Unknown for a genuinely logged-in user.
///
/// Shared with `clean::run_clean` (its `--stop-central` path) so status and clean never
/// disagree about whether — or where — central is installed.
pub(crate) fn newest_central_binary(cache_dir: &Path) -> Result<Option<PathBuf>> {
    let versions = central_versions(cache_dir)?;
    let Some(latest) = versions.last() else {
        return Ok(None);
    };
    Ok(crate::central::installed_binary_path_in(cache_dir, latest))
}

/// Gather real inputs and print the status report. Side-effecting async entry point.
///
/// All blocking work (`central::health`, `jbcentral status` parsing) runs via
/// `spawn_blocking` so the tokio executor is never blocked (R5).
///
/// Note: `central`'s `/health` carries no identity (unlike the first-party
/// `/__pm/health`). We trust the configured port's `/health` here because central is
/// a forced singleton with a fixed JetBrains destination -- there is no port-squatter
/// identity concern that motivates the first-party hops' identity check.
pub async fn run_status() -> Result<()> {
    let cache = crate::paths::cache_dir()?;
    let runs_root = crate::paths::log_dir()?;

    let cache_for_blocking = cache.clone();
    // Off-runtime: install scan, wire-config read, jbcentral status parse, health.
    let probe = tokio::task::spawn_blocking(move || -> Result<CentralProbe> {
        let versions = central_versions(&cache_for_blocking)?;
        if versions.is_empty() {
            return Ok(assemble_probe(false, None, CentralLogin::Unknown));
        }
        let wire = read_wire_config();
        // Login truth from `jbcentral status` (R20), not from any secret on disk.
        // `run_status_classified` (R23c) captures BOTH the exit code and output and runs the
        // canonical `classify_login_status`; the exit code is load-bearing -- without it the
        // classifier short-circuits to Unknown and could never report logged-in/out.
        let login = match newest_central_binary(&cache_for_blocking)? {
            Some(bin) => crate::central::run_status_classified(&bin)
                .map(CentralLogin::from)
                .unwrap_or(CentralLogin::Unknown),
            None => CentralLogin::Unknown,
        };
        let mut probe = assemble_probe(true, wire, login);
        if let Some(port) = probe.port {
            probe.running = crate::central::health(port);
        }
        Ok(probe)
    })
    .await??;

    let report = build_status_report(&cache, &runs_root, &probe)?;
    print!("{}", render_status(&report));
    Ok(())
}
