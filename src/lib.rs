//! `poverty-mode` library crate.
//!
//! One multiplexed lib+bin: `main.rs` is a thin shim over [`run`]. Every module
//! is declared here so later milestones extend the crate by FILLING stubs, never
//! by adding `mod` to `main.rs`. Integration tests `use poverty_mode::...`.

pub mod agent;
pub mod central;
pub mod clean;
pub mod cli;
pub mod config;
pub mod doctor;
pub mod download;
pub mod error;
pub mod logging;
pub mod orchestrator;
pub mod paths;
pub mod proxy;
pub mod status;
pub mod tui;

#[cfg(test)]
#[path = "test_support.rs"]
pub(crate) mod test_support;

pub use error::{Error, Result};

/// Binary entry point. Parses the CLI, initializes tracing from the global
/// `--log-file`, then dispatches the chosen subcommand. `main.rs` calls only
/// this (R1: never `mod X` in `main.rs`).
pub fn run() -> anyhow::Result<()> {
    use clap::Parser;
    let cli = cli::Cli::parse();

    // For `run`, every byte of the session's logs lands in one findable dir under
    // the user log dir; the parent's own tracing goes to `<dir>/main.log` unless an
    // explicit `--log-file` overrides it. The dir is created BEFORE `init_tracing`
    // so the global subscriber never targets the terminal the agent will own.
    let run_id = matches!(cli.command, cli::Command::Run { .. }).then(paths::new_session_name);
    let session_dir = match &run_id {
        Some(id) => Some(paths::ensure_run_dir(id)?),
        None => None,
    };
    let log_file = cli
        .log_file
        .clone()
        .or_else(|| session_dir.as_ref().map(|d| d.join("main.log")));
    logging::init_tracing(log_file.as_deref())?;

    // Emit an info event so that the log file is created on the first startup, even
    // if no subsequent events reach it in a short-lived run (the MakeWriter opens the
    // file lazily on first write).
    if let Some(ref dir) = session_dir {
        tracing::info!(session_dir = %dir.display(), "poverty-mode run started");
    }

    cli::dispatch(cli, run_id)
}
