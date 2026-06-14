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

/// One run directory under `<state>/runs/<run_id>/`.
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

/// True iff `name` is a syntactically valid ULID (26 Crockford-base32 chars).
fn is_run_id(name: &str) -> bool {
    ulid::Ulid::from_string(name).is_ok()
}

/// Enumerate run directories under `runs_root`, collecting their proxy logs.
///
/// - A missing `runs_root` is not an error; it yields an empty list.
/// - Non-directory entries directly under `runs_root` are ignored.
/// - A directory is treated as a run ONLY if its name is a valid ULID; non-ULID
///   directories are skipped so they can never be enumerated (or pruned by `clean`).
/// - Within a run directory, only files matching `<proxy>-<port>.log` are collected.
/// - Runs are sorted ascending by `run_id` (ULID == chronological). Within a run,
///   proxy logs are sorted ascending by `(name, port)` for deterministic output.
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

    runs.sort_by(|a, b| a.run_id.cmp(&b.run_id));
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
fn central_versions(cache_dir: &Path) -> Result<Vec<String>> {
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
