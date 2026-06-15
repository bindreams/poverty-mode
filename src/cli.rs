//! Clap CLI definitions and subcommand dispatch.
//!
//! Per R23b the `proxy` command is the TUPLE variant `Command::Proxy(ProxyArgs)`;
//! `ProxyArgs` carries a positional `which: ProxyName` plus three flattened
//! argument groups (`common`, `pino`, `headroom`). The dispatcher selects the
//! relevant group from `which`. The `proxy` handler builds the [`EngineConfig`]
//! from the flags and runs the async engine; every other subcommand is wired to
//! its real handler. The proxy identity flag is `--run-id` (a per-run ULID shared
//! by all hops, R10).

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::Context as _;
use clap::{Args, Parser, Subcommand};

use crate::proxy::headroom::HeadroomSettings;
use crate::proxy::pino::{CacheTtl, PinoSettings};
use crate::proxy::{self, EngineConfig, ProxyName, TransformKind, Upstream};

/// Run an AI coding agent behind a user-chosen chain of local HTTP proxies.
#[derive(Parser, Debug)]
#[command(name = "poverty-mode", version, about, long_about = None)]
pub struct Cli {
    /// Write logs to this file instead of stderr.
    #[arg(long, global = true, value_name = "PATH")]
    pub log_file: Option<PathBuf>,

    /// Hidden, accepted-and-ignored: mirrors Claude Code's `--settings <json>`
    /// flag so the in-repo hidden helper subcommands (`__post`, `__printenv`,
    /// ...) — used as stand-in agents in chain/run tests — tolerate the belt-2
    /// `--settings` argument the orchestrator injects between the agent program
    /// and its args (M7.2), exactly as the real `claude` binary would consume it.
    /// poverty-mode itself never reads this value.
    #[arg(long, global = true, hide = true, value_name = "JSON")]
    pub settings: Option<String>,

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

        #[command(flatten)]
        settings: RunSettingsArgs,

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

    /// Prune old run dirs, clear caches, and optionally stop the shared central singleton.
    Clean {
        /// Number of newest run directories to keep.
        #[arg(long, default_value_t = crate::clean::DEFAULT_KEEP_RUNS)]
        keep: usize,
        /// Also clear the downloaded-binary cache.
        #[arg(long)]
        clear_cache: bool,
        /// Stop the shared central singleton (disrupts other live sessions; off by default).
        #[arg(long)]
        stop_central: bool,
        /// Skip the interactive confirmation prompt.
        #[arg(long, short = 'y')]
        yes: bool,
    },

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

    /// Hidden: print the value of env var <name> to stdout and exit 0. Used as a
    /// deterministic in-repo "agent" in run-precedence tests.
    #[command(name = "__printenv", hide = true)]
    PrintEnv { name: String },
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

    /// Resolved `compression`: default TRUE; `--no-compression` turns it off.
    pub fn compression(&self) -> bool {
        !self.headroom.no_compression
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

    /// Cache TTL for main-agent requests (`1h` default, or `5m`).
    #[arg(long, value_name = "TTL", value_enum, default_value_t = CacheTtlArg::OneHour)]
    pub main_ttl: CacheTtlArg,

    /// Cache TTL for subagent requests (`5m` default, or `1h`).
    #[arg(long, value_name = "TTL", value_enum, default_value_t = CacheTtlArg::FiveMin)]
    pub sub_ttl: CacheTtlArg,

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
    /// Enable context compression. Presence flag (redundant with the default on).
    #[arg(long, overrides_with = "no_compression")]
    pub compression: bool,
    /// Disable compression (negates `--compression`; compression is on by default).
    #[arg(long = "no-compression", overrides_with = "compression")]
    pub no_compression: bool,
}

/// `--main-ttl` / `--sub-ttl` value enum mapping to [`CacheTtl`] (`5m` / `1h`).
#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheTtlArg {
    /// 5-minute cache TTL.
    #[value(name = "5m")]
    FiveMin,
    /// 1-hour cache TTL.
    #[value(name = "1h")]
    OneHour,
}

impl From<CacheTtlArg> for CacheTtl {
    fn from(a: CacheTtlArg) -> Self {
        match a {
            CacheTtlArg::FiveMin => CacheTtl::FiveMin,
            CacheTtlArg::OneHour => CacheTtl::OneHour,
        }
    }
}

/// Per-proxy setting overrides for `run`, namespaced by proxy id. Each flag is
/// optional; absent ⇒ no override for that field. Booleans are `--x` / `--no-x`
/// pairs folded to `Option<bool>` (None when neither is given). The resolved
/// [`Overrides`](crate::config::overrides::Overrides) is merged onto the loaded
/// config before chain resolution / picker seeding / save.
#[derive(Args, Debug, Default)]
pub struct RunSettingsArgs {
    #[arg(long = "pino-auto-cache", overrides_with = "pino_no_auto_cache")]
    pub pino_auto_cache: bool,
    #[arg(long = "pino-no-auto-cache", overrides_with = "pino_auto_cache")]
    pub pino_no_auto_cache: bool,
    #[arg(long = "pino-main-ttl", value_name = "TTL", value_enum)]
    pub pino_main_ttl: Option<CacheTtlArg>,
    #[arg(long = "pino-sub-ttl", value_name = "TTL", value_enum)]
    pub pino_sub_ttl: Option<CacheTtlArg>,
    #[arg(long = "pino-drop-tools", value_delimiter = ',', value_name = "CSV")]
    pub pino_drop_tools: Option<Vec<String>>,
    #[arg(long = "pino-strip-ansi", overrides_with = "pino_no_strip_ansi")]
    pub pino_strip_ansi: bool,
    #[arg(long = "pino-no-strip-ansi", overrides_with = "pino_strip_ansi")]
    pub pino_no_strip_ansi: bool,
    #[arg(long = "pino-model-override", value_name = "MODEL")]
    pub pino_model_override: Option<String>,
    #[arg(
        long = "headroom-compression",
        overrides_with = "headroom_no_compression"
    )]
    pub headroom_compression: bool,
    #[arg(
        long = "headroom-no-compression",
        overrides_with = "headroom_compression"
    )]
    pub headroom_no_compression: bool,
    #[arg(long = "central-port", value_name = "PORT")]
    pub central_port: Option<u16>,
    #[arg(long = "central-pinned-version", value_name = "VERSION")]
    pub central_pinned_version: Option<String>,
}

impl RunSettingsArgs {
    /// Fold a `--x` / `--no-x` presence-flag pair into `Option<bool>`: `Some(true)`
    /// if the positive flag won, `Some(false)` if the negation did, `None` if
    /// neither was given (clap `overrides_with` makes the last-specified flag win).
    fn tri(pos: bool, neg: bool) -> Option<bool> {
        if pos {
            Some(true)
        } else if neg {
            Some(false)
        } else {
            None
        }
    }

    /// Project the parsed flags into the partial [`Overrides`]. Absent flags become
    /// `None`; `--pino-drop-tools` filters empty entries (a bare empty value is an
    /// explicit clear ⇒ `Some(vec![])`).
    pub fn to_overrides(&self) -> crate::config::overrides::Overrides {
        use crate::config::overrides::{
            CentralOverride, HeadroomOverride, Overrides, PinoOverride,
        };
        Overrides {
            pino: PinoOverride {
                auto_cache: Self::tri(self.pino_auto_cache, self.pino_no_auto_cache),
                main_ttl: self.pino_main_ttl.map(Into::into),
                sub_ttl: self.pino_sub_ttl.map(Into::into),
                drop_tools: self
                    .pino_drop_tools
                    .as_ref()
                    .map(|v| v.iter().filter(|s| !s.is_empty()).cloned().collect()),
                strip_ansi: Self::tri(self.pino_strip_ansi, self.pino_no_strip_ansi),
                model_override: self.pino_model_override.clone(),
            },
            headroom: HeadroomOverride {
                compression: Self::tri(self.headroom_compression, self.headroom_no_compression),
            },
            central: CentralOverride {
                port: self.central_port,
                pinned_version: self.central_pinned_version.clone(),
            },
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
            main_ttl: args.pino.main_ttl.into(),
            sub_ttl: args.pino.sub_ttl.into(),
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
/// completion on a fresh multi-thread runtime. Every other handler is wired to its
/// real implementation; `central`/`config` delegate to [`dispatch_central`] /
/// [`dispatch_config`].
pub fn dispatch(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        Command::Run {
            proxies,
            interactive,
            save,
            no_save: _no_save,
            settings,
            agent_argv,
        } => {
            // Load the base config, then merge the `run` per-proxy setting flags
            // onto it. The overridden config is the single base for chain
            // resolution, picker seeding, and `--save`, so a `--<proxy>-<setting>`
            // flag is honored on every path (spec §override precedence).
            let config = crate::config::Config::load_or_create()?;
            let overrides = settings.to_overrides();
            let config = config.with_overrides(&overrides);

            // Resolve the chain from CLI > env > file (M2 precedence) FIRST, so
            // `--proxies` / `POVERTY_PROXY_CHAIN` are honored on EVERY path,
            // including `--interactive` (spec line 79: they share the highest
            // precedence tier). A bad name here is a hard error, never silently
            // dropped.
            let cli_names: Option<Vec<ProxyName>> = match proxies {
                Some(csv) => Some(
                    csv.iter()
                        .map(|s| match s.as_str() {
                            "pino" => Ok(ProxyName::Pino),
                            "headroom" => Ok(ProxyName::Headroom),
                            "central" => Ok(ProxyName::Central),
                            other => Err(anyhow::anyhow!("unknown proxy name '{other}'")),
                        })
                        .collect::<anyhow::Result<Vec<_>>>()?,
                ),
                None => None,
            };
            let env_chain = std::env::var("POVERTY_PROXY_CHAIN").ok();
            let resolved = config.resolve_chain(cli_names.as_deref(), env_chain.as_deref())?;

            // The interactive picker is SEEDED from that resolved chain (spec
            // §5.10), letting the user adjust it; its confirmed FULL STATE (the
            // complete ordered proxy list, settings included) drives both the run
            // chain and `--save`. The picker is synchronous (crossterm blocking
            // reads), so it runs before the Tokio runtime is built. Without
            // `--interactive`, `entries_for_chain` produces the same full-state
            // list from the resolved chain (the single ordering authority, so the
            // two paths cannot diverge).
            let entries: Vec<crate::config::ProxyEntry> = if interactive {
                match crate::tui::run_picker(&config, &resolved)? {
                    crate::tui::reducer::TuiOutcome::Run(entries) => entries,
                    crate::tui::reducer::TuiOutcome::Cancel => {
                        println!("cancelled");
                        return Ok(());
                    }
                    crate::tui::reducer::TuiOutcome::Continue => {
                        // run_picker only returns on a terminal outcome; never Continue.
                        debug_assert!(false, "run_picker returned Continue");
                        return Ok(());
                    }
                }
            } else {
                config.entries_for_chain(&resolved)
            };

            // The run chain is the enabled members, in order, as resolved proxies.
            let chain: Vec<crate::config::ResolvedProxy> = entries
                .iter()
                .filter(|e| e.enabled)
                .map(|e| crate::config::ResolvedProxy {
                    name: e.name,
                    settings: e.settings.clone(),
                })
                .collect();

            if save {
                // --save persists the complete ordered state (every enabled flag,
                // order, and per-proxy setting) atomically.
                config.save_full_state(entries.clone())?;
            }

            // `dispatch` is synchronous; `run_command` is async (R5: its blocking
            // probes are dispatched off the executor via `spawn_blocking`). Drive
            // it on a fresh multi-thread runtime, mirroring the `proxy` arm.
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            let status = rt.block_on(crate::orchestrator::run_command(
                chain,
                &agent_argv,
                config.defaults.enable_tool_search,
            ))?;
            std::process::exit(status.code().unwrap_or(1));
        }
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
        Command::Central { action } => dispatch_central(action),
        Command::Config { action } => dispatch_config(action),
        Command::Status => {
            // `dispatch` is synchronous; `run_status` is async (R5: its blocking
            // central health/`jbcentral status` probes run off the executor via
            // `spawn_blocking`). Drive it on a fresh multi-thread runtime, mirroring
            // the `run`/`proxy` arms (R23g: MODIFY the M3 NotImplemented arm).
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            rt.block_on(crate::status::run_status())
        }
        Command::Doctor => {
            // R23g: MODIFY the M3 NotImplemented arm. `run_doctor` is synchronous
            // (pure file/settings + toolchain checks); it returns `Ok(false)` when
            // any Error-severity finding exists, which we map to a non-zero exit.
            if !crate::doctor::run_doctor()? {
                std::process::exit(1);
            }
            Ok(())
        }
        Command::Clean {
            keep,
            clear_cache,
            stop_central,
            yes,
        } => crate::clean::run_clean(keep, clear_cache, stop_central, yes),
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
                    sa.sa_sigaction = on_term as *const () as usize;
                    libc::sigemptyset(&mut sa.sa_mask);
                    libc::sigaction(libc::SIGTERM, &sa, std::ptr::null_mut());
                }
            }
            std::thread::sleep(std::time::Duration::from_secs(3600));
            Ok(())
        }
        Command::PrintEnv { name } => {
            print!("{}", std::env::var(&name).unwrap_or_default());
            use std::io::Write as _;
            std::io::stdout().flush().ok();
            Ok(())
        }
    }
}

/// Handle `poverty-mode central <login|status|stop>` (spec §5.1). All three arms
/// shell out / hit the network through the blocking `central` primitives; `dispatch`
/// is synchronous and off any executor, so they are called directly (no runtime).
fn dispatch_central(action: CentralAction) -> anyhow::Result<()> {
    use crate::central;
    let cache = crate::paths::cache_dir()?;
    match action {
        CentralAction::Login => {
            // Ensure the pinned/default version is installed, then drive the
            // detect-and-prompt login flow (R20: never bypass; browser OAuth).
            let version = central::resolve_version(None);
            let bin = central::ensure_installed(&version)?;
            central::ensure_logged_in(&bin)
        }
        CentralAction::Status => {
            // Install presence (semantic-sorted, R23f), live run state via the
            // wire-config port + `/health`, and login truth from `jbcentral status`.
            let versions = crate::status::central_versions(&cache)?;
            let running_port = central_running_port(&versions);
            let login = match crate::status::newest_central_binary(&cache)? {
                Some(bin) => central::run_status_classified(&bin)
                    .unwrap_or(central::CentralLoginState::Unknown),
                None => central::CentralLoginState::Unknown,
            };
            let status = central::CentralCommandStatus {
                versions,
                running_port,
                login,
            };
            print!("{}", central::render_central_command_status(&status));
            Ok(())
        }
        CentralAction::Stop => {
            // Best-effort stop: a not-running daemon is normalized to Ok by
            // `central::stop`; a missing install has nothing to stop.
            match crate::status::newest_central_binary(&cache)? {
                Some(bin) => central::stop(&bin),
                None => {
                    println!("central not installed; nothing to stop");
                    Ok(())
                }
            }
        }
    }
}

/// The live daemon port for `central status`: the `~/.wire/config.json` port iff an
/// install exists AND that port answers `/health`. `None` (stopped) otherwise.
///
/// Liveness is read through the SAME secret-free port reader as the global `poverty-mode
/// status` (`status::wire_config_port`), not `central::read_wire_config` (which bails when
/// `proxy_secret` is missing/empty). Sharing one reader guarantees the two status commands
/// can never disagree about whether central is running for the same on-disk state.
fn central_running_port(versions: &[String]) -> Option<u16> {
    if versions.is_empty() {
        return None;
    }
    let port = crate::status::wire_config_port()?;
    crate::central::health(port).then_some(port)
}

/// Handle `poverty-mode config <show|edit|path>` (spec §5.1).
fn dispatch_config(action: ConfigAction) -> anyhow::Result<()> {
    match action {
        ConfigAction::Show => {
            // Load (creating the safe default on first run), then print the YAML —
            // exactly what a subsequent `save` would write.
            let cfg = crate::config::Config::load_or_create()?;
            print!("{}", crate::config::render_config(&cfg)?);
            Ok(())
        }
        ConfigAction::Path => {
            println!("{}", crate::paths::config_path()?.display());
            Ok(())
        }
        ConfigAction::Edit => {
            // Ensure the file exists (first run writes the default), then open it in
            // $VISUAL/$EDITOR (fallback: notepad on Windows, vi elsewhere). The editor
            // inherits stdio so a terminal editor works.
            crate::config::Config::load_or_create()?;
            let path = crate::paths::config_path()?;
            let visual = std::env::var("VISUAL").ok();
            let editor = std::env::var("EDITOR").ok();
            let argv = crate::config::resolve_editor(visual.as_deref(), editor.as_deref());
            let (program, rest) = argv
                .split_first()
                .expect("resolve_editor always returns a non-empty argv");
            let status = std::process::Command::new(program)
                .args(rest)
                .arg(&path)
                .status()
                .with_context(|| format!("launching editor `{program}` for {}", path.display()))?;
            if !status.success() {
                anyhow::bail!("editor `{program}` exited with {:?}", status.code());
            }
            Ok(())
        }
    }
}

#[cfg(test)]
#[path = "cli_tests.rs"]
mod cli_tests;
