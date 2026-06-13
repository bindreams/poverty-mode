//! Crate error type. Library-internal call sites generally use `anyhow::Result`;
//! this enum is the structured error for the few cases that need a stable
//! variant — notably the not-yet-implemented subcommand stubs.

/// The crate's structured error.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A subcommand whose real implementation lands in a later milestone.
    #[error("not yet implemented: {0}")]
    NotImplemented(&'static str),

    /// Any other failure, carried transparently from `anyhow`.
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// Convenience alias for fallible crate functions that return [`Error`].
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
#[path = "error_tests.rs"]
mod error_tests;
