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
    assert!(!plan
        .run_dirs_to_delete
        .contains(&runs_root.join("my-scratch-notes")));
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
