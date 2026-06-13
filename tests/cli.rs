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
/// public `Cli`, then dispatch through the public `dispatch`.
#[test]
fn library_dispatch_is_reachable_for_stub_subcommand() {
    use clap::Parser;
    use poverty_mode::cli::{dispatch, Cli};

    let cli = Cli::try_parse_from(["poverty-mode", "status"]).unwrap();
    let err = dispatch(cli).unwrap_err();
    assert!(
        err.to_string().contains("not yet implemented: status"),
        "library dispatch should return the stub error, got: {err}"
    );
}

// ---- Characterization guards (R12): added AFTER the dispatch stubs exist in
// M1.6. They lock in the stub contract so a later milestone wiring a real
// handler gets an immediate failing test. Not a red->green cycle. ----

#[test]
fn status_subcommand_is_not_yet_implemented() {
    let mut cmd = Command::cargo_bin("poverty-mode").unwrap();
    cmd.arg("status")
        .assert()
        .failure()
        .stderr(contains("not yet implemented: status"));
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
