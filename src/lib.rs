//! `poverty-mode` library crate.
//!
//! One multiplexed lib+bin: `main.rs` is a thin shim over [`run`]. Every module
//! is declared here so later milestones extend the crate by FILLING stubs, never
//! by adding `mod` to `main.rs`. Integration tests `use poverty_mode::...`.

pub mod cli;
pub mod error;
pub mod logging;
pub mod paths;
pub mod proxy;

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
    logging::init_tracing(cli.log_file.as_deref())?;
    cli::dispatch(cli)
}
