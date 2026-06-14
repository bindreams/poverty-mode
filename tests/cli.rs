use assert_cmd::Command;
use predicates::str::contains;

#[test]
fn help_lists_all_subcommands() {
    let mut cmd = Command::cargo_bin("poverty-mode").unwrap();
    cmd.arg("--help")
        .assert()
        .success()
        .stdout(contains("run"))
        .stdout(contains("proxy"))
        .stdout(contains("central"))
        .stdout(contains("config"))
        .stdout(contains("status"))
        .stdout(contains("doctor"))
        .stdout(contains("clean"));
}

/// Exercises the library crate import path R1 mandates: parse argv with the
/// public `Cli`, then dispatch through the public `dispatch`. `config` is still a
/// `NotImplemented` stub (M10.3 wired `status` to the real handler), so it remains
/// the reachable-stub probe for this library-import path.
#[test]
fn library_dispatch_is_reachable_for_stub_subcommand() {
    use clap::Parser;
    use poverty_mode::cli::{dispatch, Cli};

    let cli = Cli::try_parse_from(["poverty-mode", "config", "path"]).unwrap();
    let err = dispatch(cli).unwrap_err();
    assert!(
        err.to_string().contains("not yet implemented: config"),
        "library dispatch should return the stub error, got: {err}"
    );
}

// ---- Characterization guards (R12): added AFTER the dispatch stubs exist in
// M1.6. They lock in the stub contract so a later milestone wiring a real
// handler gets an immediate failing test. Not a red->green cycle. ----

/// M10.3 wired `status` to the real handler (R23g): the M3 NotImplemented arm is
/// gone, so the end-to-end `status` invocation now succeeds. Hermetic via the
/// `POVERTY_CACHE_DIR`/`POVERTY_STATE_DIR` overrides (R23j): an empty cache dir
/// yields "not installed" and short-circuits the live central probe (no spawning,
/// no `~/.wire` read); an empty state dir yields "no live runs".
#[test]
fn status_subcommand_runs_and_renders() {
    let tmp = tempfile::tempdir().unwrap();
    let mut cmd = Command::cargo_bin("poverty-mode").unwrap();
    cmd.arg("status")
        .env("POVERTY_CACHE_DIR", tmp.path().join("cache"))
        .env("POVERTY_STATE_DIR", tmp.path().join("state"))
        .assert()
        .success()
        .stdout(contains("pino (built-in)"))
        .stdout(contains("headroom (built-in)"))
        .stdout(contains("central: not installed"))
        .stdout(contains("no live runs"));
}

/// M10.5 wired `doctor` to the real handler (R23g): the M3 NotImplemented arm is
/// gone, so the end-to-end `doctor` invocation now runs the toolchain/settings
/// checks and renders findings. Run from a temp CWD with no project `.claude`
/// layers so the project-settings sources are empty; the exit code still depends
/// on host-level layers (a managed-policy `ANTHROPIC_BASE_URL` would be an
/// `Error`), so this asserts only that real doctor diagnostics are produced — not
/// a fixed exit code.
#[test]
fn doctor_subcommand_runs_and_renders() {
    let tmp = tempfile::tempdir().unwrap();
    let mut cmd = Command::cargo_bin("poverty-mode").unwrap();
    let output = cmd.arg("doctor").current_dir(tmp.path()).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Real diagnostics, not the old NotImplemented stub.
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("not yet implemented"),
        "doctor must be implemented, got stderr: {stderr}"
    );
    assert!(
        stdout.contains("no problems detected")
            || stdout.contains("WARN")
            || stdout.contains("ERROR"),
        "expected doctor diagnostics, got stdout: {stdout:?} stderr: {stderr:?}"
    );
}

/// M10.7 wired `clean` to the real handler (R23g): the M3 NotImplemented arm is
/// gone, so the end-to-end `clean` invocation now builds a real plan. Hermetic via
/// the `POVERTY_CACHE_DIR`/`POVERTY_STATE_DIR` overrides (R23j): an empty state dir
/// (no run dirs) with the default `--keep` and no `--clear-cache`/`--stop-central`
/// yields an empty plan, which short-circuits to "nothing to clean" and exits
/// success WITHOUT prompting -- so this is safe to run non-interactively.
#[test]
fn clean_subcommand_empty_plan_says_nothing_to_clean() {
    let tmp = tempfile::tempdir().unwrap();
    let mut cmd = Command::cargo_bin("poverty-mode").unwrap();
    cmd.arg("clean")
        .env("POVERTY_CACHE_DIR", tmp.path().join("cache"))
        .env("POVERTY_STATE_DIR", tmp.path().join("state"))
        .assert()
        .success()
        .stdout(contains("nothing to clean"));
}

/// A non-empty clean with `--yes` bypasses the interactive confirmation and runs
/// to completion. `--clear-cache` makes the plan non-empty even with no run dirs;
/// `--yes` skips the prompt, so the side effects execute and "clean complete" is
/// printed. `--stop-central` is NOT passed, so the shared singleton is untouched
/// (R20) -- no central binary exists in the hermetic cache anyway.
#[test]
fn clean_subcommand_yes_clears_cache_and_completes() {
    let tmp = tempfile::tempdir().unwrap();
    let cache = tmp.path().join("cache");
    std::fs::create_dir_all(cache.join("bin")).unwrap();
    std::fs::write(cache.join("bin").join("stale"), b"x").unwrap();

    let mut cmd = Command::cargo_bin("poverty-mode").unwrap();
    cmd.args(["clean", "--clear-cache", "--yes"])
        .env("POVERTY_CACHE_DIR", &cache)
        .env("POVERTY_STATE_DIR", tmp.path().join("state"))
        .assert()
        .success()
        .stdout(contains("will clear cache dir"))
        .stdout(contains("clean complete"));

    // The cache contents were removed and the dir recreated empty.
    assert!(cache.is_dir());
    assert!(!cache.join("bin").exists());
}

#[test]
fn config_path_subcommand_is_not_yet_implemented() {
    let mut cmd = Command::cargo_bin("poverty-mode").unwrap();
    cmd.args(["config", "path"])
        .assert()
        .failure()
        .stderr(contains("not yet implemented: config"));
}
