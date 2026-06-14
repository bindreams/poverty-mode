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

    /// Hidden: spawn one grouped sleeper, print its pid + READY, then park. Used
    /// by teardown tests to prove the OS reaps the child when the holder dies.
    #[command(name = "__spawn-holder", hide = true)]
    SpawnHolder,

    /// Hidden: a long sleeper used as the grouped child in teardown tests.
    #[command(name = "__sleep", hide = true)]
    Sleep,

    /// Hidden: POST an empty JSON body with a test api-key header to <url>, exit
    /// 0 on 2xx. Used as a deterministic in-repo "agent" in chain tests.
    #[command(name = "__post", hide = true)]
    Post { url: String },

    /// Hidden: write STARTED to <marker>, install a SIGTERM handler that appends
    /// SIGTERM and exits 42, then sleep. Used by signal-forwarding tests.
    #[command(name = "__sigwait", hide = true)]
    SigWait { marker: String },
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
            // Test-only fail-closed shim: a child whose OWN env has
            // PM_TEST_FAIL_PROXY=1 exits 1 before binding, so the orchestrator's
            // readiness path is deterministically testable. The orchestrator sets
            // this ONLY via the spawned child's Command::env (never the parent's
            // global env), so it is inert in production.
            if std::env::var("PM_TEST_FAIL_PROXY").as_deref() == Ok("1") {
                std::process::exit(1);
            }
            // Parent-death backstop (R22/R23h): macOS death-pipe EOF watcher (Linux:
            // redundant with PR_SET_PDEATHSIG armed in `pre_exec`; Windows: no-op).
            // It is a no-op unless this child was spawned into a `ProxyGroup` (i.e.
            // `PM_DEATH_PIPE_FD` is set), so standalone `proxy` debugging is unaffected.
            crate::orchestrator::teardown::spawn_death_watcher_from_env();
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
        Command::SpawnHolder => {
            use crate::orchestrator::teardown::ProxyGroup;
            let exe = std::env::current_exe()?;
            // `ProxyGroup::spawn` uses `tokio::process::Command` on Unix, whose
            // child construction requires an active Tokio runtime context (it
            // registers a pidfd/SIGCHLD reactor and panics if there is none).
            // `dispatch` is synchronous, so build a runtime and do the spawn +
            // park INSIDE `block_on`. The runtime is never dropped (we park
            // forever inside it), and the group is `mem::forget`-ten — so neither
            // the runtime nor `Drop`/`kill_all` ever reaps the child. The OS must
            // reap it purely because THIS holder dies (Unix: death-pipe write end
            // close + PR_SET_PDEATHSIG; Windows: job handle close).
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            rt.block_on(async {
                let mut group = ProxyGroup::new()?;
                let spawned = group.spawn(&exe, &["__sleep".to_string()], &[])?;
                println!("{}", spawned.pid);
                println!("HOLDER_READY");
                use std::io::Write as _;
                std::io::stdout().flush().ok();
                std::mem::forget(group);
                loop {
                    std::thread::park();
                }
                #[allow(unreachable_code)]
                anyhow::Ok(())
            })
        }
        Command::Sleep => {
            crate::orchestrator::teardown::spawn_death_watcher_from_env();
            std::thread::sleep(std::time::Duration::from_secs(3600));
            Ok(())
        }
        Command::Post { url } => {
            // `reqwest::blocking` MUST NOT run on a runtime thread (R5), so the
            // send happens on a blocking-pool thread via `spawn_blocking`. The
            // surrounding runtime exists only to drive that join.
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            rt.block_on(async move {
                let resp = tokio::task::spawn_blocking(move || {
                    reqwest::blocking::Client::builder()
                        .redirect(reqwest::redirect::Policy::none())
                        .build()
                        .and_then(|c| {
                            c.post(&url)
                                .header("content-type", "application/json")
                                .header("x-api-key", "sk-test")
                                .body(r#"{"model":"claude-x","messages":[]}"#)
                                .send()
                        })
                })
                .await
                .map_err(|e| anyhow::anyhow!("post task join: {e}"))?;
                match resp {
                    Ok(r) if r.status().is_success() => Ok(()),
                    Ok(r) => anyhow::bail!("__post got HTTP {}", r.status()),
                    Err(e) => anyhow::bail!("__post failed: {e}"),
                }
            })
        }
        Command::SigWait { marker } => {
            use std::io::Write as _;
            {
                let mut f = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&marker)?;
                writeln!(f, "STARTED")?;
            }
            #[cfg(unix)]
            {
                // Install a SIGTERM handler (sync; std-only via signal-hook is not
                // a dep, so use a raw sigaction through libc).
                static MARKER: std::sync::OnceLock<String> = std::sync::OnceLock::new();
                let _ = MARKER.set(marker.clone());
                extern "C" fn on_term(_sig: libc::c_int) {
                    if let Some(m) = MARKER.get() {
                        if let Ok(mut f) = std::fs::OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(m)
                        {
                            use std::io::Write as _;
                            let _ = writeln!(f, "SIGTERM");
                        }
                    }
                    // Exit 42 from the handler (async-signal-unsafe writes above are
                    // acceptable in this TEST helper only).
                    unsafe { libc::_exit(42) };
                }
                unsafe {
                    let mut sa: libc::sigaction = std::mem::zeroed();
                    sa.sa_sigaction = on_term as usize;
                    libc::sigemptyset(&mut sa.sa_mask);
                    libc::sigaction(libc::SIGTERM, &sa, std::ptr::null_mut());
                }
            }
            std::thread::sleep(std::time::Duration::from_secs(3600));
            Ok(())
        }
    }
}

#[cfg(test)]
#[path = "cli_tests.rs"]
mod cli_tests;
