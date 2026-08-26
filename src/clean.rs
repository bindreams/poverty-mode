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
        self.run_dirs_to_delete.is_empty() && self.cache_dir_to_clear.is_none() && !self.stop_central
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
    // The single run-id-gated (`run_ulid`) run-dir enumerator (shared with
    // `paths::prune_run_dirs_in`) so a non-run directory is never scheduled for deletion.
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
        std::fs::create_dir_all(cache).with_context(|| format!("recreating cache dir {}", cache.display()))?;
    }
    Ok(())
}

use std::fmt::Write as _;
use std::io::Write as _;

/// Render a preview of the clean plan for the confirmation prompt (pure).
pub fn render_clean_plan(plan: &CleanPlan) -> String {
    if plan.is_empty() {
        return "clean: nothing to clean\n".to_string();
    }
    let mut out = String::new();
    if !plan.run_dirs_to_delete.is_empty() {
        let _ = writeln!(
            out,
            "will delete {} run director{}:",
            plan.run_dirs_to_delete.len(),
            if plan.run_dirs_to_delete.len() == 1 { "y" } else { "ies" }
        );
        for dir in &plan.run_dirs_to_delete {
            let _ = writeln!(out, "  {}", dir.display());
        }
    }
    if let Some(cache) = &plan.cache_dir_to_clear {
        let _ = writeln!(out, "will clear cache dir: {}", cache.display());
    }
    if plan.stop_central {
        let _ = writeln!(
            out,
            "will STOP the shared central singleton \
             (other live sessions using central will be disrupted)"
        );
    }
    out
}

/// The central binary `--stop-central` should spawn: the configured name, UNRESOLVED.
///
/// Deliberately not a lookup. Resolving here would both diverge from what actually spawns and turn a
/// shadowed non-executable file into a hard `clean` failure; `central::stop` reports absence instead.
fn central_stop_target(executable: Option<&str>) -> PathBuf {
    crate::central::central_executable(executable)
}

/// Read a yes/no answer from stdin. Returns true only for an explicit y/yes.
fn confirm(prompt: &str) -> Result<bool> {
    print!("{prompt}");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    let answer = line.trim().to_ascii_lowercase();
    Ok(answer == "y" || answer == "yes")
}

/// Execute a confirmed plan: stop central first (if opted in), THEN run the
/// filesystem side. `resolve_bin` and `stop` are injected so tests can drive the
/// stop path without a real central binary or process.
///
/// **Ordering is load-bearing**, though no longer because of the binary: central is external and
/// lives on `PATH`, so `--clear-cache` cannot delete it. Stopping first means a stop failure aborts
/// BEFORE any filesystem mutation, leaving the user's runs and cache untouched. Stop errors are
/// surfaced (central::stop normalizes "not running" to Ok, so any Err is a real failure).
fn execute_confirmed_clean(
    plan: &CleanPlan,
    resolve_bin: impl FnOnce() -> Result<PathBuf>,
    stop: impl FnOnce(&Path) -> Result<crate::central::StopOutcome>,
) -> Result<()> {
    // Central stop, only if opted in, and BEFORE the filesystem side so a stop failure aborts
    // without having touched the user's runs or cache.
    if plan.stop_central {
        let bin = resolve_bin()?;
        match stop(&bin)? {
            crate::central::StopOutcome::Stopped => {}
            crate::central::StopOutcome::NotInstalled => {
                println!("central not installed; nothing to stop");
            }
        }
    }

    execute_clean_plan(plan)?;
    Ok(())
}

/// Gather real inputs, preview, confirm (unless `assume_yes`), then execute. Central
/// stop happens only when `stop_central` is set AND the user confirms, AFTER the
/// emptiness check -- never on a no-op or aborted clean (R20). Stop errors propagate.
pub fn run_clean(keep: usize, clear_cache: bool, stop_central: bool, assume_yes: bool) -> Result<()> {
    let cache = crate::paths::cache_dir()?;
    let runs_root = crate::paths::log_dir()?;
    let plan = build_clean_plan(&runs_root, &cache, keep, clear_cache, stop_central)?;

    print!("{}", render_clean_plan(&plan));
    if plan.is_empty() {
        return Ok(());
    }

    if !assume_yes && !confirm("proceed? [y/N] ")? {
        println!("aborted");
        return Ok(());
    }

    // Resolve the binary the way the run does (external used as-is, else the download cache).
    // Config is read lazily — only when actually stopping central, so a plain `clean` (no
    // `--stop-central`) never loads or parses config. `execute_confirmed_clean` invokes this
    // closure only under `plan.stop_central`.
    let resolve_bin = || -> Result<PathBuf> {
        let executable = crate::config::Config::load_or_default()?.central_executable();
        Ok(central_stop_target(executable.as_deref()))
    };
    execute_confirmed_clean(&plan, resolve_bin, crate::central::stop)?;

    println!("clean complete");
    Ok(())
}
