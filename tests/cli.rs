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
        .stdout(contains("config"))
        .stdout(contains("status"))
        .stdout(contains("doctor"))
        .stdout(contains("clean"));
}

/// Exercises the library crate import path R1 mandates: parse argv with the
/// public `Cli`, then dispatch through the public `dispatch`. FIX-D wired `config`
/// to its real handler, so `config path` now succeeds via this library-import
/// path. It is pure path math (no file is created), so it needs no isolation.
#[test]
fn library_dispatch_is_reachable_for_config_subcommand() {
    use clap::Parser;
    use poverty_mode::cli::{dispatch, Cli};

    let cli = Cli::try_parse_from(["poverty-mode", "config", "path"]).unwrap();
    dispatch(cli, None).expect("`config path` should dispatch successfully via the library import path");
}

// ---- Characterization guards (R12): added AFTER the dispatch stubs exist in
// M1.6. They lock in the stub contract so a later milestone wiring a real
// handler gets an immediate failing test. Not a red->green cycle. ----

/// M10.3 wired `status` to the real handler (R23g): the M3 NotImplemented arm is
/// gone, so the end-to-end `status` invocation now succeeds. Fully hermetic: an
/// isolated `XDG_CONFIG_HOME` carries a Download-mode config (`executable: null`) so
/// status scans the empty `POVERTY_CACHE_DIR` and renders "not installed" without
/// spawning any external binary; `HOME`/`USERPROFILE` are pinned to the temp dir so
/// the probe's `~/.wire/config.json` read finds nothing (no real port, no real
/// `/health` call). An empty `POVERTY_LOG_DIR` yields "no live runs".
///
/// The configured-External path is covered hermetically and cross-platform by
/// `tests/diagnostics.rs::status_reports_configured_external_central`.
#[test]
fn status_subcommand_runs_and_renders() {
    let tmp = tempfile::tempdir().unwrap();
    let config_home = tmp.path().join("config");
    std::fs::create_dir_all(&config_home).unwrap();
    // Download-mode config: `executable: null` => status scans the cache instead of
    // probing an external binary, so the empty cache renders "not installed".
    std::fs::write(
        config_home.join("poverty-mode.yaml"),
        "version: 1\n\
         proxies:\n\
         - name: central\n\
         \x20\x20enabled: false\n\
         \x20\x20settings:\n\
         \x20\x20\x20\x20executable: null\n\
         defaults:\n\
         \x20\x20enable_tool_search: true\n",
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("poverty-mode").unwrap();
    cmd.arg("status")
        .env("XDG_CONFIG_HOME", &config_home)
        .env("POVERTY_CACHE_DIR", tmp.path().join("cache"))
        .env("POVERTY_LOG_DIR", tmp.path().join("logs"))
        // Isolate the probe's `~/.wire/config.json` read from the real home.
        .env("HOME", tmp.path())
        .env("USERPROFILE", tmp.path())
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
        stdout.contains("no problems detected") || stdout.contains("WARN") || stdout.contains("ERROR"),
        "expected doctor diagnostics, got stdout: {stdout:?} stderr: {stderr:?}"
    );
}

/// M10.7 wired `clean` to the real handler (R23g): the M3 NotImplemented arm is
/// gone, so the end-to-end `clean` invocation now builds a real plan. Hermetic via
/// the `POVERTY_CACHE_DIR`/`POVERTY_LOG_DIR` overrides (R23j): an empty log dir
/// (no run dirs) with the default `--keep` and no `--clear-cache`/`--stop-central`
/// yields an empty plan, which short-circuits to "nothing to clean" and exits
/// success WITHOUT prompting -- so this is safe to run non-interactively.
#[test]
fn clean_subcommand_empty_plan_says_nothing_to_clean() {
    let tmp = tempfile::tempdir().unwrap();
    let mut cmd = Command::cargo_bin("poverty-mode").unwrap();
    cmd.arg("clean")
        .env("POVERTY_CACHE_DIR", tmp.path().join("cache"))
        .env("POVERTY_LOG_DIR", tmp.path().join("logs"))
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
        .env("POVERTY_LOG_DIR", tmp.path().join("logs"))
        .assert()
        .success()
        .stdout(contains("will clear cache dir"))
        .stdout(contains("clean complete"));

    // The cache contents were removed and the dir recreated empty.
    assert!(cache.is_dir());
    assert!(!cache.join("bin").exists());
}

/// FIX-D wired `config` to the real handler: `config path` now prints the resolved
/// config-file path and exits success. Hermetic via `XDG_CONFIG_HOME` (set only for
/// the child process), so the printed path points into the isolated config home and
/// no real user file is touched. `config path` creates nothing.
#[test]
fn config_path_subcommand_prints_resolved_path() {
    let tmp = tempfile::tempdir().unwrap();
    let mut cmd = Command::cargo_bin("poverty-mode").unwrap();
    cmd.args(["config", "path"])
        .env("XDG_CONFIG_HOME", tmp.path())
        .assert()
        .success()
        .stdout(contains("poverty-mode.yaml"));
    // The file itself must NOT have been created by `config path` (pure path math).
    assert!(!tmp.path().join("poverty-mode.yaml").exists());
}

/// FIX-D: `config show` loads-or-creates the config, then prints its YAML. On first
/// run it writes the safe all-disabled default and emits it. Hermetic via
/// `XDG_CONFIG_HOME` (child-only env) so it never reads or writes the real config.
#[test]
fn config_show_subcommand_creates_default_and_prints_yaml() {
    let tmp = tempfile::tempdir().unwrap();
    let mut cmd = Command::cargo_bin("poverty-mode").unwrap();
    cmd.args(["config", "show"])
        .env("XDG_CONFIG_HOME", tmp.path())
        .assert()
        .success()
        .stdout(contains("version: 1"))
        .stdout(contains("name: pino"))
        .stdout(contains("name: central"))
        .stdout(contains("enable_tool_search: true"));
    // First run wrote the default config into the isolated home.
    assert!(tmp.path().join("poverty-mode.yaml").exists());
}
