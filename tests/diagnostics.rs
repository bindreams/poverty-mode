use assert_cmd::Command;
use predicates::prelude::*;
use std::path::Path;
use tempfile::TempDir;

/// Build a Command for the binary with explicit, injected state/cache/config roots.
fn pm(home: &TempDir) -> Command {
    let mut cmd = Command::cargo_bin("poverty-mode").unwrap();
    let p = home.path();
    cmd.env("POVERTY_STATE_DIR", p.join("state"))
        .env("POVERTY_CACHE_DIR", p.join("cache"))
        .env("XDG_CONFIG_HOME", p.join("config"))
        // Neutralize a real ~/.claude and ~/.wire bleeding into doctor/status.
        .env("HOME", p)
        .env("USERPROFILE", p);
    cmd
}

/// The runs root the binary will use given the injected POVERTY_STATE_DIR.
fn runs_root(home: &TempDir) -> std::path::PathBuf {
    home.path().join("state").join("runs")
}

// Valid ULIDs (ascending == chronological).
const A: &str = "01HXXXXXXXXXXXXXXXXXXXXXXA";
const B: &str = "01HXXXXXXXXXXXXXXXXXXXXXXB";
const C: &str = "01HXXXXXXXXXXXXXXXXXXXXXXC";

fn seed_run(root: &Path, id: &str) {
    let dir = root.join(id);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("pino-40001.log"), "x").unwrap();
}

#[test]
fn status_runs_and_reports_no_runs_on_clean_machine() {
    let home = TempDir::new().unwrap();
    pm(&home)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("pino (built-in)"))
        .stdout(predicate::str::contains("central: not installed"))
        .stdout(predicate::str::contains("no live runs"));
}

#[test]
fn status_lists_seeded_run() {
    let home = TempDir::new().unwrap();
    seed_run(&runs_root(&home), A);
    pm(&home)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains(A))
        .stdout(predicate::str::contains("pino:40001"));
}

#[test]
fn doctor_runs_and_exits_zero_when_no_settings_conflicts() {
    let home = TempDir::new().unwrap();
    // Empty HOME -> no ~/.claude/settings.json -> no base-url conflict. On the five
    // supported CI targets there are no Error-severity toolchain findings, so exit 0.
    pm(&home).arg("doctor").assert().success().stdout(
        predicate::str::contains("no problems detected").or(predicate::str::contains("WARN")),
    );
}

#[test]
fn clean_with_nothing_to_do_succeeds() {
    let home = TempDir::new().unwrap();
    pm(&home)
        .args(["clean", "--yes"])
        .assert()
        .success()
        .stdout(predicate::str::contains("nothing to clean"));
}

#[test]
fn clean_actually_prunes_old_run_dirs_with_yes() {
    let home = TempDir::new().unwrap();
    let root = runs_root(&home);
    for id in [A, B, C] {
        seed_run(&root, id);
    }
    // Sanity: all three exist before clean.
    assert!(root.join(A).exists());
    assert!(root.join(B).exists());
    assert!(root.join(C).exists());

    pm(&home)
        .args(["clean", "--keep", "1", "--yes"])
        .assert()
        .success()
        .stdout(predicate::str::contains("clean complete"));

    // Keep newest 1 (C) -> A and B deleted, C survives. Assert ACTUAL deletion.
    assert!(!root.join(A).exists(), "oldest run A should be pruned");
    assert!(!root.join(B).exists(), "run B should be pruned");
    assert!(root.join(C).exists(), "newest run C must survive");
}

#[test]
fn clean_does_not_stop_central_without_flag() {
    // With no --stop-central, the preview must never mention stopping central, even
    // when there is other work to do (shared-singleton safety, R20).
    let home = TempDir::new().unwrap();
    seed_run(&runs_root(&home), A);
    seed_run(&runs_root(&home), B);
    pm(&home)
        .args(["clean", "--keep", "1", "--yes"])
        .assert()
        .success()
        .stdout(predicate::str::contains("will STOP the shared central").not());
}
