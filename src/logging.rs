//! Tracing initialization. Logs go to a file (no ANSI) when `--log-file` is
//! given, else to stderr (with ANSI only when stderr is a terminal). The level
//! filter is read from `POVERTY_MODE_LOG` and defaults to `info`.

use std::io::IsTerminal;
use std::path::Path;

use tracing::Subscriber;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::{fmt, EnvFilter};

/// Environment variable used to override the log filter (e.g. `debug`,
/// `poverty_mode=trace`).
const LOG_ENV: &str = "POVERTY_MODE_LOG";

fn env_filter() -> EnvFilter {
    EnvFilter::try_from_env(LOG_ENV).unwrap_or_else(|_| EnvFilter::new("info"))
}

/// Build a `fmt` subscriber writing to `writer`. `ansi` controls colorization.
/// Exposed for headless unit tests (drive it with `tracing::subscriber::with_default`).
pub fn build_subscriber<W>(writer: W, ansi: bool) -> impl Subscriber + Send + Sync
where
    W: for<'w> MakeWriter<'w> + Send + Sync + 'static,
{
    fmt()
        .with_env_filter(env_filter())
        .with_writer(writer)
        .with_ansi(ansi)
        .with_target(true)
        .finish()
}

/// Install the global tracing subscriber. Writes to `log_file` (creating parent
/// directories) when `Some`, otherwise to stderr.
///
/// `set_global_default` is one-shot per process: call this exactly once at
/// startup (and from at most one test). It returns `Err` rather than panicking
/// if the global is already set.
pub fn init_tracing(log_file: Option<&Path>) -> anyhow::Result<()> {
    use tracing::subscriber::set_global_default;

    match log_file {
        Some(path) => {
            if let Some(parent) = path.parent() {
                if !parent.as_os_str().is_empty() {
                    std::fs::create_dir_all(parent)?;
                }
            }
            // A `Fn() -> File` is a `MakeWriter` (blanket impl over `Fn() -> W
            // where W: io::Write`). Opening append per event keeps writes ordered
            // and on disk synchronously — the file test can read immediately
            // after `info!` with no synchronization primitive.
            let path_owned = path.to_path_buf();
            let make = move || {
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&path_owned)
                    .expect("log file should be openable for append")
            };
            let subscriber = build_subscriber(make, false);
            set_global_default(subscriber)?;
        }
        None => {
            let subscriber = build_subscriber(std::io::stderr, std::io::stderr().is_terminal());
            set_global_default(subscriber)?;
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "logging_tests.rs"]
mod logging_tests;
