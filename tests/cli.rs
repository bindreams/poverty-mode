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

#[test]
fn doctor_subcommand_is_not_yet_implemented() {
    let mut cmd = Command::cargo_bin("poverty-mode").unwrap();
    cmd.arg("doctor")
        .assert()
        .failure()
        .stderr(contains("not yet implemented: doctor"));
}

#[test]
fn clean_subcommand_is_not_yet_implemented() {
    let mut cmd = Command::cargo_bin("poverty-mode").unwrap();
    cmd.arg("clean")
        .assert()
        .failure()
        .stderr(contains("not yet implemented: clean"));
}

#[test]
fn config_path_subcommand_is_not_yet_implemented() {
    let mut cmd = Command::cargo_bin("poverty-mode").unwrap();
    cmd.args(["config", "path"])
        .assert()
        .failure()
        .stderr(contains("not yet implemented: config"));
}
