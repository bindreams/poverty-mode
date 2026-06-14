//! Clap CLI definitions and subcommand dispatch.
//!
//! Per R23b the `proxy` command is the TUPLE variant `Command::Proxy(ProxyArgs)`;
//! `ProxyArgs` carries a positional `which: ProxyName` plus three flattened
//! argument groups (`common`, `pino`, `headroom`). The dispatcher selects the
//! relevant group from `which`. M3 FILLS the `proxy` handler (build
//! [`EngineConfig`] from the flags, then run the async engine); the remaining
//! handlers are `NotImplemented` stubs that later milestones FILL. The proxy
//! identity flag is `--run-id` (a per-run ULID shared by all hops, R10).

use std::net::SocketAddr;
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use crate::error::Error;
use crate::proxy::headroom::HeadroomSettings;
use crate::proxy::pino::{PinoSettings, TailTtl};
use crate::proxy::{self, EngineConfig, ProxyName, TransformKind, Upstream};

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
// R23b mandates the unboxed tuple variant `Command::Proxy(ProxyArgs)` as the canonical
// cross-milestone shape (M3/M6 match `Command::Proxy(args)`); boxing would diverge from it.
#[allow(clippy::large_enum_variant)]
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

impl ProxyArgs {
    /// Resolved `auto_cache`: default false; `--auto-cache` on, `--no-auto-cache`
    /// off (clap `overrides_with` makes the last flag win).
    pub fn auto_cache(&self) -> bool {
        self.pino.auto_cache && !self.pino.no_auto_cache
    }

    /// Resolved `strip_ansi`: default TRUE; `--no-strip-ansi` turns it off.
    pub fn strip_ansi(&self) -> bool {
        !self.pino.no_strip_ansi
    }

    /// Resolved `compression`: default false; `--compression` on, `--no-compression` off.
    pub fn compression(&self) -> bool {
        self.headroom.compression && !self.headroom.no_compression
    }
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
///
/// `auto_cache` / `strip_ansi` are presence flags with `--no-*` companions
/// (MINOR finding): each pair carries the positive flag plus its negation, and
/// the resolved value is read via [`ProxyArgs::auto_cache`] / [`ProxyArgs::strip_ansi`]
/// (clap `overrides_with` makes the last-specified flag win).
#[derive(Args, Debug)]
pub struct PinoArgs {
    /// Inject prompt-cache breakpoints. Presence flag (default off).
    #[arg(long, overrides_with = "no_auto_cache")]
    pub auto_cache: bool,
    /// Disable cache-breakpoint injection (negates `--auto-cache`).
    #[arg(long = "no-auto-cache", overrides_with = "auto_cache")]
    pub no_auto_cache: bool,

    /// Rolling-tail cache TTL (`5m` default, or `1h`).
    #[arg(long, value_name = "TTL", value_enum, default_value_t = TailTtlArg::FiveMin)]
    pub tail_ttl: TailTtlArg,

    /// Tool names to drop from `tools` and scrub from reminders.
    #[arg(long, value_delimiter = ',', value_name = "CSV")]
    pub drop_tools: Vec<String>,

    /// Strip ANSI escape sequences from text content. Presence flag (default on);
    /// disable with `--no-strip-ansi`.
    #[arg(long = "strip-ansi", overrides_with = "no_strip_ansi")]
    pub strip_ansi: bool,
    /// Disable ANSI stripping (negates the default-on behavior).
    #[arg(long = "no-strip-ansi", overrides_with = "strip_ansi")]
    pub no_strip_ansi: bool,

    /// Override the requested model identifier.
    #[arg(long, value_name = "MODEL")]
    pub model_override: Option<String>,
}

/// headroom-only flag (flattened onto [`ProxyArgs`]; read by the dispatcher when
/// `which == ProxyName::Headroom`).
///
/// `compression` is a presence flag with a `--no-compression` companion (MINOR
/// finding); the resolved value is read via [`ProxyArgs::compression`].
#[derive(Args, Debug)]
pub struct HeadroomArgs {
    /// Enable context compression. Presence flag (default off).
    #[arg(long, overrides_with = "no_compression")]
    pub compression: bool,
    /// Disable compression (negates `--compression`; explicit off is the default).
    #[arg(long = "no-compression", overrides_with = "compression")]
    pub no_compression: bool,
}

/// `--tail-ttl` value enum mapping to [`TailTtl`] (`5m` / `1h`).
#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum TailTtlArg {
    /// 5-minute rolling tail (default).
    #[value(name = "5m")]
    FiveMin,
    /// 1-hour rolling tail.
    #[value(name = "1h")]
    OneHour,
}

impl From<TailTtlArg> for TailTtl {
    fn from(a: TailTtlArg) -> Self {
        match a {
            TailTtlArg::FiveMin => TailTtl::FiveMin,
            TailTtlArg::OneHour => TailTtl::OneHour,
        }
    }
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

/// Build the [`TransformKind`] for the chosen proxy from its (resolved) CLI
/// flags. The presence/`--no-*` pairs are folded by the [`ProxyArgs`] accessors;
/// empty `--drop-tools` entries (e.g. trailing comma) are dropped.
pub fn transform_from_proxy_args(args: &ProxyArgs) -> TransformKind {
    match args.which {
        ProxyName::Pino => TransformKind::Pino(PinoSettings {
            auto_cache: args.auto_cache(),
            tail_ttl: args.pino.tail_ttl.into(),
            drop_tools: args
                .pino
                .drop_tools
                .iter()
                .filter(|s| !s.is_empty())
                .cloned()
                .collect(),
            strip_ansi: args.strip_ansi(),
            model_override: args.pino.model_override.clone(),
        }),
        ProxyName::Headroom => TransformKind::Headroom(HeadroomSettings {
            compression: args.compression(),
        }),
        // `which` is parsed by `parse_first_party_proxy`, which only accepts
        // `pino`/`headroom`; `central` can never reach here.
        ProxyName::Central => unreachable!("central is not a first-party proxy"),
    }
}

/// Build the full [`EngineConfig`] for a `proxy` invocation from its parsed args.
pub fn engine_config_from_proxy_args(args: &ProxyArgs) -> EngineConfig {
    EngineConfig {
        name: args.which,
        listen: args.common.listen,
        upstream: Upstream {
            url: args.common.upstream.clone(),
        },
        run_id: args.common.run_id.clone(),
        log_file: args.common.body_log_file.clone(),
        transform: transform_from_proxy_args(args),
    }
}

/// Dispatch a parsed CLI to the matching subcommand handler. The `proxy` arm is
/// FILLED here (M3): it builds the [`EngineConfig`] and runs the async engine to
/// completion on a fresh multi-thread runtime. The remaining handlers are
/// `NotImplemented` stubs that later milestones FILL.
pub fn dispatch(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        Command::Run { .. } => Err(Error::NotImplemented("run").into()),
        Command::Proxy(args) => {
            // DEFERRED TO M6 (R22 parent-death wiring): M6 inserts
            // `crate::orchestrator::teardown::spawn_death_watcher_from_env();` as
            // the FIRST statement of this arm so the macOS death-pipe backstop
            // fires (the child terminates on parent death via read-end EOF, R23h).
            // It cannot be added in M3 because `teardown` is authored in M6; M3
            // surfaces the deferral here rather than silently omitting it.
            let cfg = engine_config_from_proxy_args(&args);
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            rt.block_on(proxy::run_proxy(cfg))
        }
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
