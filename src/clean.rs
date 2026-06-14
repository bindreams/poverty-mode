//! `poverty-mode clean`: prune run dirs, clear caches, and optionally stop the
//! shared central singleton (gated; never by default -- R20).

use std::path::{Path, PathBuf};

use anyhow::Result;

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

/// List valid-ULID run-id subdirectories of `runs_root`, sorted ascending. A
/// non-ULID directory is never returned (so it can never be pruned). Missing dir ->
/// empty.
fn sorted_run_ids(runs_root: &Path) -> Result<Vec<String>> {
    if !runs_root.exists() {
        return Ok(Vec::new());
    }
    let mut ids = Vec::new();
    for entry in std::fs::read_dir(runs_root)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            if let Some(name) = entry.file_name().to_str() {
                if ulid::Ulid::from_string(name).is_ok() {
                    ids.push(name.to_string());
                }
            }
        }
    }
    ids.sort();
    Ok(ids)
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
    let ids = sorted_run_ids(runs_root)?;
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

/// Execute the filesystem side of a clean plan: remove run dirs, then clear the
/// cache dir's contents (the cache dir itself is recreated empty so subsequent runs
/// find it present). Central stop is handled separately by `run_clean` (it needs the
/// installed binary path + error surfacing) and is intentionally NOT done here.
pub fn execute_clean_plan(plan: &CleanPlan) -> Result<()> {
    for dir in &plan.run_dirs_to_delete {
        if dir.exists() {
            std::fs::remove_dir_all(dir)?;
        }
    }
    if let Some(cache) = &plan.cache_dir_to_clear {
        if cache.exists() {
            std::fs::remove_dir_all(cache)?;
        }
        std::fs::create_dir_all(cache)?;
    }
    Ok(())
}
