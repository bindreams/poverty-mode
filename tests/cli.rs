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
