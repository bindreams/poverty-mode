use super::*;
use clap::Parser;

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
        "--strip-ansi",
        "true",
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
            assert!(args.pino.auto_cache);
            assert_eq!(args.pino.tail_ttl.as_deref(), Some("1h"));
            assert_eq!(
                args.pino.drop_tools,
                Some(vec!["WebFetch".to_string(), "WebSearch".to_string()])
            );
            assert_eq!(args.pino.strip_ansi, Some(true));
            assert_eq!(args.pino.model_override.as_deref(), Some("claude-3-5-haiku"));
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
        "true",
    ])
    .expect("proxy headroom argv should parse");
    match cli.command {
        Command::Proxy(args) => {
            assert_eq!(args.which, ProxyName::Headroom);
            assert_eq!(args.common.run_id, "01ARZ");
            assert_eq!(args.headroom.compression, Some(true));
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
            assert!(args.pino.auto_cache);
            assert_eq!(args.headroom.compression, None);
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
fn dispatch_run_returns_not_implemented() {
    let cli = Cli::try_parse_from(["poverty-mode", "run", "--", "claude"]).unwrap();
    let err = dispatch(cli).unwrap_err();
    assert!(
        err.to_string().contains("not yet implemented: run"),
        "got: {err}"
    );
}
