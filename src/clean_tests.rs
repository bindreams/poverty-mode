use super::*;
use std::fs;

// Three valid ULIDs in ascending (oldest..newest) order.
const A: &str = "01HXXXXXXXXXXXXXXXXXXXXXXA";
const B: &str = "01HXXXXXXXXXXXXXXXXXXXXXXB";
const C: &str = "01HXXXXXXXXXXXXXXXXXXXXXXC";
const D: &str = "01HXXXXXXXXXXXXXXXXXXXXXXD";
const E: &str = "01HXXXXXXXXXXXXXXXXXXXXXXE";

#[test]
fn prune_keeps_newest_n_runs() {
    let runs = vec![
        A.to_string(),
        B.to_string(),
        C.to_string(),
        D.to_string(),
        E.to_string(),
    ];
    // Keep newest 2 => delete oldest 3.
    let to_delete = runs_to_prune(&runs, 2);
    assert_eq!(to_delete, vec![A.to_string(), B.to_string(), C.to_string()]);
}

#[test]
fn prune_keep_zero_deletes_all() {
    let runs = vec![A.to_string(), B.to_string()];
    let to_delete = runs_to_prune(&runs, 0);
    assert_eq!(to_delete, runs);
}

#[test]
fn prune_keep_more_than_present_deletes_nothing() {
    let runs = vec![A.to_string(), B.to_string()];
    let to_delete = runs_to_prune(&runs, 10);
    assert!(to_delete.is_empty());
}

#[test]
fn prune_keep_equal_to_count_deletes_nothing() {
    let runs = vec![A.to_string(), B.to_string()];
    let to_delete = runs_to_prune(&runs, 2);
    assert!(to_delete.is_empty());
}

#[test]
fn build_clean_plan_lists_run_dirs_and_cache_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let state = tmp.path().join("state");
    let cache = tmp.path().join("cache");
    let runs_root = state.join("runs");

    for id in [A, B, C] {
        fs::create_dir_all(runs_root.join(id)).unwrap();
        fs::write(runs_root.join(id).join("pino-1.log"), "x").unwrap();
    }
    fs::create_dir_all(cache.join("bin").join("jbcentral").join("0.2.9")).unwrap();

    // Keep newest 1 run, request cache clear, do NOT stop central.
    let plan = build_clean_plan(&runs_root, &cache, 1, true, false).unwrap();

    // Delete oldest two run dirs.
    assert_eq!(plan.run_dirs_to_delete.len(), 2);
    assert!(plan.run_dirs_to_delete.contains(&runs_root.join(A)));
    assert!(plan.run_dirs_to_delete.contains(&runs_root.join(B)));
    assert!(!plan.run_dirs_to_delete.contains(&runs_root.join(C)));

    // Cache cleared; central NOT stopped.
    assert_eq!(plan.cache_dir_to_clear, Some(cache.clone()));
    assert!(!plan.stop_central);
    assert!(!plan.is_empty());
}

#[test]
fn build_clean_plan_ignores_non_ulid_run_dirs() {
    // A non-ULID directory under runs/ must never be scheduled for deletion.
    let tmp = tempfile::tempdir().unwrap();
    let runs_root = tmp.path().join("runs");
    let cache = tmp.path().join("cache");
    fs::create_dir_all(runs_root.join("my-scratch-notes")).unwrap();
    fs::create_dir_all(runs_root.join(A)).unwrap();
    fs::create_dir_all(runs_root.join(B)).unwrap();

    // Keep 0 -> delete all *runs*, but the non-ULID dir is not a run.
    let plan = build_clean_plan(&runs_root, &cache, 0, false, false).unwrap();
    assert_eq!(plan.run_dirs_to_delete.len(), 2);
    assert!(!plan.run_dirs_to_delete.contains(&runs_root.join("my-scratch-notes")));
}

#[test]
fn build_clean_plan_without_cache_clear() {
    let tmp = tempfile::tempdir().unwrap();
    let runs_root = tmp.path().join("runs");
    let cache = tmp.path().join("cache");
    fs::create_dir_all(&runs_root).unwrap();

    let plan = build_clean_plan(&runs_root, &cache, 5, false, false).unwrap();
    assert!(plan.run_dirs_to_delete.is_empty());
    assert_eq!(plan.cache_dir_to_clear, None);
    assert!(!plan.stop_central);
    assert!(plan.is_empty());
}

#[test]
fn build_clean_plan_with_stop_central_only_is_not_empty() {
    // stop_central alone makes the plan non-empty (so confirmation is required).
    let tmp = tempfile::tempdir().unwrap();
    let runs_root = tmp.path().join("runs");
    let cache = tmp.path().join("cache");
    fs::create_dir_all(&runs_root).unwrap();

    let plan = build_clean_plan(&runs_root, &cache, 5, false, true).unwrap();
    assert!(plan.run_dirs_to_delete.is_empty());
    assert_eq!(plan.cache_dir_to_clear, None);
    assert!(plan.stop_central);
    assert!(!plan.is_empty());
}

#[test]
fn execute_clean_plan_removes_run_dirs_and_clears_cache() {
    let tmp = tempfile::tempdir().unwrap();
    let runs_root = tmp.path().join("runs");
    let cache = tmp.path().join("cache");
    for id in [A, B] {
        fs::create_dir_all(runs_root.join(id)).unwrap();
    }
    fs::create_dir_all(cache.join("bin")).unwrap();
    fs::write(cache.join("bin").join("f"), "x").unwrap();

    let plan = CleanPlan {
        run_dirs_to_delete: vec![runs_root.join(A)],
        cache_dir_to_clear: Some(cache.clone()),
        stop_central: false,
    };
    execute_clean_plan(&plan).unwrap();

    assert!(!runs_root.join(A).exists());
    assert!(runs_root.join(B).exists());
    // Cache dir itself remains, contents removed.
    assert!(cache.exists());
    assert!(!cache.join("bin").exists());
}

#[test]
fn execute_clean_plan_tolerates_already_absent_paths() {
    // A run dir scheduled for deletion can vanish between planning and execution
    // (concurrent run, manual rm). Removal must treat NotFound as success, never
    // abort the whole clean. Likewise a never-created cache dir is cleared+recreated.
    let tmp = tempfile::tempdir().unwrap();
    let runs_root = tmp.path().join("runs");
    let cache = tmp.path().join("cache"); // never created
    fs::create_dir_all(runs_root.join(B)).unwrap();

    let plan = CleanPlan {
        // A is already gone; B exists. Both must succeed.
        run_dirs_to_delete: vec![runs_root.join(A), runs_root.join(B)],
        cache_dir_to_clear: Some(cache.clone()),
        stop_central: false,
    };
    execute_clean_plan(&plan).unwrap();

    assert!(!runs_root.join(A).exists());
    assert!(!runs_root.join(B).exists());
    // The absent cache dir is recreated empty so subsequent runs find it present.
    assert!(cache.is_dir());
}

#[test]
fn central_stop_target_is_the_configured_name_unresolved() {
    // Never a lookup: resolving here would diverge from what actually spawns, and a shadowed
    // non-executable file on PATH would turn `clean` into a hard failure.
    assert_eq!(
        central_stop_target(Some("/opt/jb/central")),
        PathBuf::from("/opt/jb/central")
    );
    assert_eq!(central_stop_target(Some("  ")), PathBuf::from("central"));
    assert_eq!(central_stop_target(None), PathBuf::from("central"));
}

#[test]
fn stop_central_uses_external_executable_when_resolver_returns_it() {
    // External-by-default regression: with an external central configured, `--stop-central`
    // must stop THAT binary, not a cache lookup. The resolver yields the external path
    // verbatim, and stop is invoked with it.
    let tmp = tempfile::tempdir().unwrap();
    let external = tmp.path().join("central");
    std::fs::write(&external, "x").unwrap();

    let plan = CleanPlan {
        run_dirs_to_delete: vec![],
        cache_dir_to_clear: None,
        stop_central: true,
    };

    let stopped: std::cell::RefCell<Option<PathBuf>> = std::cell::RefCell::new(None);
    execute_confirmed_clean(
        &plan,
        || Ok(central_stop_target(Some(external.to_str().unwrap()))),
        |bin| {
            *stopped.borrow_mut() = Some(bin.to_path_buf());
            Ok(crate::central::StopOutcome::Stopped)
        },
    )
    .unwrap();

    assert_eq!(
        stopped.into_inner().as_deref(),
        Some(external.as_path()),
        "central::stop must be invoked with the explicitly configured executable"
    );
}

#[test]
fn confirmed_clean_without_stop_central_never_resolves_or_stops() {
    // Without --stop-central, neither the resolver nor stop runs (the resolver would
    // panic if called), and the filesystem side still executes.
    let tmp = tempfile::tempdir().unwrap();
    let cache = tmp.path().join("cache");
    fs::create_dir_all(cache.join("bin")).unwrap();
    fs::write(cache.join("bin").join("f"), "x").unwrap();

    let plan = CleanPlan {
        run_dirs_to_delete: vec![],
        cache_dir_to_clear: Some(cache.clone()),
        stop_central: false,
    };

    execute_confirmed_clean(
        &plan,
        || panic!("resolver must not run when stop_central is false"),
        |_| panic!("stop must not run when stop_central is false"),
    )
    .unwrap();

    assert!(cache.is_dir());
    assert!(!cache.join("bin").exists(), "cache cleared");
}

#[test]
fn render_clean_plan_previews_actions() {
    let plan = CleanPlan {
        run_dirs_to_delete: vec![PathBuf::from("/state/runs/01a"), PathBuf::from("/state/runs/01b")],
        cache_dir_to_clear: Some(PathBuf::from("/cache")),
        stop_central: false,
    };
    let out = render_clean_plan(&plan);
    assert!(out.contains("2 run director"), "got: {out}");
    assert!(out.contains("01a"), "got: {out}");
    assert!(out.contains("01b"), "got: {out}");
    assert!(out.contains("/cache"), "got: {out}");
    // No central line unless stop_central is set.
    assert!(!out.to_lowercase().contains("central"), "got: {out}");
}

#[test]
fn render_clean_plan_includes_central_stop_when_requested() {
    let plan = CleanPlan {
        run_dirs_to_delete: vec![],
        cache_dir_to_clear: None,
        stop_central: true,
    };
    let out = render_clean_plan(&plan);
    // The shared-singleton warning must be visible so the user knows other sessions
    // may be affected.
    assert!(out.to_lowercase().contains("central"), "got: {out}");
    assert!(out.to_lowercase().contains("shared"), "got: {out}");
    // Not "nothing to clean": stop_central alone is a real action.
    assert!(!out.contains("nothing to clean"), "got: {out}");
}

#[test]
fn render_clean_plan_empty_says_nothing_to_do() {
    let plan = CleanPlan {
        run_dirs_to_delete: vec![],
        cache_dir_to_clear: None,
        stop_central: false,
    };
    let out = render_clean_plan(&plan);
    assert!(out.contains("nothing to clean"), "got: {out}");
}

// `clean --stop-central` reports an unspawnable central instead of failing the whole clean.
#[test]
fn clean_reports_absence_without_aborting_the_filesystem_side() {
    let tmp = tempfile::tempdir().unwrap();
    let runs = tmp.path().join("runs");
    std::fs::create_dir_all(&runs).unwrap();
    let cache = tmp.path().join("cache");
    std::fs::create_dir_all(&cache).unwrap();

    let plan = build_clean_plan(&runs, &cache, 0, false, true).unwrap();
    let stopped = std::cell::RefCell::new(false);

    execute_confirmed_clean(
        &plan,
        || Ok(PathBuf::from("poverty-mode-no-such-central-xyz")),
        |_bin| {
            *stopped.borrow_mut() = true;
            Ok(crate::central::StopOutcome::NotInstalled)
        },
    )
    .unwrap();

    assert!(
        stopped.into_inner(),
        "stop is always attempted: only the OS can say whether central is spawnable"
    );
}
