use super::*;
use clap::Parser;

fn parse_proxy(args: &[&str]) -> ProxyArgs {
    let mut full = vec!["poverty-mode", "proxy"];
    full.extend_from_slice(args);
    let cli = Cli::parse_from(full);
    match cli.command {
        Command::Proxy(a) => a,
        other => panic!("expected proxy subcommand, got {other:?}"),
    }
}

#[test]
fn parses_run_with_proxies_and_trailing_agent() {
    let cli = Cli::try_parse_from([
        "poverty-mode",
        "run",
        "--proxies",
        "pino,headroom",
        "--",
        "claude",
        "--dangerously-skip-permissions",
    ])
    .expect("run argv should parse");
    match cli.command {
        Command::Run {
            proxies,
            interactive,
            save,
            no_save,
            agent_argv,
            ..
        } => {
            assert_eq!(
                proxies,
                Some(vec!["pino".to_string(), "headroom".to_string()])
            );
            assert!(!interactive);
            assert!(!save);
            assert!(!no_save);
            assert_eq!(
                agent_argv,
                vec![
                    "claude".to_string(),
                    "--dangerously-skip-permissions".to_string()
                ]
            );
        }
        other => panic!("expected Run, got {other:?}"),
    }
}

#[test]
fn run_parses_prefixed_setting_flags_into_overrides() {
    use crate::proxy::pino::CacheTtl;
    let cli = Cli::try_parse_from([
        "poverty-mode",
        "run",
        "--proxies",
        "pino,headroom",
        "--pino-main-ttl",
        "1h",
        "--pino-sub-ttl",
        "5m",
        "--pino-no-auto-cache",
        "--pino-drop-tools",
        "Bash,Edit",
        "--pino-model-override",
        "claude-opus-4-8",
        "--headroom-no-compression",
        "--central-port",
        "9000",
        "--central-pinned-version",
        "1.2.3",
        "--",
        "claude",
    ])
    .unwrap();
    let Command::Run { settings, .. } = cli.command else {
        panic!("expected Run")
    };
    let ov = settings.to_overrides();
    assert_eq!(ov.pino.main_ttl, Some(CacheTtl::OneHour));
    assert_eq!(ov.pino.sub_ttl, Some(CacheTtl::FiveMin));
    assert_eq!(ov.pino.auto_cache, Some(false));
    assert_eq!(ov.pino.drop_tools, Some(vec!["Bash".into(), "Edit".into()]));
    assert_eq!(ov.pino.model_override.as_deref(), Some("claude-opus-4-8"));
    assert_eq!(ov.headroom.compression, Some(false));
    assert_eq!(ov.central.port, Some(9000));
    assert_eq!(ov.central.pinned_version.as_deref(), Some("1.2.3"));
    assert_eq!(ov.pino.strip_ansi, None);
}

#[test]
fn run_without_setting_flags_yields_empty_overrides() {
    let cli = Cli::try_parse_from(["poverty-mode", "run", "--", "claude"]).unwrap();
    let Command::Run { settings, .. } = cli.command else {
        panic!()
    };
    assert_eq!(
        settings.to_overrides(),
        crate::config::overrides::Overrides::default()
    );
}

#[test]
fn run_pino_auto_cache_pair_resolves_true() {
    let cli =
        Cli::try_parse_from(["poverty-mode", "run", "--pino-auto-cache", "--", "claude"]).unwrap();
    let Command::Run { settings, .. } = cli.command else {
        panic!()
    };
    assert_eq!(settings.to_overrides().pino.auto_cache, Some(true));
}

#[test]
fn run_empty_drop_tools_clears_the_list() {
    // Decision: passing --pino-drop-tools with an empty value is an explicit clear
    // (Some(vec![])), which replaces any configured list with empty on apply.
    let cli = Cli::try_parse_from([
        "poverty-mode",
        "run",
        "--pino-drop-tools",
        "",
        "--",
        "claude",
    ])
    .unwrap();
    let Command::Run { settings, .. } = cli.command else {
        panic!()
    };
    assert_eq!(settings.to_overrides().pino.drop_tools, Some(vec![]));
}

#[test]
fn parses_proxy_pino_with_transform_flags() {
    let cli = Cli::try_parse_from([
        "poverty-mode",
        "proxy",
        "pino",
        "--listen",
        "127.0.0.1:0",
        "--upstream",
        "https://api.anthropic.com",
        "--run-id",
        "01ARZ",
        "--auto-cache",
        "--main-ttl",
        "1h",
        "--sub-ttl",
        "5m",
        "--drop-tools",
        "WebFetch,WebSearch",
        "--no-strip-ansi",
        "--model-override",
        "claude-3-5-haiku",
    ])
    .expect("proxy pino argv should parse");
    match cli.command {
        // R23b: `Command::Proxy` is a TUPLE variant carrying `ProxyArgs`; the
        // proxy is selected by the `which` positional and the dispatcher reads
        // the matching flattened group.
        Command::Proxy(args) => {
            assert_eq!(args.which, ProxyName::Pino);
            assert_eq!(args.common.listen.to_string(), "127.0.0.1:0");
            assert_eq!(args.common.upstream.as_str(), "https://api.anthropic.com/");
            assert_eq!(args.common.run_id, "01ARZ");
            assert!(args.auto_cache());
            assert_eq!(args.pino.main_ttl, CacheTtlArg::OneHour);
            assert_eq!(args.pino.sub_ttl, CacheTtlArg::FiveMin);
            assert_eq!(
                args.pino.drop_tools,
                vec!["WebFetch".to_string(), "WebSearch".to_string()]
            );
            assert!(!args.strip_ansi());
            assert_eq!(
                args.pino.model_override.as_deref(),
                Some("claude-3-5-haiku")
            );
        }
        other => panic!("expected Proxy, got {other:?}"),
    }
}

#[test]
fn parses_proxy_headroom_with_compression_flag() {
    let cli = Cli::try_parse_from([
        "poverty-mode",
        "proxy",
        "headroom",
        "--listen",
        "127.0.0.1:0",
        "--upstream",
        "http://127.0.0.1:9000",
        "--run-id",
        "01ARZ",
        "--compression",
    ])
    .expect("proxy headroom argv should parse");
    match cli.command {
        Command::Proxy(args) => {
            assert_eq!(args.which, ProxyName::Headroom);
            assert_eq!(args.common.run_id, "01ARZ");
            assert!(args.compression());
        }
        other => panic!("expected Proxy, got {other:?}"),
    }
}

#[test]
fn proxy_which_rejects_unknown_proxy_name() {
    // `which` is a positional parsed by `parse_first_party_proxy`: only
    // `pino`/`headroom` are accepted on the `proxy` subcommand; an unknown name
    // (or `central`) is rejected by the custom value parser, which clap surfaces
    // as `ValueValidation`.
    let err = Cli::try_parse_from([
        "poverty-mode",
        "proxy",
        "bogus",
        "--listen",
        "127.0.0.1:0",
        "--upstream",
        "http://127.0.0.1:9000",
        "--run-id",
        "x",
    ])
    .unwrap_err();
    assert_eq!(err.kind(), clap::error::ErrorKind::ValueValidation);
}

#[test]
fn proxy_pino_reads_pino_group_via_which() {
    // R23b flattens both groups onto ProxyArgs; the dispatcher selects by
    // `which`. A pino run leaves the headroom-only flag at its default (compression on).
    let cli = Cli::try_parse_from([
        "poverty-mode",
        "proxy",
        "pino",
        "--listen",
        "127.0.0.1:0",
        "--upstream",
        "https://api.anthropic.com",
        "--run-id",
        "x",
        "--auto-cache",
    ])
    .expect("proxy pino argv should parse");
    match cli.command {
        Command::Proxy(args) => {
            assert_eq!(args.which, ProxyName::Pino);
            assert!(args.auto_cache());
            assert!(args.compression(), "compression defaults on");
        }
        other => panic!("expected Proxy, got {other:?}"),
    }
}

#[test]
fn parses_central_and_config_subactions() {
    let cli = Cli::try_parse_from(["poverty-mode", "central", "login"]).unwrap();
    assert!(matches!(
        cli.command,
        Command::Central {
            action: CentralAction::Login
        }
    ));

    let cli = Cli::try_parse_from(["poverty-mode", "config", "path"]).unwrap();
    assert!(matches!(
        cli.command,
        Command::Config {
            action: ConfigAction::Path
        }
    ));
}

#[test]
fn rejects_invalid_main_ttl() {
    let err = Cli::try_parse_from([
        "poverty-mode",
        "proxy",
        "pino",
        "--listen",
        "127.0.0.1:0",
        "--upstream",
        "https://api.anthropic.com",
        "--run-id",
        "x",
        "--main-ttl",
        "10m",
    ])
    .unwrap_err();
    assert_eq!(err.kind(), clap::error::ErrorKind::InvalidValue);
}

#[test]
fn rejects_invalid_sub_ttl() {
    let err = Cli::try_parse_from([
        "poverty-mode",
        "proxy",
        "pino",
        "--listen",
        "127.0.0.1:0",
        "--upstream",
        "https://api.anthropic.com",
        "--run-id",
        "x",
        "--sub-ttl",
        "10m",
    ])
    .unwrap_err();
    assert_eq!(err.kind(), clap::error::ErrorKind::InvalidValue);
}

#[test]
fn proxy_body_log_file_is_independent_of_global_log_file() {
    // The global `--log-file` (tracing destination, `Cli.log_file`) and the
    // per-proxy body-tee (`CommonProxyArgs`) MUST be distinct sinks (preamble
    // R10: `EngineConfig.log_file` is a separate body-tee log). They previously
    // collided on the same clap arg id `log_file` / long flag `--log-file`, so a
    // single `--log-file` on a `proxy` invocation populated BOTH fields. The
    // body-tee now has its own `--body-log-file` flag; the two are independent.
    let cli = Cli::try_parse_from([
        "poverty-mode",
        "--log-file",
        "/tmp/tracing.log",
        "proxy",
        "pino",
        "--listen",
        "127.0.0.1:0",
        "--upstream",
        "https://api.anthropic.com",
        "--run-id",
        "x",
        "--body-log-file",
        "/tmp/bodies.log",
    ])
    .expect("proxy argv with both log flags should parse");
    assert_eq!(
        cli.log_file.as_deref(),
        Some(std::path::Path::new("/tmp/tracing.log"))
    );
    match cli.command {
        Command::Proxy(args) => {
            assert_eq!(
                args.common.body_log_file.as_deref(),
                Some(std::path::Path::new("/tmp/bodies.log"))
            );
        }
        other => panic!("expected Proxy, got {other:?}"),
    }
}

#[test]
fn proxy_body_log_file_does_not_set_global_log_file() {
    // A `--body-log-file` on a `proxy` invocation sets ONLY the body-tee field;
    // the global tracing destination stays unset.
    let cli = Cli::try_parse_from([
        "poverty-mode",
        "proxy",
        "pino",
        "--listen",
        "127.0.0.1:0",
        "--upstream",
        "https://api.anthropic.com",
        "--run-id",
        "x",
        "--body-log-file",
        "/tmp/bodies.log",
    ])
    .expect("proxy argv with body-log-file should parse");
    assert_eq!(cli.log_file, None);
    match cli.command {
        Command::Proxy(args) => {
            assert_eq!(
                args.common.body_log_file.as_deref(),
                Some(std::path::Path::new("/tmp/bodies.log"))
            );
        }
        other => panic!("expected Proxy, got {other:?}"),
    }
}

// NOTE: the "run --interactive reaches the picker (not the NotImplemented/milestone-M9
// stub) and hits the non-TTY guard" assertion lives in tests/interactive_dispatch.rs as a
// BINARY-level assert_cmd check. It cannot be an in-process lib test: run_picker's guard
// queries the real OS fds via IsTerminal, but libtest only redirects Rust's Stdout writer
// (not fds 0/1), so an in-process call under an interactive `cargo test` sees a live TTY,
// skips the guard, and hangs on event::read. The piped assert_cmd child is non-TTY regardless.
#[test]
fn proxy_bool_flags_are_presence_with_negations() {
    // Resolved values are read via the accessors (auto_cache(), strip_ansi(),
    // compression()), which fold the raw presence flag + its --no-* companion.
    // strip_ansi defaults true; --no-strip-ansi turns it off. auto_cache defaults
    // false; --auto-cache turns it on.
    let a = parse_proxy(&[
        "pino",
        "--listen",
        "127.0.0.1:0",
        "--upstream",
        "http://127.0.0.1:9/",
        "--run-id",
        "r",
    ]);
    assert!(a.strip_ansi(), "strip_ansi defaults true");
    assert!(!a.auto_cache(), "auto_cache defaults false");

    let a = parse_proxy(&[
        "pino",
        "--listen",
        "127.0.0.1:0",
        "--upstream",
        "http://127.0.0.1:9/",
        "--run-id",
        "r",
        "--auto-cache",
        "--no-strip-ansi",
    ]);
    assert!(a.auto_cache(), "--auto-cache is a presence flag");
    assert!(!a.strip_ansi(), "--no-strip-ansi negates the default");

    // compression defaults true; --no-compression turns it off.
    let bare = &[
        "headroom",
        "--listen",
        "127.0.0.1:0",
        "--upstream",
        "http://127.0.0.1:9/",
        "--run-id",
        "r",
    ];
    let a = parse_proxy(bare);
    assert!(a.compression(), "compression defaults true");

    let a = parse_proxy(&[
        "headroom",
        "--listen",
        "127.0.0.1:0",
        "--upstream",
        "http://127.0.0.1:9/",
        "--run-id",
        "r",
        "--no-compression",
    ]);
    assert!(!a.compression(), "--no-compression negates the default");

    let a = parse_proxy(&[
        "headroom",
        "--listen",
        "127.0.0.1:0",
        "--upstream",
        "http://127.0.0.1:9/",
        "--run-id",
        "r",
        "--compression",
    ]);
    assert!(
        a.compression(),
        "--compression keeps it on (redundant with default)"
    );
}

#[test]
fn proxy_transform_kind_matches_chosen_proxy() {
    let a = parse_proxy(&[
        "pino",
        "--listen",
        "127.0.0.1:0",
        "--upstream",
        "http://127.0.0.1:9/",
        "--run-id",
        "r",
        "--auto-cache",
        "--main-ttl",
        "1h",
        "--drop-tools",
        "NotebookEdit,CronList",
    ]);
    match transform_from_proxy_args(&a) {
        TransformKind::Pino(s) => {
            assert!(s.auto_cache);
            assert_eq!(
                s.drop_tools,
                vec!["NotebookEdit".to_string(), "CronList".to_string()]
            );
        }
        other => panic!("expected pino transform, got {other:?}"),
    }

    let a = parse_proxy(&[
        "headroom",
        "--listen",
        "127.0.0.1:0",
        "--upstream",
        "http://127.0.0.1:9/",
        "--run-id",
        "r",
        "--compression",
    ]);
    match transform_from_proxy_args(&a) {
        TransformKind::Headroom(s) => assert!(s.compression),
        other => panic!("expected headroom transform, got {other:?}"),
    }
}

// FIX-D: `central` / `config` subcommand dispatch =====
//
// These exercise the safe, hermetic arms of the new dispatch handlers. The env
// guards (`crate::test_support`) serialize and isolate the config/cache roots so
// the tests never touch the real user dirs (R13/R23j). The live `central login`
// and `central status`-against-a-running-daemon paths spawn `jbcentral` and need a
// real install + login; those are covered by `#[ignore]` live tests below.

#[test]
fn dispatch_config_path_succeeds_in_isolated_home() {
    let guard = crate::test_support::ConfigHomeGuard::new();
    let cli = Cli::try_parse_from(["poverty-mode", "config", "path"]).unwrap();
    // `config path` is pure path math (no file is created); it must succeed and
    // resolve under the isolated config home.
    dispatch(cli).expect("`config path` should succeed");
    assert_eq!(crate::paths::config_path().unwrap(), guard.config_file());
}

#[test]
fn dispatch_config_show_creates_default_and_succeeds() {
    let guard = crate::test_support::ConfigHomeGuard::new();
    assert!(
        !guard.config_file().exists(),
        "config file must be absent before `config show`"
    );
    let cli = Cli::try_parse_from(["poverty-mode", "config", "show"]).unwrap();
    // `config show` loads-or-creates: on first run it writes the safe default, then
    // prints it. The file must exist afterwards and re-parse into the canonical default.
    dispatch(cli).expect("`config show` should succeed");
    let text = std::fs::read_to_string(guard.config_file()).expect("config written on first show");
    let cfg: crate::config::Config = serde_yaml::from_str(&text).unwrap();
    assert_eq!(cfg, crate::config::Config::default_all_disabled());
}

#[test]
fn dispatch_central_stop_when_not_installed_is_ok() {
    // Point the cache at an empty temp dir: no jbcentral install => `central stop`
    // has nothing to stop and returns Ok WITHOUT spawning any process or hitting
    // the network. This is the safe, hermetic stop path.
    let dir = tempfile::TempDir::new().unwrap();
    let _guard = crate::test_support::EnvVarGuard::set("POVERTY_CACHE_DIR", Some(dir.path()));
    let cli = Cli::try_parse_from(["poverty-mode", "central", "stop"]).unwrap();
    dispatch(cli).expect("`central stop` with no install should be Ok");
}

#[test]
fn dispatch_central_status_when_not_installed_is_ok() {
    // With an empty cache there is no install, so `central status` reports
    // not-installed/stopped/unknown and returns Ok without any network probe
    // (the empty-versions short-circuit skips `/health` entirely).
    let dir = tempfile::TempDir::new().unwrap();
    let _guard = crate::test_support::EnvVarGuard::set("POVERTY_CACHE_DIR", Some(dir.path()));
    let cli = Cli::try_parse_from(["poverty-mode", "central", "status"]).unwrap();
    dispatch(cli).expect("`central status` with no install should be Ok");
}

#[test]
#[ignore = "live: spawns `jbcentral` and drives the interactive browser-OAuth login (needs a real install + JetBrains AI Pro)"]
fn dispatch_central_login_live() {
    let cli = Cli::try_parse_from(["poverty-mode", "central", "login"]).unwrap();
    dispatch(cli).expect("`central login` should succeed against a real jbcentral");
}

#[test]
#[ignore = "live: classifies login via a real `jbcentral status` and probes a running daemon's /health"]
fn dispatch_central_status_live() {
    let cli = Cli::try_parse_from(["poverty-mode", "central", "status"]).unwrap();
    dispatch(cli).expect("`central status` should succeed against a real jbcentral");
}
