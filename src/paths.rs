use anyhow::Context;
use std::fs;
use std::io::Write;
use std::path::Path;

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

#[cfg(test)]
#[path = "paths_tests.rs"]
mod paths_tests;
