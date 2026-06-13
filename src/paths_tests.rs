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
    assert_eq!(mode, 0o600, "config writes must be owner-only on POSIX, got {mode:o}");
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
        assert!(allowed.contains(c), "char {c:?} not in Crockford base32 alphabet");
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
