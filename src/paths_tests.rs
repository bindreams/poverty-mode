use super::*;
use std::fs;

#[test]
fn atomic_write_creates_file_with_exact_bytes() {
    let dir = tempfile::TempDir::new().unwrap();
    let target = dir.path().join("data.bin");

    atomic_write(&target, b"hello world").unwrap();

    let got = fs::read(&target).unwrap();
    assert_eq!(got, b"hello world");
}

#[test]
fn atomic_write_overwrites_existing_file() {
    let dir = tempfile::TempDir::new().unwrap();
    let target = dir.path().join("data.bin");

    atomic_write(&target, b"first").unwrap();
    atomic_write(&target, b"second-and-longer").unwrap();

    let got = fs::read(&target).unwrap();
    assert_eq!(got, b"second-and-longer");
}

#[test]
fn atomic_write_leaves_no_temp_files_behind() {
    let dir = tempfile::TempDir::new().unwrap();
    let target = dir.path().join("data.bin");

    atomic_write(&target, b"payload").unwrap();

    let entries: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(entries, vec!["data.bin".to_string()]);
}

#[test]
fn atomic_write_creates_parent_dir_if_missing() {
    let dir = tempfile::TempDir::new().unwrap();
    let target = dir.path().join("nested").join("deeper").join("data.bin");

    atomic_write(&target, b"x").unwrap();

    let got = fs::read(&target).unwrap();
    assert_eq!(got, b"x");
}

#[cfg(unix)]
#[test]
fn atomic_write_hardens_file_perms_to_0600_on_unix() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::TempDir::new().unwrap();
    let target = dir.path().join("secret.bin");

    atomic_write(&target, b"secret").unwrap();

    let mode = fs::metadata(&target).unwrap().permissions().mode() & 0o777;
    assert_eq!(
        mode, 0o600,
        "config writes must be owner-only on POSIX, got {mode:o}"
    );
}

#[test]
fn with_file_lock_runs_closure_and_returns_value() {
    let dir = tempfile::TempDir::new().unwrap();
    let lock = dir.path().join("cache.lock");

    let out = with_file_lock(&lock, || Ok(42u32)).unwrap();
    assert_eq!(out, 42);
    // The lock file is created as a side effect.
    assert!(lock.exists());
}

#[test]
fn with_file_lock_propagates_closure_error() {
    let dir = tempfile::TempDir::new().unwrap();
    let lock = dir.path().join("cache.lock");

    let err = with_file_lock::<()>(&lock, || Err(anyhow::anyhow!("boom"))).unwrap_err();
    assert!(err.to_string().contains("boom"));
}

#[test]
fn with_file_lock_serializes_concurrent_holders() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::thread;

    let dir = tempfile::TempDir::new().unwrap();
    let lock = Arc::new(dir.path().join("cache.lock"));

    // `inside` counts how many threads are in the critical section at once.
    // If the lock works, the observed maximum is always 1.
    let inside = Arc::new(AtomicUsize::new(0));
    let max_seen = Arc::new(AtomicUsize::new(0));

    let mut handles = Vec::new();
    for _ in 0..8 {
        let lock = Arc::clone(&lock);
        let inside = Arc::clone(&inside);
        let max_seen = Arc::clone(&max_seen);
        handles.push(thread::spawn(move || {
            with_file_lock(&lock, || {
                let now = inside.fetch_add(1, Ordering::SeqCst) + 1;
                max_seen.fetch_max(now, Ordering::SeqCst);
                // A bounded amount of real CPU work (NOT a timed sleep) so that
                // overlap would be observable if the lock failed.
                let mut acc = 0u64;
                for i in 0..200_000u64 {
                    acc = acc.wrapping_add(i);
                }
                std::hint::black_box(acc);
                inside.fetch_sub(1, Ordering::SeqCst);
                Ok(())
            })
            .unwrap();
        }));
    }
    for h in handles {
        h.join().unwrap();
    }

    assert_eq!(max_seen.load(Ordering::SeqCst), 1);
}

#[test]
fn new_run_id_is_lowercase_crockford_base32_len_26() {
    let id = new_run_id();
    assert_eq!(id.len(), 26, "ULID canonical form is 26 chars");
    // Crockford base32 excludes I, L, O, U; lowercase per contract.
    let allowed = "0123456789abcdefghjkmnpqrstvwxyz";
    for c in id.chars() {
        assert!(
            c.is_ascii_lowercase() || c.is_ascii_digit(),
            "char {c:?} not lowercase/digit"
        );
        assert!(
            allowed.contains(c),
            "char {c:?} not in Crockford base32 alphabet"
        );
    }
}

#[test]
fn new_run_id_is_unique_across_calls() {
    let a = new_run_id();
    let b = new_run_id();
    assert_ne!(a, b);
}

#[test]
fn new_run_id_has_fixed_canonical_length() {
    // Guards against accidental non-canonical formatting: every id is the same
    // 26-char length, and two consecutive ids are never equal (uniqueness is
    // asserted above).
    let a = new_run_id();
    let b = new_run_id();
    assert_eq!(a.len(), 26);
    assert_eq!(b.len(), 26);
    assert_ne!(a, b);
}

use crate::test_support::{EnvVarGuard, XdgConfigGuard};

#[test]
fn config_path_honors_xdg_config_home_when_set() {
    let dir = tempfile::TempDir::new().unwrap();
    let _g = XdgConfigGuard::set(Some(dir.path()));

    let p = config_path().unwrap();
    assert_eq!(p, dir.path().join("poverty-mode.yaml"));
}

#[test]
fn config_path_falls_back_to_platform_dir_when_xdg_unset() {
    let _g = XdgConfigGuard::set(None);

    let p = config_path().unwrap();
    // Whatever the platform dir is, the file name must be poverty-mode.yaml.
    assert_eq!(p.file_name().unwrap(), "poverty-mode.yaml");
    // And it must be an absolute path (every platform config dir is absolute).
    assert!(
        p.is_absolute(),
        "config path must be absolute, got {}",
        p.display()
    );
}

#[test]
fn config_path_xdg_empty_is_treated_as_unset() {
    // POSIX: an empty XDG var must be ignored, the same as unset.
    let _g = XdgConfigGuard::set(Some(std::path::Path::new("")));
    let p = config_path().unwrap();
    assert_eq!(p.file_name().unwrap(), "poverty-mode.yaml");
    assert!(p.is_absolute());
}

#[test]
fn log_dir_and_cache_dir_are_absolute_and_distinct() {
    let l_dir = tempfile::TempDir::new().unwrap();
    let c_dir = tempfile::TempDir::new().unwrap();
    let _g = EnvVarGuard::set_pair(
        ("POVERTY_LOG_DIR", Some(l_dir.path())),
        ("POVERTY_CACHE_DIR", Some(c_dir.path())),
    );

    let l = log_dir().unwrap();
    let c = cache_dir().unwrap();
    assert!(l.is_absolute());
    assert!(c.is_absolute());
    assert_ne!(l, c);
}

#[test]
fn cache_dir_honors_poverty_cache_dir_override() {
    let dir = tempfile::TempDir::new().unwrap();
    let _g = EnvVarGuard::set("POVERTY_CACHE_DIR", Some(dir.path()));
    assert_eq!(cache_dir().unwrap(), dir.path());
}

#[test]
fn run_dir_is_log_dir_name() {
    let dir = tempfile::TempDir::new().unwrap();
    let _g = EnvVarGuard::set("POVERTY_LOG_DIR", Some(dir.path()));
    let id = "01hxyzrunid0000000000000abc";
    assert_eq!(run_dir(id).unwrap(), dir.path().join(id));
}

#[test]
fn ensure_run_dir_creates_log_dir_name() {
    let dir = tempfile::TempDir::new().unwrap();
    let _g = EnvVarGuard::set("POVERTY_LOG_DIR", Some(dir.path()));
    let id = new_run_id();
    let created = ensure_run_dir(&id).unwrap();
    assert_eq!(created, dir.path().join(&id));
    assert!(created.is_dir(), "ensure_run_dir must create the directory");
}

#[cfg(unix)]
#[test]
fn ensure_run_dir_hardens_dir_to_0700_on_unix() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::TempDir::new().unwrap();
    let _g = EnvVarGuard::set("POVERTY_LOG_DIR", Some(dir.path()));
    let id = new_run_id();
    let created = ensure_run_dir(&id).unwrap();
    let mode = std::fs::metadata(&created).unwrap().permissions().mode() & 0o777;
    assert_eq!(
        mode, 0o700,
        "run dir must be owner-only on POSIX, got {mode:o}"
    );
}

#[cfg(unix)]
#[test]
fn harden_dir_perms_sets_0700_on_unix() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::TempDir::new().unwrap();
    let sub = dir.path().join("sub");
    std::fs::create_dir(&sub).unwrap();
    // Loosen it first so the assertion proves harden_dir_perms tightened it.
    std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o755)).unwrap();

    harden_dir_perms(&sub).unwrap();

    let mode = std::fs::metadata(&sub).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o700, "got {mode:o}");
}

#[cfg(unix)]
#[test]
fn harden_file_perms_sets_0600_on_unix() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::TempDir::new().unwrap();
    let f = dir.path().join("log");
    std::fs::write(&f, b"x").unwrap();
    std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o644)).unwrap();

    harden_file_perms(&f).unwrap();

    let mode = std::fs::metadata(&f).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "got {mode:o}");
}

#[cfg(not(unix))]
#[test]
fn harden_perms_are_noop_on_non_unix() {
    // On Windows these must succeed without changing anything observable.
    let dir = tempfile::TempDir::new().unwrap();
    let f = dir.path().join("log");
    std::fs::write(&f, b"x").unwrap();
    harden_file_perms(&f).unwrap();
    let sub = dir.path().join("sub");
    std::fs::create_dir(&sub).unwrap();
    harden_dir_perms(&sub).unwrap();
}

#[test]
fn prune_run_dirs_keeps_newest_n_by_ulid_order() {
    // Build an isolated runs/ dir and seed it with known-ordered ULID-like ids.
    let tmp = tempfile::TempDir::new().unwrap();
    let runs = tmp.path().join("runs");
    std::fs::create_dir_all(&runs).unwrap();
    // Lexicographically ascending ids => last is "newest".
    let ids = [
        "01000000000000000000000001",
        "01000000000000000000000002",
        "01000000000000000000000003",
        "01000000000000000000000004",
        "01000000000000000000000005",
    ];
    for id in ids {
        std::fs::create_dir(runs.join(id)).unwrap();
    }
    // A stray non-directory entry must be ignored, not crash pruning.
    std::fs::write(runs.join("stray.txt"), b"x").unwrap();

    prune_run_dirs_in(&runs, 2).unwrap();

    let mut kept: Vec<String> = std::fs::read_dir(&runs)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n != "stray.txt")
        .collect();
    kept.sort();
    assert_eq!(
        kept,
        vec![
            "01000000000000000000000004".to_string(),
            "01000000000000000000000005".to_string()
        ]
    );
    // The stray file is left untouched (pruning only removes run *directories*).
    assert!(runs.join("stray.txt").exists());
}

#[test]
fn prune_run_dirs_keep_zero_removes_all_run_dirs() {
    let tmp = tempfile::TempDir::new().unwrap();
    let runs = tmp.path().join("runs");
    std::fs::create_dir_all(&runs).unwrap();
    for id in ["01000000000000000000000001", "01000000000000000000000002"] {
        std::fs::create_dir(runs.join(id)).unwrap();
    }

    prune_run_dirs_in(&runs, 0).unwrap();

    let remaining: Vec<_> = std::fs::read_dir(&runs).unwrap().collect();
    assert!(remaining.is_empty(), "keep=0 must remove every run dir");
}

#[test]
fn prune_run_dirs_no_op_when_fewer_than_keep() {
    let tmp = tempfile::TempDir::new().unwrap();
    let runs = tmp.path().join("runs");
    std::fs::create_dir_all(&runs).unwrap();
    std::fs::create_dir(runs.join("01000000000000000000000001")).unwrap();

    prune_run_dirs_in(&runs, 10).unwrap();

    assert!(runs.join("01000000000000000000000001").is_dir());
}

#[test]
fn prune_run_dirs_no_op_when_runs_dir_absent() {
    let tmp = tempfile::TempDir::new().unwrap();
    let runs = tmp.path().join("runs"); // never created
                                        // Must succeed (Ok) without error and without creating anything.
    prune_run_dirs_in(&runs, 3).unwrap();
    assert!(!runs.exists());
}

#[test]
fn prune_run_dirs_never_removes_non_ulid_directories() {
    // A non-run directory under runs/ (e.g. a user's scratch dir) must NEVER be
    // pruned, regardless of how many real runs are kept. Only valid-ULID dirs are
    // runs; everything else is left untouched (same gate as `clean`).
    let tmp = tempfile::TempDir::new().unwrap();
    let runs = tmp.path().join("runs");
    std::fs::create_dir_all(&runs).unwrap();
    std::fs::create_dir(runs.join("my-scratch-notes")).unwrap();
    std::fs::create_dir(runs.join("01000000000000000000000001")).unwrap();
    std::fs::create_dir(runs.join("01000000000000000000000002")).unwrap();

    // keep=0 removes every *run*, but the non-ULID dir is not a run.
    prune_run_dirs_in(&runs, 0).unwrap();

    assert!(
        runs.join("my-scratch-notes").is_dir(),
        "a non-ULID directory must never be pruned"
    );
    assert!(!runs.join("01000000000000000000000001").exists());
    assert!(!runs.join("01000000000000000000000002").exists());
}

#[test]
fn enumerate_run_ids_returns_ulid_dirs_sorted_skipping_others() {
    let tmp = tempfile::TempDir::new().unwrap();
    let runs = tmp.path().join("runs");
    std::fs::create_dir_all(&runs).unwrap();
    // Created out of order to prove the result is sorted ascending.
    std::fs::create_dir(runs.join("01000000000000000000000003")).unwrap();
    std::fs::create_dir(runs.join("01000000000000000000000001")).unwrap();
    std::fs::create_dir(runs.join("01000000000000000000000002")).unwrap();
    // Non-ULID dir and a stray file are both excluded.
    std::fs::create_dir(runs.join("my-scratch-notes")).unwrap();
    std::fs::write(runs.join("stray.txt"), b"x").unwrap();

    let ids = enumerate_run_ids(&runs).unwrap();
    assert_eq!(
        ids,
        vec![
            "01000000000000000000000001".to_string(),
            "01000000000000000000000002".to_string(),
            "01000000000000000000000003".to_string(),
        ]
    );
}

#[test]
fn enumerate_run_ids_empty_when_runs_dir_absent() {
    let tmp = tempfile::TempDir::new().unwrap();
    let runs = tmp.path().join("runs"); // never created
    assert!(enumerate_run_ids(&runs).unwrap().is_empty());
}

#[test]
fn enumerate_run_ids_accepts_suffixed_session_names_sorted_by_ulid() {
    let tmp = tempfile::TempDir::new().unwrap();
    let runs = tmp.path().join("logs");
    std::fs::create_dir_all(&runs).unwrap();
    // Out of chronological order on disk; ULID suffix dictates the sort.
    std::fs::create_dir(runs.join("proj-20260615-120000-01000000000000000000000003")).unwrap();
    std::fs::create_dir(runs.join("proj-20260615-110000-01000000000000000000000001")).unwrap();
    std::fs::create_dir(runs.join("other-20260615-130000-01000000000000000000000002")).unwrap();
    std::fs::create_dir(runs.join("not-a-run")).unwrap();

    let ids = enumerate_run_ids(&runs).unwrap();
    assert_eq!(
        ids,
        vec![
            "proj-20260615-110000-01000000000000000000000001".to_string(),
            "other-20260615-130000-01000000000000000000000002".to_string(),
            "proj-20260615-120000-01000000000000000000000003".to_string(),
        ]
    );
}

// log_dir / session naming =====

#[test]
fn log_dir_honors_poverty_log_dir_override() {
    let dir = tempfile::TempDir::new().unwrap();
    let _g = EnvVarGuard::set("POVERTY_LOG_DIR", Some(dir.path()));
    assert_eq!(log_dir().unwrap(), dir.path());
}

#[test]
fn log_dir_empty_override_falls_back_to_absolute() {
    let _g = EnvVarGuard::set("POVERTY_LOG_DIR", Some(std::path::Path::new("")));
    let d = log_dir().unwrap();
    assert!(
        d.is_absolute(),
        "fallback log dir must be absolute, got {}",
        d.display()
    );
    // `Path::ends_with` matches whole components; build the suffix from components so
    // it is correct on Windows (`\`) as well as Unix (`/`).
    assert!(
        d.ends_with(std::path::Path::new("poverty-mode").join("logs")),
        "fallback must be the XDG state subdir, got {}",
        d.display()
    );
}

#[test]
fn sanitize_stem_replaces_unsafe_chars_and_defaults_empty() {
    assert_eq!(sanitize_stem("poverty-mode"), "poverty-mode");
    assert_eq!(sanitize_stem("my project!"), "my_project_");
    assert_eq!(sanitize_stem("a/b\\c"), "a_b_c");
    assert_eq!(sanitize_stem(""), "root");
}

#[test]
fn new_session_name_is_findable_and_carries_a_ulid() {
    let name = new_session_name();
    // Ends with a 26-char lowercase ULID after the last dash.
    let last = name.rsplit('-').next().unwrap();
    assert_eq!(last.len(), 26, "session name must end with a ULID: {name}");
    assert!(
        run_ulid(&name).is_some(),
        "session name must be recognized as a run dir: {name}"
    );
    // Has a prefix before the ULID (stem + timestamp).
    assert!(
        name.len() > 27,
        "session name must carry stem + timestamp: {name}"
    );
}

#[test]
fn run_ulid_accepts_bare_and_suffixed_ulids_rejects_junk() {
    let ulid = "01000000000000000000000001";
    assert_eq!(run_ulid(ulid), Some(ulid));
    let suffixed = format!("poverty-mode-20260615-143022-{ulid}");
    assert_eq!(run_ulid(&suffixed), Some(ulid));
    assert_eq!(run_ulid("not-a-ulid"), None);
    assert_eq!(run_ulid("main"), None);
    assert_eq!(run_ulid("my-scratch-notes"), None);
}
