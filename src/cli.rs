//! Clap CLI definitions and subcommand dispatch.
//!
//! Per R23b the `proxy` command is the TUPLE variant `Command::Proxy(ProxyArgs)`;
//! `ProxyArgs` carries a positional `which: ProxyName` plus three flattened
//! argument groups (`common`, `pino`, `headroom`). The dispatcher selects the
//! relevant group from `which`. Each handler is a `NotImplemented` stub; later
//! milestones FILL them. The proxy identity flag is `--run-id` (a per-run ULID
//! shared by all hops, R10).

use std::net::SocketAddr;
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use crate::error::Error;
use crate::proxy::ProxyName;

/// Run an AI coding agent behind a user-chosen chain of local HTTP proxies.
#[derive(Parser, Debug)]
#[command(name = "poverty-mode", version, about, long_about = None)]
pub struct Cli {
    /// Write logs to this file instead of stderr.
    #[arg(long, global = true, value_name = "PATH")]
    pub log_file: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Run an agent behind the resolved proxy chain.
    Run {
        /// Comma-separated ordered chain, e.g. `pino,headroom,central`.
        #[arg(long, value_delimiter = ',', value_name = "CSV")]
        proxies: Option<Vec<String>>,

        /// Pick the chain interactively in a TUI.
        #[arg(long)]
        interactive: bool,

        /// Persist the resolved chain back to the config file.
        #[arg(long, overrides_with = "no_save")]
        save: bool,

        /// Do not persist the resolved chain (the default).
        #[arg(long = "no-save", overrides_with = "save")]
        no_save: bool,

        /// The agent and its arguments, after `--`.
        #[arg(last = true, value_name = "AGENT_ARGV")]
        agent_argv: Vec<String>,
    },

    /// Run a single first-party proxy in the foreground.
    ///
    /// R23b: a TUPLE variant carrying `ProxyArgs` (M3/M6 match `Command::Proxy(args)`
    /// and read `args.which`).
    Proxy(ProxyArgs),

    /// Manage the JB Central singleton.
    Central {
        #[command(subcommand)]
        action: CentralAction,
    },

    /// Inspect or edit the config file.
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },

    /// Summarize installed components and live runs.
    Status,

    /// Diagnose environment and settings conflicts.
    Doctor,

    /// Stop singletons and prune run dirs and caches.
    Clean,
}

/// Arguments for the `proxy` subcommand (R23b). The positional `which` selects
/// which first-party proxy to run; the three flattened groups carry the common
/// options plus each proxy's own flags. The dispatcher (M3/M6) reads `which` and
/// the matching group.
#[derive(Args, Debug)]
pub struct ProxyArgs {
    /// Which first-party proxy to run (`pino` or `headroom`).
    #[arg(value_name = "PROXY", value_parser = parse_first_party_proxy)]
    pub which: ProxyName,

    #[command(flatten)]
    pub common: CommonProxyArgs,

    #[command(flatten)]
    pub pino: PinoArgs,

    #[command(flatten)]
    pub headroom: HeadroomArgs,
}

/// Restrict the `which` positional to the first-party proxies (`pino`/`headroom`).
/// `central` is a downloaded singleton, never run via `proxy`, so it (and any
/// unknown name) is rejected with a clap `InvalidValue` error.
fn parse_first_party_proxy(s: &str) -> Result<ProxyName, String> {
    match s {
        "pino" => Ok(ProxyName::Pino),
        "headroom" => Ok(ProxyName::Headroom),
        other => Err(format!(
            "invalid proxy {other:?} (expected one of: pino, headroom)"
        )),
    }
}

/// Flags common to every first-party `proxy` invocation.
#[derive(Args, Debug)]
pub struct CommonProxyArgs {
    /// Bind address; use `HOST:0` for an OS-assigned port.
    #[arg(long, value_name = "HOST:PORT")]
    pub listen: SocketAddr,

    /// Upstream URL (scheme, host, optional port and path prefix).
    #[arg(long, value_name = "URL")]
    pub upstream: url::Url,

    /// Per-run ULID, shared by all hops of one run; reported by `/__pm/health`.
    #[arg(long, value_name = "ULID")]
    pub run_id: String,

    /// Tee request/response bodies to this log file. Distinct from the global
    /// `--log-file` (tracing destination): this is the per-proxy body-tee sink
    /// that M3 wires into `EngineConfig.log_file` (R10). The field name (hence
    /// the clap arg id) is `body_log_file`, not `log_file`, so it does not merge
    /// with the global `Cli::log_file` arg when both appear on a `proxy` call.
    #[arg(long, value_name = "PATH")]
    pub body_log_file: Option<PathBuf>,
}

/// pino-only transform flags (flattened onto [`ProxyArgs`]; read by the
/// dispatcher when `which == ProxyName::Pino`).
#[derive(Args, Debug)]
pub struct PinoArgs {
    /// Enable cache-breakpoint injection.
    #[arg(long)]
    pub auto_cache: bool,

    /// Rolling-tail cache TTL.
    #[arg(long, value_name = "TTL", value_parser = ["5m", "1h"])]
    pub tail_ttl: Option<String>,

    /// Tool names to drop from `tools` and scrub from reminders.
    #[arg(long, value_delimiter = ',', value_name = "CSV")]
    pub drop_tools: Option<Vec<String>>,

    /// Strip ANSI escape sequences from text content.
    #[arg(long, value_name = "BOOL")]
    pub strip_ansi: Option<bool>,

    /// Override the requested model identifier.
    #[arg(long, value_name = "MODEL")]
    pub model_override: Option<String>,
}

/// headroom-only flag (flattened onto [`ProxyArgs`]; read by the dispatcher when
/// `which == ProxyName::Headroom`).
#[derive(Args, Debug)]
pub struct HeadroomArgs {
    /// Enable context compression (headroom).
    #[arg(long, value_name = "BOOL")]
    pub compression: Option<bool>,
}

#[derive(Subcommand, Debug)]
pub enum CentralAction {
    /// Run the interactive JetBrains login flow.
    Login,
    /// Report central install and login status.
    Status,
    /// Stop the central singleton daemon.
    Stop,
}

#[derive(Subcommand, Debug)]
pub enum ConfigAction {
    /// Print the effective config.
    Show,
    /// Open the config file in $EDITOR.
    Edit,
    /// Print the config file path.
    Path,
}

/// Dispatch a parsed CLI to the matching subcommand handler. Every handler is a
/// `NotImplemented` stub; later milestones FILL them.
pub fn dispatch(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        Command::Run { .. } => Err(Error::NotImplemented("run").into()),
        Command::Proxy(_) => Err(Error::NotImplemented("proxy").into()),
        Command::Central { .. } => Err(Error::NotImplemented("central").into()),
        Command::Config { .. } => Err(Error::NotImplemented("config").into()),
        Command::Status => Err(Error::NotImplemented("status").into()),
        Command::Doctor => Err(Error::NotImplemented("doctor").into()),
        Command::Clean => Err(Error::NotImplemented("clean").into()),
    }
}

#[cfg(test)]
#[path = "cli_tests.rs"]
mod cli_tests;
