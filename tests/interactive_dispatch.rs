//! M9.12 reaching tests for `run --interactive` wiring.
//!
//! Two automatable concerns guard the M9.11 wiring:
//! 1. `--interactive` dispatches into `tui::run_picker` instead of the old M6
//!    `anyhow::bail!("--interactive requires the TUI (milestone M9)")` placeholder.
//! 2. The non-TTY guard fails loudly with a typed [`TuiError::NotATerminal`] when
//!    stdio is not a terminal, rather than hanging on `event::read`.
//!
//! Both are observed via the binary: `assert_cmd` spawns the child with piped
//! (non-TTY) stdin/stdout, so the guard fires deterministically and the process
//! exits with the terminal-required message — proving the request reached
//! `run_picker` and not the removed bail.
//!
//! These are deliberately binary-level (not in-process) checks. `run_picker`'s
//! guard queries the real OS file descriptors via `IsTerminal`; libtest's output
//! capture only redirects the Rust-level `Stdout` writer, NOT fds 0/1, so an
//! in-process `run_picker` call would see a live TTY under an interactive
//! `cargo test`, skip the guard, and hang on `event::read`. The piped assert_cmd
//! child is non-TTY regardless of the parent terminal, so it is the portable way
//! to exercise the `NotATerminal` path.

use assert_cmd::Command;
use predicates::prelude::PredicateBooleanExt;
use predicates::str::contains;

/// `run --interactive` under non-TTY stdio must fail with the picker's
/// not-a-terminal error, proving it dispatched into `tui::run_picker` rather than
/// the removed M6 `--interactive requires the TUI (milestone M9)` bail.
#[test]
fn interactive_run_reaches_picker_and_hits_non_tty_guard() {
    let cfg_home = tempfile::tempdir().unwrap();

    let mut cmd = Command::cargo_bin("poverty-mode").unwrap();
    cmd.env("XDG_CONFIG_HOME", cfg_home.path())
        .env_remove("POVERTY_PROXY_CHAIN")
        .env_remove("ANTHROPIC_BASE_URL")
        .arg("run")
        .arg("--interactive")
        .args(["--", "true"]);

    cmd.assert()
        .failure()
        .stderr(contains(
            "interactive picker requires a terminal (stdin/stdout is not a TTY)",
        ))
        // The old placeholder must be gone: a milestone-M9 bail means the request
        // never reached run_picker.
        .stderr(contains("milestone M9").not());
}

/// `--interactive` must NOT silently drop `--proxies` (spec §5.10: the TUI is
/// seeded from the RESOLVED chain; spec line 79 puts `--proxies`/`--interactive`
/// at the same precedence tier). A bogus `--proxies` value is resolved BEFORE the
/// picker, so it fails with an "unknown proxy" error — not the non-TTY guard. The
/// silent-drop defect would instead ignore `--proxies` entirely and reach the
/// picker, surfacing the terminal error.
#[test]
fn interactive_resolves_proxies_before_picker_so_bogus_proxies_errors() {
    let cfg_home = tempfile::tempdir().unwrap();

    let mut cmd = Command::cargo_bin("poverty-mode").unwrap();
    cmd.env("XDG_CONFIG_HOME", cfg_home.path())
        .env_remove("POVERTY_PROXY_CHAIN")
        .env_remove("ANTHROPIC_BASE_URL")
        .arg("run")
        .arg("--interactive")
        .args(["--proxies", "bogus"])
        .args(["--", "true"]);

    cmd.assert()
        .failure()
        .stderr(contains("unknown proxy name"))
        // `--proxies` must be consumed by resolution, NOT silently dropped and the
        // picker reached (which would surface the non-TTY guard instead).
        .stderr(contains("is not a TTY").not());
}

/// Likewise, `POVERTY_PROXY_CHAIN` must feed the interactive picker's seed. A
/// bogus env chain is resolved before the picker and errors, proving the env is
/// not silently dropped under `--interactive`.
#[test]
fn interactive_resolves_env_chain_before_picker_so_bogus_env_errors() {
    let cfg_home = tempfile::tempdir().unwrap();

    let mut cmd = Command::cargo_bin("poverty-mode").unwrap();
    cmd.env("XDG_CONFIG_HOME", cfg_home.path())
        .env("POVERTY_PROXY_CHAIN", "bogus")
        .env_remove("ANTHROPIC_BASE_URL")
        .arg("run")
        .arg("--interactive")
        .args(["--", "true"]);

    cmd.assert()
        .failure()
        .stderr(contains("unknown proxy name"))
        .stderr(contains("is not a TTY").not());
}
