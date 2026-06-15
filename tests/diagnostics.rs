use assert_cmd::Command;
use predicates::prelude::*;
use std::path::Path;
use tempfile::TempDir;

mod common;
use common::fakebin::write_fake_jbcentral;

/// Build a Command for the binary with explicit, injected state/cache/config roots.
fn pm(home: &TempDir) -> Command {
    let mut cmd = Command::cargo_bin("poverty-mode").unwrap();
    let p = home.path();
    cmd.env("POVERTY_LOG_DIR", p.join("logs"))
        .env("POVERTY_CACHE_DIR", p.join("cache"))
        .env("XDG_CONFIG_HOME", p.join("config"))
        // Point managed-settings detection at a hermetic (absent) path so a real
        // system managed-settings.json never bleeds into doctor.
        .env("POVERTY_MANAGED_SETTINGS", p.join("managed-settings.json"))
        // Neutralize a real ~/.claude and ~/.wire bleeding into doctor/status.
        .env("HOME", p)
        .env("USERPROFILE", p);
    cmd
}

/// The runs root the binary will use given the injected POVERTY_LOG_DIR.
fn runs_root(home: &TempDir) -> std::path::PathBuf {
    home.path().join("logs")
}

/// The config file path the binary will use given the injected XDG_CONFIG_HOME.
fn config_file(home: &TempDir) -> std::path::PathBuf {
    home.path().join("config").join("poverty-mode.yaml")
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

/// Write the given central `settings:` YAML body into the injected config home so a
/// hermetic `status` run resolves a known central source instead of writing (and then
/// honoring) the External-by-default `default_all_disabled` config.
fn seed_central_config(home: &TempDir, settings_body: &str) {
    let config_dir = home.path().join("config");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::write(
        config_dir.join("poverty-mode.yaml"),
        format!(
            "version: 1\n\
             proxies:\n\
             - name: central\n\
             \x20\x20enabled: false\n\
             \x20\x20settings:\n{settings_body}\
             defaults:\n\
             \x20\x20enable_tool_search: true\n"
        ),
    )
    .unwrap();
}

#[test]
fn status_runs_and_reports_no_runs_on_clean_machine() {
    let home = TempDir::new().unwrap();
    // Download-mode config (`executable: null`): status scans the (empty) managed cache
    // rather than probing an external binary, so a clean machine reads "not installed".
    seed_central_config(&home, "    executable: null\n");
    pm(&home)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("pino (built-in)"))
        .stdout(predicate::str::contains("central: not installed"))
        .stdout(predicate::str::contains("no live runs"));
}

#[test]
fn status_reports_configured_external_central() {
    // External-by-default: with `executable` set, status reports the binary's
    // `--version` first line, not the managed cache. A fake jbcentral keeps this
    // deterministic regardless of what (if anything) is on the runner's PATH.
    let home = TempDir::new().unwrap();
    // `--version` => known line; `status` => exit 1 (logged out).
    let exe = write_fake_jbcentral(home.path(), "jbcentral 9.9.9 (fake)", 1);
    seed_central_config(&home, &format!("    executable: {}\n", exe.display()));

    pm(&home)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("central: external jbcentral 9.9.9 (fake)"))
        .stdout(predicate::str::contains("not installed").not());
}

#[test]
fn status_lists_seeded_run() {
    let home = TempDir::new().unwrap();
    // Download-mode config so the run-listing assertion does not depend on (or spawn)
    // an external central binary from the runner's PATH.
    seed_central_config(&home, "    executable: null\n");
    seed_run(&runs_root(&home), A);
    pm(&home)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains(A))
        .stdout(predicate::str::contains("pino:40001"));
}

#[test]
fn status_does_not_create_config_file_on_clean_machine() {
    // Read-only diagnostic: `status` on a machine with NO config file must report its
    // findings WITHOUT creating `poverty-mode.yaml` as a side effect (regression guard
    // for load_or_create vs. load_or_default). No config is seeded here.
    let home = TempDir::new().unwrap();
    assert!(!config_file(&home).exists());

    pm(&home)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("pino (built-in)"));

    assert!(!config_file(&home).exists(), "status must not create the config file");
}

#[test]
fn doctor_does_not_create_config_file_on_clean_machine() {
    // `doctor` is read-only too: it must never create `poverty-mode.yaml`.
    let home = TempDir::new().unwrap();
    assert!(!config_file(&home).exists());

    pm(&home).arg("doctor").assert().success();

    assert!(!config_file(&home).exists(), "doctor must not create the config file");
}

#[test]
fn doctor_runs_and_exits_zero_when_no_settings_conflicts() {
    let home = TempDir::new().unwrap();
    // Empty HOME -> no ~/.claude/settings.json -> no base-url conflict. On the five
    // supported CI targets there are no Error-severity toolchain findings, so exit 0.
    pm(&home)
        .arg("doctor")
        .assert()
        .success()
        .stdout(predicate::str::contains("no problems detected").or(predicate::str::contains("WARN")));
}

#[test]
fn doctor_reports_managed_policy_conflict_as_error_and_exits_nonzero() {
    // End-to-end proof that managed-policy detection is wired into production: a
    // managed-settings.json that pins ANTHROPIC_BASE_URL is the ONLY Severity::Error
    // settings path. doctor must surface it as an ERROR and exit non-zero so a CI/admin
    // gate fails when the chain would be bypassed by managed policy.
    let home = TempDir::new().unwrap();
    let managed = home.path().join("managed-settings.json");
    std::fs::write(
        &managed,
        r#"{"env": {"ANTHROPIC_BASE_URL": "https://locked.corp.example"}}"#,
    )
    .unwrap();
    pm(&home)
        .env("POVERTY_MANAGED_SETTINGS", &managed)
        .arg("doctor")
        .assert()
        .failure()
        .stdout(predicate::str::contains("ERROR"))
        .stdout(predicate::str::contains("managed policy"));
}

#[test]
fn doctor_no_managed_error_when_managed_file_absent() {
    // The default hermetic managed path does not exist -> no Managed error, exit 0
    // (proves the managed branch fires ONLY on an actual managed override).
    let home = TempDir::new().unwrap();
    pm(&home)
        .arg("doctor")
        .assert()
        .success()
        .stdout(predicate::str::contains("managed policy").not());
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
