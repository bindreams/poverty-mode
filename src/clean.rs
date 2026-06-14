//! `poverty-mode clean`: prune run dirs, clear caches, and optionally stop the
//! shared central singleton (gated; never by default -- R20).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

#[cfg(test)]
#[path = "clean_tests.rs"]
mod clean_tests;

/// Default number of newest run directories to keep when pruning.
pub const DEFAULT_KEEP_RUNS: usize = 5;

/// Given run ids sorted ascending (oldest first; ULID order == chronological),
/// return the ids to delete to keep only the newest `keep` runs.
pub fn runs_to_prune(sorted_run_ids: &[String], keep: usize) -> Vec<String> {
    if sorted_run_ids.len() <= keep {
        return Vec::new();
    }
    let cut = sorted_run_ids.len() - keep;
    sorted_run_ids[..cut].to_vec()
}

/// A previewable set of destructive actions for `clean`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CleanPlan {
    pub run_dirs_to_delete: Vec<PathBuf>,
    pub cache_dir_to_clear: Option<PathBuf>,
    /// Stop the shared central singleton. Only set when the user explicitly opts in
    /// (`--stop-central`); stopping a singleton mid-session is destructive (R20).
    pub stop_central: bool,
}

impl CleanPlan {
    pub fn is_empty(&self) -> bool {
        self.run_dirs_to_delete.is_empty()
            && self.cache_dir_to_clear.is_none()
            && !self.stop_central
    }
}

/// Build the clean plan: which run dirs to delete (keep newest `keep`), whether to
/// clear the cache, and whether to stop central. Pure w.r.t. its path arguments
/// (no deletion or process control here).
pub fn build_clean_plan(
    runs_root: &Path,
    cache_dir: &Path,
    keep: usize,
    clear_cache: bool,
    stop_central: bool,
) -> Result<CleanPlan> {
    // The single ULID-gated run-dir enumerator (shared with `paths::prune_run_dirs_in`)
    // so a non-run directory is never scheduled for deletion.
    let ids = crate::paths::enumerate_run_ids(runs_root)?;
    let to_delete: Vec<PathBuf> = runs_to_prune(&ids, keep)
        .into_iter()
        .map(|id| runs_root.join(id))
        .collect();
    let cache_dir_to_clear = if clear_cache {
        Some(cache_dir.to_path_buf())
    } else {
        None
    };
    Ok(CleanPlan {
        run_dirs_to_delete: to_delete,
        cache_dir_to_clear,
        stop_central,
    })
}

/// Remove a directory tree, treating an already-absent path as success. Ask
/// forgiveness, not permission: an `exists()`-then-`remove` probe races (the path
/// can vanish between the two), so we remove unconditionally and only swallow
/// `NotFound`. The failing path is attached on any other error.
fn remove_dir_all_idempotent(path: &Path) -> Result<()> {
    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(anyhow::Error::from(e).context(format!("removing {}", path.display()))),
    }
}

/// Execute the filesystem side of a clean plan: remove run dirs, then clear the
/// cache dir's contents (the cache dir itself is recreated empty so subsequent runs
/// find it present). Central stop is handled separately by `run_clean` (it needs the
/// installed binary path + error surfacing) and is intentionally NOT done here.
pub fn execute_clean_plan(plan: &CleanPlan) -> Result<()> {
    for dir in &plan.run_dirs_to_delete {
        remove_dir_all_idempotent(dir)?;
    }
    if let Some(cache) = &plan.cache_dir_to_clear {
        remove_dir_all_idempotent(cache)?;
        std::fs::create_dir_all(cache)
            .with_context(|| format!("recreating cache dir {}", cache.display()))?;
    }
    Ok(())
}
