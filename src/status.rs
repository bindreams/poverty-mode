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
