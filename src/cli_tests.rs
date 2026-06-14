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
        "--tail-ttl",
        "1h",
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
            assert_eq!(args.pino.tail_ttl, TailTtlArg::OneHour);
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
    // `which`. A pino run leaves the headroom-only flag at its default.
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
            assert!(!args.compression());
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
fn rejects_invalid_tail_ttl() {
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
        "--tail-ttl",
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

#[test]
fn dispatch_run_returns_not_implemented() {
    let cli = Cli::try_parse_from(["poverty-mode", "run", "--", "claude"]).unwrap();
    let err = dispatch(cli).unwrap_err();
    assert!(
        err.to_string().contains("not yet implemented: run"),
        "got: {err}"
    );
}

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

    // compression defaults false; --compression turns it on.
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
    assert!(a.compression(), "--compression is a presence flag");
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
        "--tail-ttl",
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
