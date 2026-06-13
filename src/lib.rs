//! `poverty-mode` library crate.
//!
//! One multiplexed lib+bin: `main.rs` is a thin shim over [`run`]. Every module
//! is declared here so later milestones extend the crate by FILLING stubs, never
//! by adding `mod` to `main.rs`. Integration tests `use poverty_mode::...`.

pub mod cli;
pub mod error;
pub mod logging;
pub mod proxy;

// `pub use error::{Error, Result};` is added in Task M1.2, once `error.rs`
// defines those items (the M1.1 `error.rs` is a doc-comment-only stub).

/// Binary entry point. Finalized in Task M1.6 to parse the CLI, init tracing,
/// and dispatch. Until then it is a fail-loud stub so the crate compiles.
pub fn run() -> anyhow::Result<()> {
    Err(anyhow::anyhow!("run() not yet wired"))
}
