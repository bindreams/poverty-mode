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
