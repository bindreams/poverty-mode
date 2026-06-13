use anyhow::Context;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Write `bytes` to `path` atomically: write to a uniquely-named temp file in the
/// SAME directory, fsync it, harden its permissions on POSIX, then rename it over
/// the target. A same-directory rename is atomic on every supported filesystem, so
/// a reader never observes a half-written file. Missing parent directories are
/// created. On POSIX the resulting file is owner-only (0600); on Windows the mode
/// step is a no-op (acceptable — see spec 12).
pub fn atomic_write(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| Path::new(".").to_path_buf());
    fs::create_dir_all(&parent)
        .with_context(|| format!("creating parent dir {}", parent.display()))?;

    let mut tmp = tempfile::Builder::new()
        .prefix(".pm-tmp-")
        .tempfile_in(&parent)
        .with_context(|| format!("creating temp file in {}", parent.display()))?;
    tmp.write_all(bytes).context("writing temp file")?;
    tmp.as_file().sync_all().context("fsync temp file")?;
    harden_file_perms(tmp.path())
        .with_context(|| format!("hardening perms on temp file in {}", parent.display()))?;
    tmp.persist(path)
        .map_err(|e| e.error)
        .with_context(|| format!("renaming temp file onto {}", path.display()))?;
    Ok(())
}

/// Restrict `path` to owner read/write (0600) on POSIX. No-op on non-Unix targets.
#[cfg(unix)]
pub fn harden_file_perms(path: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .with_context(|| format!("setting 0600 on {}", path.display()))
}

/// No-op on non-Unix targets (Windows has no POSIX mode bits; body logging is off
/// by default — see spec 12/15).
#[cfg(not(unix))]
pub fn harden_file_perms(_path: &Path) -> anyhow::Result<()> {
    Ok(())
}

use fs2::FileExt;

/// Run `f` while holding an advisory EXCLUSIVE lock on `lock_path`. The lock file
/// is created if missing and is never deleted (the lock is the open handle, not
/// the file's existence). The lock is released when the handle is dropped, which
/// happens on every exit path — normal return, `?` early-return, or panic — so no
/// explicit unlock is needed.
pub fn with_file_lock<T>(
    lock_path: &Path,
    f: impl FnOnce() -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    if let Some(parent) = lock_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating lock dir {}", parent.display()))?;
    }
    let file = fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        // Never truncate: the lock is the open handle, not the file's contents, and
        // truncation would needlessly mutate a file shared across processes.
        .truncate(false)
        .open(lock_path)
        .with_context(|| format!("opening lock file {}", lock_path.display()))?;
    file.lock_exclusive()
        .with_context(|| format!("locking {}", lock_path.display()))?;
    // `file` is a named local: it lives until the end of this function, so the lock
    // is held across `f()` and released when the handle is dropped on return — on
    // every exit path (normal, `?` early-return, or panic). We rely on close-on-drop
    // rather than an explicit RAII unlock guard, whose only effect would be releasing
    // microseconds earlier — fully overlapped by the guaranteed close-on-drop.
    f()
}

/// A fresh, time-sortable run id: a ULID rendered in lowercase Crockford base32.
/// 26 chars, lexicographically monotonic by creation time, collision-resistant.
pub fn new_run_id() -> String {
    ulid::Ulid::new().to_string().to_lowercase()
}

const APP_QUALIFIER: &str = "";
const APP_ORG: &str = "poverty-mode";
const APP_NAME: &str = "poverty-mode";
const CONFIG_FILE_NAME: &str = "poverty-mode.yaml";
const STATE_DIR_ENV: &str = "POVERTY_STATE_DIR";
const CACHE_DIR_ENV: &str = "POVERTY_CACHE_DIR";

fn project_dirs() -> anyhow::Result<directories::ProjectDirs> {
    directories::ProjectDirs::from(APP_QUALIFIER, APP_ORG, APP_NAME)
        .context("could not determine platform application directories")
}

/// Read an env override, returning `Some(PathBuf)` only when the var is set to a
/// non-empty value (an empty value is treated as unset, like `XDG_CONFIG_HOME`).
fn env_dir_override(var: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// Absolute path to the config file. Honors `XDG_CONFIG_HOME` on every OS when it
/// is set to a non-empty value (`<XDG_CONFIG_HOME>/poverty-mode.yaml`); otherwise
/// the platform config directory.
pub fn config_path() -> anyhow::Result<PathBuf> {
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return Ok(PathBuf::from(xdg).join(CONFIG_FILE_NAME));
        }
    }
    Ok(project_dirs()?.config_dir().join(CONFIG_FILE_NAME))
}

/// Absolute path to the per-user state directory (holds `runs/`). Honors the
/// `POVERTY_STATE_DIR` env override (non-empty value wins) for hermetic tests
/// (R23j); otherwise the platform `data_dir`.
pub fn state_dir() -> anyhow::Result<PathBuf> {
    if let Some(dir) = env_dir_override(STATE_DIR_ENV) {
        return Ok(dir);
    }
    Ok(project_dirs()?.data_dir().to_path_buf())
}

/// Absolute path to the per-user cache directory (holds downloaded binaries).
/// Honors the `POVERTY_CACHE_DIR` env override (non-empty value wins) for hermetic
/// tests (R23j); otherwise the platform `cache_dir`.
pub fn cache_dir() -> anyhow::Result<PathBuf> {
    if let Some(dir) = env_dir_override(CACHE_DIR_ENV) {
        return Ok(dir);
    }
    Ok(project_dirs()?.cache_dir().to_path_buf())
}

/// Absolute path to a single run's directory: `<state>/runs/<run_id>`. Pure path
/// math; this does not create the directory (see `ensure_run_dir`).
pub fn run_dir(run_id: &str) -> anyhow::Result<PathBuf> {
    Ok(state_dir()?.join("runs").join(run_id))
}

/// Restrict `path` to owner-only directory access (0700) on POSIX. No-op on
/// non-Unix targets.
#[cfg(unix)]
pub fn harden_dir_perms(path: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .with_context(|| format!("setting 0700 on {}", path.display()))
}

/// No-op on non-Unix targets.
#[cfg(not(unix))]
pub fn harden_dir_perms(_path: &Path) -> anyhow::Result<()> {
    Ok(())
}

/// Create `<state>/runs/<run_id>` (and any missing parents), then harden it to
/// 0700 on POSIX, and return its absolute path. Idempotent: an existing run dir is
/// re-hardened and returned. Per-run unique paths mean concurrent sessions never
/// share a directory (spec 5.11).
pub fn ensure_run_dir(run_id: &str) -> anyhow::Result<PathBuf> {
    let dir = run_dir(run_id)?;
    fs::create_dir_all(&dir).with_context(|| format!("creating run dir {}", dir.display()))?;
    harden_dir_perms(&dir)?;
    Ok(dir)
}

/// Keep the `keep` most-recent run directories under `<state>/runs/` and remove
/// the rest. Called once on startup by the orchestrator (M6). Thin wrapper over
/// `prune_run_dirs_in` against the real state-derived runs directory.
pub fn prune_run_dirs(keep: usize) -> anyhow::Result<()> {
    let runs = state_dir()?.join("runs");
    prune_run_dirs_in(&runs, keep)
}

/// Keep the `keep` most-recent run directories in `runs_dir` and remove the rest.
/// Recency is by directory-name lexical order (ULID run-ids sort by creation
/// time — no mtime/timer is consulted). Non-directory entries are ignored. A
/// missing `runs_dir` is a no-op. Removal errors are surfaced (not swallowed).
fn prune_run_dirs_in(runs_dir: &Path, keep: usize) -> anyhow::Result<()> {
    let read = match fs::read_dir(runs_dir) {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => {
            return Err(anyhow::Error::from(e))
                .with_context(|| format!("reading runs dir {}", runs_dir.display()));
        }
    };

    let mut dirs: Vec<(String, PathBuf)> = Vec::new();
    for entry in read {
        let entry = entry
            .with_context(|| format!("reading entry under {}", runs_dir.display()))?;
        let file_type = entry
            .file_type()
            .with_context(|| format!("stat {}", entry.path().display()))?;
        if file_type.is_dir() {
            let name = entry.file_name().to_string_lossy().into_owned();
            dirs.push((name, entry.path()));
        }
    }

    // Sort ascending by name: oldest first, newest last.
    dirs.sort_by(|a, b| a.0.cmp(&b.0));

    let remove_count = dirs.len().saturating_sub(keep);
    for (_name, path) in dirs.into_iter().take(remove_count) {
        fs::remove_dir_all(&path)
            .with_context(|| format!("pruning old run dir {}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "paths_tests.rs"]
mod paths_tests;
