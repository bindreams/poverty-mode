use super::*;
use std::fs;
use std::path::Path;

fn touch(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

// Two valid ULIDs (26 Crockford-base32 chars). Lexical order == chronological order.
const OLDER: &str = "01HXXXXXXXXXXXXXXXXXXXXXXA";
const NEWER: &str = "01HXXXXXXXXXXXXXXXXXXXXXXB";

#[test]
fn enumerate_runs_empty_when_runs_dir_missing() {
    let tmp = tempfile::tempdir().unwrap();
    // <tmp>/runs does not exist at all.
    let runs = enumerate_runs(&tmp.path().join("runs")).unwrap();
    assert!(runs.is_empty());
}

#[test]
fn enumerate_runs_empty_when_runs_dir_present_but_empty() {
    let tmp = tempfile::tempdir().unwrap();
    let runs_root = tmp.path().join("runs");
    fs::create_dir_all(&runs_root).unwrap();
    let runs = enumerate_runs(&runs_root).unwrap();
    assert!(runs.is_empty());
}

#[test]
fn enumerate_runs_collects_proxy_logs_sorted_by_run_id() {
    let tmp = tempfile::tempdir().unwrap();
    let runs_root = tmp.path().join("runs");

    touch(&runs_root.join(NEWER).join("pino-51001.log"), "log\n");
    touch(&runs_root.join(NEWER).join("headroom-51002.log"), "log\n");
    touch(&runs_root.join(OLDER).join("central-9100.log"), "log\n");

    // A stray non-directory entry and a non-.log file must be ignored.
    touch(&runs_root.join("stray.txt"), "ignore me");
    touch(&runs_root.join(OLDER).join("notes.md"), "ignore me");

    let runs = enumerate_runs(&runs_root).unwrap();
    assert_eq!(runs.len(), 2);

    // Sorted ascending by run_id => older first.
    assert_eq!(runs[0].run_id, OLDER);
    assert_eq!(runs[1].run_id, NEWER);

    // older run: one proxy log (central-9100).
    assert_eq!(runs[0].proxies.len(), 1);
    assert_eq!(runs[0].proxies[0].name, "central");
    assert_eq!(runs[0].proxies[0].port, 9100);
    assert_eq!(
        runs[0].proxies[0].log,
        runs_root.join(OLDER).join("central-9100.log")
    );

    // newer run: two proxy logs, sorted by name within the run.
    assert_eq!(runs[1].proxies.len(), 2);
    assert_eq!(runs[1].proxies[0].name, "headroom");
    assert_eq!(runs[1].proxies[0].port, 51002);
    assert_eq!(runs[1].proxies[1].name, "pino");
    assert_eq!(runs[1].proxies[1].port, 51001);
}

#[test]
fn enumerate_runs_skips_logs_without_port_suffix() {
    let tmp = tempfile::tempdir().unwrap();
    let runs_root = tmp.path().join("runs");

    touch(&runs_root.join(OLDER).join("pino-51001.log"), "ok");
    // Malformed: no "-<port>" segment, or a non-numeric/over-u16 port -> skipped.
    touch(&runs_root.join(OLDER).join("garbage.log"), "skip");
    touch(&runs_root.join(OLDER).join("pino-notaport.log"), "skip");
    touch(&runs_root.join(OLDER).join("pino-99999.log"), "skip"); // > u16::MAX

    let runs = enumerate_runs(&runs_root).unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].proxies.len(), 1);
    assert_eq!(runs[0].proxies[0].name, "pino");
    assert_eq!(runs[0].proxies[0].port, 51001);
}

#[test]
fn enumerate_runs_skips_non_ulid_directories() {
    // A directory under runs/ whose name is NOT a valid ULID must be ignored,
    // so it can never be enumerated (and thus never pruned by `clean`).
    let tmp = tempfile::tempdir().unwrap();
    let runs_root = tmp.path().join("runs");

    touch(&runs_root.join(NEWER).join("pino-51001.log"), "real run");
    // Human-created stray dir that is not a ULID.
    fs::create_dir_all(runs_root.join("my-scratch-notes")).unwrap();
    fs::write(runs_root.join("my-scratch-notes").join("a.log"), "x").unwrap();
    // A 26-char-but-invalid-base32 name (contains 'I', 'L', 'O', 'U' which Crockford excludes).
    fs::create_dir_all(runs_root.join("ILOUILOUILOUILOUILOUILOUIL")).unwrap();

    let runs = enumerate_runs(&runs_root).unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].run_id, NEWER);
}
