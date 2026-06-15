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
/// builder stays pure and headless-testable. Not `Copy`: `install` carries owned
/// strings (the External display / Download versions) resolved in the probe.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CentralProbe {
    /// `/health` on the configured port returned 200.
    pub running: bool,
    /// Login state parsed from `jbcentral status`.
    pub login: CentralLogin,
    /// The configured/actual proxy port, if known.
    pub port: Option<u16>,
    /// Resolved install state. Computed in the blocking probe (it may spawn
    /// `<exe> --version` for External, or scan the cache for Download) so the
    /// report builder stays pure.
    pub install: CentralInstall,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CentralInstall {
    NotInstalled,
    Installed {
        versions: Vec<String>,
    },
    /// An external `jbcentral` binary is configured; `display` is a best-effort
    /// human label (its `--version` first line, falling back to the path).
    External {
        display: String,
    },
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
    let login = if install == CentralInstall::NotInstalled {
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
        CentralInstall::NotInstalled => {
            let _ = writeln!(out, "  central: not installed");
        }
        CentralInstall::Installed { versions } => {
            let _ = writeln!(out, "  central: installed {}", versions.join(", "));
        }
        CentralInstall::External { display } => {
            let _ = writeln!(out, "  central: external {display}");
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

/// Minimal parsed view of `~/.wire/config.json` for the live central probe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WireConfig {
    pub port: Option<u16>,
}

/// Build a Download-mode `CentralProbe` from the independent sources (pure).
///
/// - `install`: the cache-scanned install state (`NotInstalled` or `Installed`).
/// - `wire`: the parsed `~/.wire/config.json`, if any.
/// - `login`: the tri-state parsed from `jbcentral status` (Unknown if not probed).
///
/// `running` is left `false` here; the caller flips it to the real `/health` result
/// for the carried port (see `run_status`). With `NotInstalled` we emit a fully dead
/// probe so login is forced Unknown by `build_status_report`. External mode does not
/// use this helper — `run_status` builds its probe directly.
pub fn assemble_probe(install: CentralInstall, wire: Option<WireConfig>, login: CentralLogin) -> CentralProbe {
    debug_assert!(
        !matches!(install, CentralInstall::External { .. }),
        "assemble_probe is the Download-mode helper; External probes are built directly in run_status"
    );
    if install == CentralInstall::NotInstalled {
        return CentralProbe {
            running: false,
            login: CentralLogin::Unknown,
            port: None,
            install,
        };
    }
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

/// Parse the live-probe port out of `~/.wire/config.json` text. Pure (no I/O).
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

/// The configured central `executable`, read from the trailing Central entry of the
/// loaded config. `None`/blank ⇒ Download mode; `Some(..)` ⇒ External (see
/// [`crate::central::central_source`]). Mirrors the orchestrator's resolution so
/// status reports the same binary the chain would run.
fn configured_central_executable() -> Result<Option<String>> {
    // Read-only: `status` is a diagnostic and must never create `poverty-mode.yaml`
    // as a side effect (load_or_create would write the default on first run).
    Ok(crate::config::Config::load_or_default()?.central_executable())
}

/// Best-effort human label for an external central binary: the first non-empty,
/// trimmed line of `<exe> --version`'s stdout (e.g. `jbcentral 0.2.10 (commit: ...)`).
/// On ANY failure (spawn error, non-zero exit, no usable line) fall back to the path.
/// This spawns a child process (R5), so it belongs in the blocking probe — never in
/// the pure `build_status_report`.
fn external_display(exe: &Path) -> String {
    let fallback = || exe.display().to_string();
    let Ok(output) = std::process::Command::new(exe).arg("--version").output() else {
        return fallback();
    };
    if !output.status.success() {
        return fallback();
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(str::to_string)
        .unwrap_or_else(fallback)
}

/// Blocking central probe (R5): resolve install + login + run-state honoring the
/// configured `executable`. External mode labels the binary via `external_display`
/// and classifies login by running `<exe> status`; Download mode scans the managed
/// cache exactly as before. Run-state (`/health` on the wire-config port) applies to
/// both. Called via `spawn_blocking` from `run_status`.
fn probe_central() -> Result<CentralProbe> {
    let executable = configured_central_executable()?;
    let wire = read_wire_config();

    let mut probe = match crate::central::central_source(executable.as_deref()) {
        crate::central::CentralSource::External(exe) => {
            // Login truth from `<exe> status` (R20/R23c): exit code + output through the
            // canonical classifier. Unknown if the binary cannot be run.
            let login = crate::central::run_status_classified(&exe)
                .map(CentralLogin::from)
                .unwrap_or(CentralLogin::Unknown);
            CentralProbe {
                running: false,
                login,
                port: wire.and_then(|w| w.port),
                install: CentralInstall::External {
                    display: external_display(&exe),
                },
            }
        }
        crate::central::CentralSource::Download => {
            let cache = crate::paths::cache_dir()?;
            let versions = central_versions(&cache)?;
            if versions.is_empty() {
                assemble_probe(CentralInstall::NotInstalled, None, CentralLogin::Unknown)
            } else {
                // Login truth from `jbcentral status` (R20), not from any secret on disk.
                let login = match newest_central_binary(&cache)? {
                    Some(bin) => crate::central::run_status_classified(&bin)
                        .map(CentralLogin::from)
                        .unwrap_or(CentralLogin::Unknown),
                    None => CentralLogin::Unknown,
                };
                assemble_probe(CentralInstall::Installed { versions }, wire, login)
            }
        }
    };

    if let Some(port) = probe.port {
        probe.running = crate::central::health(port);
    }
    Ok(probe)
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
    let runs_root = crate::paths::log_dir()?;

    // Off-runtime: config load, install scan / `<exe> --version`, wire-config read,
    // jbcentral status parse, health. Resolution honors the configured `executable`
    // (External-by-default), not just the managed download cache.
    let probe = tokio::task::spawn_blocking(probe_central).await??;

    let report = build_status_report(&runs_root, &probe)?;
    print!("{}", render_status(&report));
    Ok(())
}
