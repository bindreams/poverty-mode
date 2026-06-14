use super::*;
use crate::config::{CentralSettings, ProxySettings, ResolvedProxy};
use crate::proxy::headroom::HeadroomSettings;
use crate::proxy::pino::{PinoSettings, TailTtl};
use crate::proxy::ProxyName;

pub(crate) fn pino_rp() -> ResolvedProxy {
    ResolvedProxy {
        name: ProxyName::Pino,
        settings: ProxySettings::Pino(PinoSettings {
            auto_cache: true,
            tail_ttl: TailTtl::FiveMin,
            drop_tools: vec![],
            strip_ansi: true,
            model_override: None,
        }),
    }
}

pub(crate) fn headroom_rp() -> ResolvedProxy {
    ResolvedProxy {
        name: ProxyName::Headroom,
        settings: ProxySettings::Headroom(HeadroomSettings { compression: false }),
    }
}

pub(crate) fn central_rp() -> ResolvedProxy {
    ResolvedProxy {
        name: ProxyName::Central,
        settings: ProxySettings::Central(CentralSettings {
            port: None,
            pinned_version: None,
        }),
    }
}

#[test]
fn serialize_chain_renders_lowercase_csv_in_order() {
    let chain = vec![pino_rp(), headroom_rp(), central_rp()];
    assert_eq!(serialize_chain(&chain), "pino,headroom,central");
}

#[test]
fn serialize_chain_single_proxy() {
    assert_eq!(serialize_chain(&[pino_rp()]), "pino");
}

#[test]
fn serialize_chain_empty_is_empty_string() {
    let empty: Vec<ResolvedProxy> = vec![];
    assert_eq!(serialize_chain(&empty), "");
}

#[test]
fn parse_chain_reads_names_in_order() {
    assert_eq!(
        parse_chain("pino,headroom,central"),
        vec!["pino", "headroom", "central"]
    );
}

#[test]
fn parse_chain_trims_whitespace_and_drops_empties() {
    assert_eq!(parse_chain("  pino , headroom "), vec!["pino", "headroom"]);
    let empty: Vec<String> = vec![];
    assert_eq!(parse_chain(""), empty);
    assert_eq!(parse_chain("   "), empty);
}

#[test]
fn serialize_then_parse_round_trips() {
    let chain = vec![pino_rp(), headroom_rp(), central_rp()];
    let s = serialize_chain(&chain);
    let names = parse_chain(&s);
    assert_eq!(names, vec!["pino", "headroom", "central"]);
}

use crate::central::CentralInfo;

#[test]
fn central_is_tail_true_only_when_last_is_central() {
    assert!(central_is_tail(&[pino_rp(), central_rp()]));
    assert!(central_is_tail(&[central_rp()]));
    assert!(!central_is_tail(&[pino_rp(), headroom_rp()]));
    assert!(!central_is_tail(&[]));
    // central not last is not "tail" by this positional predicate.
    assert!(!central_is_tail(&[central_rp(), pino_rp()]));
}

#[test]
fn tail_is_central_wire_url_when_central_is_tail() {
    let inputs = TailInputs {
        central: Some(CentralInfo {
            port: 19516,
            secret: "abc123".to_string(),
        }),
        preexisting_base_url: Some("https://user-gateway.example.com".to_string()),
    };
    let up = resolve_tail_upstream(&inputs).unwrap();
    // central wins over a pre-existing base url.
    assert_eq!(
        up.url.as_str(),
        "http://127.0.0.1:19516/wire/abc123/claude-code/anthropic"
    );
}

#[test]
fn tail_is_preexisting_base_url_when_no_central() {
    let inputs = TailInputs {
        central: None,
        preexisting_base_url: Some("https://user-gateway.example.com/".to_string()),
    };
    let up = resolve_tail_upstream(&inputs).unwrap();
    assert_eq!(up.url.as_str(), "https://user-gateway.example.com/");
}

#[test]
fn tail_is_preexisting_base_url_with_path_prefix_preserved() {
    let inputs = TailInputs {
        central: None,
        preexisting_base_url: Some("https://gw.example.com/proxy".to_string()),
    };
    let up = resolve_tail_upstream(&inputs).unwrap();
    assert_eq!(up.url.as_str(), "https://gw.example.com/proxy");
    assert_eq!(up.path_prefix(), "/proxy");
}

#[test]
fn tail_defaults_to_anthropic_when_no_central_and_no_preexisting() {
    let inputs = TailInputs {
        central: None,
        preexisting_base_url: None,
    };
    let up = resolve_tail_upstream(&inputs).unwrap();
    assert_eq!(up.url.as_str(), "https://api.anthropic.com/");
}

#[test]
fn tail_treats_empty_preexisting_as_unset() {
    // An empty/whitespace ANTHROPIC_BASE_URL is the same as not set -> default.
    let inputs = TailInputs {
        central: None,
        preexisting_base_url: Some("   ".to_string()),
    };
    let up = resolve_tail_upstream(&inputs).unwrap();
    assert_eq!(up.url.as_str(), "https://api.anthropic.com/");
}

#[test]
fn tail_errors_on_unparseable_preexisting_base_url() {
    let inputs = TailInputs {
        central: None,
        preexisting_base_url: Some("not a url".to_string()),
    };
    let err = resolve_tail_upstream(&inputs).unwrap_err();
    assert!(
        err.to_string()
            .to_lowercase()
            .contains("anthropic_base_url"),
        "error should name the offending env var: {err}"
    );
}

fn get<'a>(env: &'a [(String, String)], key: &str) -> Option<&'a str> {
    env.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

#[test]
fn agent_env_always_sets_chain_and_tool_search() {
    let chain = vec![pino_rp(), headroom_rp()];
    let env = compute_agent_env(&chain, false);
    assert_eq!(get(&env, "POVERTY_PROXY_CHAIN"), Some("pino,headroom"));
    assert_eq!(get(&env, "ENABLE_TOOL_SEARCH"), Some("true"));
}

#[test]
fn agent_env_omits_auth_token_for_non_central_tail() {
    let chain = vec![pino_rp()];
    let env = compute_agent_env(&chain, false);
    assert_eq!(get(&env, "ANTHROPIC_AUTH_TOKEN"), None);
}

#[test]
fn agent_env_sets_wire_proxy_auth_token_for_central_tail() {
    let chain = vec![pino_rp(), central_rp()];
    let env = compute_agent_env(&chain, true);
    assert_eq!(get(&env, "ANTHROPIC_AUTH_TOKEN"), Some("wire-proxy"));
    assert_eq!(get(&env, "POVERTY_PROXY_CHAIN"), Some("pino,central"));
    assert_eq!(get(&env, "ENABLE_TOOL_SEARCH"), Some("true"));
}

#[test]
fn agent_env_never_includes_base_url_key() {
    // ANTHROPIC_BASE_URL is set by the Agent from its base_url arg, not here.
    let chain = vec![pino_rp(), central_rp()];
    let env = compute_agent_env(&chain, true);
    assert_eq!(get(&env, "ANTHROPIC_BASE_URL"), None);
}

#[test]
fn agent_env_for_empty_chain_has_empty_chain_value() {
    let chain: Vec<ResolvedProxy> = vec![];
    let env = compute_agent_env(&chain, false);
    assert_eq!(get(&env, "POVERTY_PROXY_CHAIN"), Some(""));
    assert_eq!(get(&env, "ENABLE_TOOL_SEARCH"), Some("true"));
    assert_eq!(get(&env, "ANTHROPIC_AUTH_TOKEN"), None);
}

// proxy_child_args =====
//
// NOTE (deviation from the M6.4 plan text, see milestone report): the plan's
// literal argv used `--log-file`, `--strip-ansi false`, and `--compression true`.
// Those do NOT parse against M1's actual `proxy` CLI: the per-proxy body-tee flag
// is `--body-log-file` (the global `--log-file` is a different, tracing arg), and
// `--strip-ansi`/`--auto-cache`/`--compression` are PRESENCE flags with `--no-*`
// companions, not value flags. M6.4's whole purpose is "the exact `proxy <name>`
// argv for a self-spawned hop", so the builder MUST emit a parseable argv. These
// tests pin the corrected, round-trippable argv (the `parses_back` test proves it
// re-parses through clap to identical resolved settings).

use std::path::PathBuf;

use crate::cli::{Cli, Command};
use clap::Parser;

fn pino_custom() -> ResolvedProxy {
    ResolvedProxy {
        name: ProxyName::Pino,
        settings: ProxySettings::Pino(PinoSettings {
            auto_cache: true,
            tail_ttl: TailTtl::OneHour,
            drop_tools: vec!["WebFetch".to_string(), "WebSearch".to_string()],
            strip_ansi: false,
            model_override: Some("claude-3-5-haiku".to_string()),
        }),
    }
}

#[test]
fn proxy_child_args_pino_full_flags() {
    let spec = ProxyHopSpec {
        proxy: &pino_custom(),
        listen: "127.0.0.1:0".to_string(),
        upstream: "http://127.0.0.1:55001".to_string(),
        run_id: "01HRUN".to_string(),
        log_file: PathBuf::from("/runs/r1/pino-0.log"),
    };
    let args = proxy_child_args(&spec);
    assert_eq!(
        args,
        vec![
            "proxy".to_string(),
            "pino".to_string(),
            "--listen".to_string(),
            "127.0.0.1:0".to_string(),
            "--upstream".to_string(),
            "http://127.0.0.1:55001".to_string(),
            "--run-id".to_string(),
            "01HRUN".to_string(),
            "--body-log-file".to_string(),
            "/runs/r1/pino-0.log".to_string(),
            "--auto-cache".to_string(),
            "--tail-ttl".to_string(),
            "1h".to_string(),
            "--drop-tools".to_string(),
            "WebFetch,WebSearch".to_string(),
            "--no-strip-ansi".to_string(),
            "--model-override".to_string(),
            "claude-3-5-haiku".to_string(),
        ]
    );
}

#[test]
fn proxy_child_args_pino_minimal_flags_omits_optional() {
    let spec = ProxyHopSpec {
        proxy: &pino_rp(),
        listen: "127.0.0.1:0".to_string(),
        upstream: "http://127.0.0.1:1".to_string(),
        run_id: "c".to_string(),
        log_file: PathBuf::from("/x/pino-0.log"),
    };
    let args = proxy_child_args(&spec);
    // pino_rp(): auto_cache=true, tail_ttl=5m, drop_tools=[], strip_ansi=true, model_override=None
    assert!(args.contains(&"--auto-cache".to_string()));
    assert!(args
        .windows(2)
        .any(|w| w == ["--tail-ttl".to_string(), "5m".to_string()]));
    assert!(
        !args.contains(&"--drop-tools".to_string()),
        "empty drop_tools omitted: {args:?}"
    );
    assert!(
        !args.contains(&"--model-override".to_string()),
        "unset model_override omitted: {args:?}"
    );
    // strip_ansi=true is the CLI default, so neither --strip-ansi nor
    // --no-strip-ansi is needed; we emit nothing.
    assert!(
        !args.contains(&"--no-strip-ansi".to_string()),
        "default strip_ansi=true emits no flag: {args:?}"
    );
}

#[test]
fn proxy_child_args_pino_no_auto_cache_omits_flag() {
    let rp = ResolvedProxy {
        name: ProxyName::Pino,
        settings: ProxySettings::Pino(PinoSettings {
            auto_cache: false,
            tail_ttl: TailTtl::FiveMin,
            drop_tools: vec![],
            strip_ansi: true,
            model_override: None,
        }),
    };
    let spec = ProxyHopSpec {
        proxy: &rp,
        listen: "127.0.0.1:0".to_string(),
        upstream: "http://127.0.0.1:1".to_string(),
        run_id: "c".to_string(),
        log_file: PathBuf::from("/x/pino-0.log"),
    };
    let args = proxy_child_args(&spec);
    assert!(
        !args.contains(&"--auto-cache".to_string()),
        "auto-cache off must omit the flag: {args:?}"
    );
}

#[test]
fn proxy_child_args_headroom_compression_flag() {
    let rp = ResolvedProxy {
        name: ProxyName::Headroom,
        settings: ProxySettings::Headroom(HeadroomSettings { compression: true }),
    };
    let spec = ProxyHopSpec {
        proxy: &rp,
        listen: "127.0.0.1:0".to_string(),
        upstream: "http://127.0.0.1:2".to_string(),
        run_id: "c2".to_string(),
        log_file: PathBuf::from("/x/headroom-1.log"),
    };
    let args = proxy_child_args(&spec);
    assert_eq!(args[0], "proxy");
    assert_eq!(args[1], "headroom");
    assert!(
        args.contains(&"--compression".to_string()),
        "compression=true emits the presence flag: {args:?}"
    );
    assert!(
        !args.contains(&"--auto-cache".to_string()),
        "no pino flags on headroom: {args:?}"
    );
}

#[test]
fn proxy_child_args_headroom_no_compression_emits_negation() {
    let rp = ResolvedProxy {
        name: ProxyName::Headroom,
        settings: ProxySettings::Headroom(HeadroomSettings { compression: false }),
    };
    let spec = ProxyHopSpec {
        proxy: &rp,
        listen: "127.0.0.1:0".to_string(),
        upstream: "http://127.0.0.1:2".to_string(),
        run_id: "c2".to_string(),
        log_file: PathBuf::from("/x/headroom-1.log"),
    };
    let args = proxy_child_args(&spec);
    assert!(
        args.contains(&"--no-compression".to_string()),
        "compression=false emits --no-compression: {args:?}"
    );
    assert!(
        !args.contains(&"--compression".to_string()),
        "compression=false must not emit --compression: {args:?}"
    );
}

/// Parse a generated argv back through the real clap `proxy` parser and return
/// the resolved settings, proving the self-spawn argv is genuinely parseable.
fn reparse(args: &[String]) -> ResolvedProxy {
    let mut argv = vec!["poverty-mode".to_string()];
    argv.extend(args.iter().cloned());
    let cli = Cli::try_parse_from(&argv).expect("generated proxy argv must parse via clap");
    let pargs = match cli.command {
        Command::Proxy(a) => a,
        other => panic!("expected Command::Proxy, got {other:?}"),
    };
    let settings = match pargs.which {
        ProxyName::Pino => ProxySettings::Pino(PinoSettings {
            auto_cache: pargs.auto_cache(),
            tail_ttl: pargs.pino.tail_ttl.into(),
            drop_tools: pargs
                .pino
                .drop_tools
                .iter()
                .filter(|s| !s.is_empty())
                .cloned()
                .collect(),
            strip_ansi: pargs.strip_ansi(),
            model_override: pargs.pino.model_override.clone(),
        }),
        ProxyName::Headroom => ProxySettings::Headroom(HeadroomSettings {
            compression: pargs.compression(),
        }),
        ProxyName::Central => panic!("central is never a first-party proxy hop"),
    };
    ResolvedProxy {
        name: pargs.which,
        settings,
    }
}

#[test]
fn proxy_child_args_round_trips_through_clap() {
    // Every variant of resolved settings must survive argv -> clap -> settings.
    for rp in [
        pino_custom(),
        pino_rp(),
        ResolvedProxy {
            name: ProxyName::Pino,
            settings: ProxySettings::Pino(PinoSettings {
                auto_cache: false,
                tail_ttl: TailTtl::FiveMin,
                drop_tools: vec![],
                strip_ansi: true,
                model_override: None,
            }),
        },
        ResolvedProxy {
            name: ProxyName::Headroom,
            settings: ProxySettings::Headroom(HeadroomSettings { compression: true }),
        },
        ResolvedProxy {
            name: ProxyName::Headroom,
            settings: ProxySettings::Headroom(HeadroomSettings { compression: false }),
        },
    ] {
        let spec = ProxyHopSpec {
            proxy: &rp,
            listen: "127.0.0.1:0".to_string(),
            upstream: "http://127.0.0.1:55001".to_string(),
            run_id: "01HRUN".to_string(),
            log_file: PathBuf::from("/runs/r1/hop.log"),
        };
        let args = proxy_child_args(&spec);
        let reparsed = reparse(&args);
        assert_eq!(
            reparsed, rp,
            "argv did not round-trip for {rp:?} -> {args:?}"
        );
    }
}

// read_ready_line =====

use tokio::io::BufReader;

#[tokio::test]
async fn read_ready_line_parses_valid_line() {
    let line = r#"{"ready":true,"port":54321,"proxy":"pino","run_id":"rid-1"}"#.to_string() + "\n";
    let mut reader = BufReader::new(line.as_bytes());
    let rl = read_ready_line(&mut reader, ProxyName::Pino, "rid-1")
        .await
        .unwrap();
    assert_eq!(rl.port, 54321);
    assert_eq!(rl.proxy, "pino");
    assert_eq!(rl.run_id, "rid-1");
    assert!(rl.ready);
}

#[tokio::test]
async fn read_ready_line_ignores_non_json_noise_until_json() {
    // A child might print a stray plain-text log line before the READY line; we
    // skip non-JSON-object lines and take the first parseable ReadyLine.
    let s = "starting up\nwarming\n".to_string()
        + r#"{"ready":true,"port":7000,"proxy":"headroom","run_id":"x"}"#
        + "\n";
    let mut reader = BufReader::new(s.as_bytes());
    let rl = read_ready_line(&mut reader, ProxyName::Headroom, "x")
        .await
        .unwrap();
    assert_eq!(rl.port, 7000);
}

#[tokio::test]
async fn read_ready_line_errors_on_eof_without_ready() {
    let s = "some log\nmore log\n";
    let mut reader = BufReader::new(s.as_bytes());
    let err = read_ready_line(&mut reader, ProxyName::Pino, "rid")
        .await
        .unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("ready"),
        "EOF before READY must error: {err}"
    );
}

#[tokio::test]
async fn read_ready_line_surfaces_malformed_ready_object_not_silent_skip() {
    // A JSON object that HAS a "ready" key but the wrong shape (missing port)
    // must surface a parse error, NOT be skipped and re-reported later as EOF.
    let line = r#"{"ready":true,"proxy":"pino"}"#.to_string() + "\n";
    let mut reader = BufReader::new(line.as_bytes());
    let err = read_ready_line(&mut reader, ProxyName::Pino, "rid")
        .await
        .unwrap_err();
    let m = err.to_string().to_lowercase();
    assert!(
        m.contains("malformed") || m.contains("parse") || m.contains("ready line"),
        "malformed READY object must be diagnosed, not swallowed: {m}"
    );
    // And it must NOT degrade into the generic EOF message.
    assert!(
        !m.contains("closed its stdout"),
        "must not masquerade as EOF: {m}"
    );
}

#[tokio::test]
async fn read_ready_line_errors_on_proxy_name_mismatch() {
    let line = r#"{"ready":true,"port":1,"proxy":"headroom","run_id":"rid"}"#.to_string() + "\n";
    let mut reader = BufReader::new(line.as_bytes());
    let err = read_ready_line(&mut reader, ProxyName::Pino, "rid")
        .await
        .unwrap_err();
    let m = err.to_string().to_lowercase();
    assert!(
        m.contains("proxy") && (m.contains("pino") || m.contains("headroom")),
        "{m}"
    );
}

#[tokio::test]
async fn read_ready_line_errors_on_run_id_mismatch() {
    let line = r#"{"ready":true,"port":1,"proxy":"pino","run_id":"other"}"#.to_string() + "\n";
    let mut reader = BufReader::new(line.as_bytes());
    let err = read_ready_line(&mut reader, ProxyName::Pino, "expected")
        .await
        .unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("run id")
            || err.to_string().to_lowercase().contains("run_id"),
        "{err}"
    );
}

#[tokio::test]
async fn read_ready_line_errors_when_ready_false() {
    let line = r#"{"ready":false,"port":1,"proxy":"pino","run_id":"rid"}"#.to_string() + "\n";
    let mut reader = BufReader::new(line.as_bytes());
    let err = read_ready_line(&mut reader, ProxyName::Pino, "rid")
        .await
        .unwrap_err();
    assert!(err.to_string().to_lowercase().contains("ready"), "{err}");
}

// nested_reuse_decision =====

use url::Url;

#[test]
fn nested_reuse_decision_some_when_sig_matches_and_live() {
    let live = |_u: &Url| true;
    let got = nested_reuse_decision(
        "pino,headroom",                           // desired signature
        Some("pino,headroom".to_string()),         // env POVERTY_PROXY_CHAIN
        Some("http://127.0.0.1:4100".to_string()), // env ANTHROPIC_BASE_URL
        live,
    );
    assert_eq!(
        got.map(|u| u.to_string()),
        Some("http://127.0.0.1:4100/".to_string())
    );
}

#[test]
fn nested_reuse_decision_none_when_chain_env_unset() {
    let live = |_u: &Url| true;
    let got = nested_reuse_decision(
        "pino",
        None,
        Some("http://127.0.0.1:4100".to_string()),
        live,
    );
    assert!(got.is_none());
}

#[test]
fn nested_reuse_decision_none_when_base_env_unset() {
    let live = |_u: &Url| true;
    let got = nested_reuse_decision("pino", Some("pino".to_string()), None, live);
    assert!(got.is_none());
}

#[test]
fn nested_reuse_decision_none_when_not_live() {
    let dead = |_u: &Url| false; // health probe failed
    let got = nested_reuse_decision(
        "pino",
        Some("pino".to_string()),
        Some("http://127.0.0.1:4100".to_string()),
        dead,
    );
    assert!(got.is_none());
}

#[test]
fn nested_reuse_decision_none_when_env_sig_differs_from_desired() {
    let live = |_u: &Url| true;
    // env says the live chain is "headroom" but we WANT "pino" -> do not reuse.
    let got = nested_reuse_decision(
        "pino",
        Some("headroom".to_string()),
        Some("http://127.0.0.1:4100".to_string()),
        live,
    );
    assert!(got.is_none(), "differing chain signature must NOT reuse");
}

#[test]
fn nested_reuse_decision_none_when_base_url_unparseable() {
    let live = |_u: &Url| true;
    let got = nested_reuse_decision(
        "pino",
        Some("pino".to_string()),
        Some("::::not a url".to_string()),
        live,
    );
    assert!(got.is_none());
}

#[test]
fn nested_reuse_decision_none_when_desired_chain_empty() {
    // An empty desired chain never reuses (there is nothing to compose with).
    let live = |_u: &Url| true;
    let got = nested_reuse_decision(
        "",
        Some("".to_string()),
        Some("http://127.0.0.1:4100".to_string()),
        live,
    );
    assert!(got.is_none(), "empty desired chain must not short-circuit");
}

#[test]
fn hop_log_file_embeds_literal_port_token_per_proxy() {
    let dir = std::path::Path::new("runs").join("01ABC");
    // §5.11 consumer parses `<proxy>-<port>.log`; the producer must emit the literal
    // `{port}` token (the engine resolves it to the real bound port), NOT a hop index
    // like `pino-0.log`. Reverting the producer to a hop index fails this assertion.
    assert_eq!(
        hop_log_file(&dir, ProxyName::Pino)
            .file_name()
            .unwrap()
            .to_string_lossy(),
        "pino-{port}.log"
    );
    assert_eq!(
        hop_log_file(&dir, ProxyName::Headroom)
            .file_name()
            .unwrap()
            .to_string_lossy(),
        "headroom-{port}.log"
    );
}

// ensure_central_started (FIX-A: central-tail orchestration wiring) =====
//
// The production `ensure_central_started` must drive the REAL `central::*`
// install/login/start/health pipeline (R4/R5/M8) — not the removed M6
// placeholder that `bail!`ed "(milestone M8)". These tests pin the orchestration
// through the `CentralOps` seam (the same injection style as `ProxyManager`,
// R15), so the seam cannot silently regress to the bail or to wrong threading.

use std::cell::RefCell;
use std::path::Path;

/// A recording fake `CentralOps`: captures the exact arguments the orchestrator
/// threads into each central operation and returns scripted results, so a test
/// asserts the call order, the resolved version, and the port handed to `start`.
struct FakeCentralOps {
    calls: RefCell<Vec<String>>,
    /// What `resolve_version` returns (and records the `cfg_pinned` it saw).
    resolved_version: String,
    /// Captured `cfg_pinned` passed to `resolve_version`.
    seen_pinned: RefCell<Option<Option<String>>>,
    /// Captured `version` passed to `ensure_installed`.
    seen_install_version: RefCell<Option<String>>,
    /// Captured `(port, version)` passed to `start`.
    seen_start: RefCell<Option<(Option<u16>, String)>>,
    /// Captured `port` passed to `health`.
    seen_health_port: RefCell<Option<u16>>,
    /// What `start` returns.
    start_info: CentralInfo,
    /// What `health` returns.
    health_ok: bool,
}

impl FakeCentralOps {
    /// A fake scripted with `resolved_version`, the `CentralInfo` `start` returns,
    /// and whether `health` reports OK. All capture slots start empty.
    fn new(resolved_version: &str, start_info: CentralInfo, health_ok: bool) -> Self {
        FakeCentralOps {
            calls: RefCell::new(Vec::new()),
            resolved_version: resolved_version.to_string(),
            seen_pinned: RefCell::new(None),
            seen_install_version: RefCell::new(None),
            seen_start: RefCell::new(None),
            seen_health_port: RefCell::new(None),
            start_info,
            health_ok,
        }
    }
}

impl CentralOps for FakeCentralOps {
    fn resolve_version(&self, cfg_pinned: Option<&str>) -> String {
        self.calls.borrow_mut().push("resolve_version".to_string());
        *self.seen_pinned.borrow_mut() = Some(cfg_pinned.map(str::to_string));
        self.resolved_version.clone()
    }
    fn ensure_installed(&self, version: &str) -> anyhow::Result<PathBuf> {
        self.calls.borrow_mut().push("ensure_installed".to_string());
        *self.seen_install_version.borrow_mut() = Some(version.to_string());
        Ok(PathBuf::from("/fake/jbcentral"))
    }
    fn ensure_logged_in(&self, _bin: &Path) -> anyhow::Result<()> {
        self.calls.borrow_mut().push("ensure_logged_in".to_string());
        Ok(())
    }
    fn start(&self, _bin: &Path, port: Option<u16>, version: &str) -> anyhow::Result<CentralInfo> {
        self.calls.borrow_mut().push("start".to_string());
        *self.seen_start.borrow_mut() = Some((port, version.to_string()));
        Ok(self.start_info.clone())
    }
    fn health(&self, port: u16) -> bool {
        self.calls.borrow_mut().push("health".to_string());
        *self.seen_health_port.borrow_mut() = Some(port);
        self.health_ok
    }
}

fn central_rp_with(port: Option<u16>, pinned: Option<&str>) -> ResolvedProxy {
    ResolvedProxy {
        name: ProxyName::Central,
        settings: ProxySettings::Central(CentralSettings {
            port,
            pinned_version: pinned.map(str::to_string),
        }),
    }
}

#[test]
fn ensure_central_started_drives_real_central_pipeline_in_order() {
    let ops = FakeCentralOps::new(
        "0.2.9",
        CentralInfo {
            port: 41733,
            secret: "s3cr3t".to_string(),
        },
        true,
    );
    let chain = vec![pino_rp(), central_rp_with(Some(20000), Some("9.9.9"))];

    let info = ensure_central_started_with(&chain, &ops).expect("central start must succeed");

    // The seam returns exactly what `start` produced.
    assert_eq!(
        info,
        CentralInfo {
            port: 41733,
            secret: "s3cr3t".to_string()
        }
    );
    // It drove the real pipeline (NOT the removed M8 bail), in order.
    assert_eq!(
        *ops.calls.borrow(),
        vec![
            "resolve_version",
            "ensure_installed",
            "ensure_logged_in",
            "start",
            "health"
        ]
    );
    // R4: version is resolved from the entry's pinned_version (live-or-fallback),
    // then threaded unchanged into install and start.
    assert_eq!(
        ops.seen_pinned.borrow().clone(),
        Some(Some("9.9.9".to_string()))
    );
    assert_eq!(
        ops.seen_install_version.borrow().clone(),
        Some("0.2.9".to_string())
    );
    // The Central entry's port is threaded into `start`; health probes the
    // LIVE daemon's port from the returned CentralInfo (not the requested one).
    assert_eq!(
        ops.seen_start.borrow().clone(),
        Some((Some(20000), "0.2.9".to_string()))
    );
    assert_eq!(*ops.seen_health_port.borrow(), Some(41733));
}

#[test]
fn ensure_central_started_threads_default_when_unpinned_and_no_port() {
    let ops = FakeCentralOps::new(
        "0.2.9",
        CentralInfo {
            port: 19516,
            secret: "x".to_string(),
        },
        true,
    );
    // central-only chain, port: None, pinned_version: None.
    let chain = vec![central_rp_with(None, None)];

    ensure_central_started_with(&chain, &ops).unwrap();

    assert_eq!(ops.seen_pinned.borrow().clone(), Some(None));
    assert_eq!(
        ops.seen_start.borrow().clone(),
        Some((None, "0.2.9".to_string()))
    );
}

#[test]
fn ensure_central_started_fails_closed_when_unhealthy() {
    let ops = FakeCentralOps::new(
        "0.2.9",
        CentralInfo {
            port: 5,
            secret: "x".to_string(),
        },
        false, // daemon started but never reports healthy
    );
    let chain = vec![central_rp_with(None, None)];

    let err = ensure_central_started_with(&chain, &ops).unwrap_err();
    let m = err.to_string().to_lowercase();
    assert!(
        m.contains("health") || m.contains("healthy"),
        "unhealthy central must fail closed, naming health: {m}"
    );
    // It must NOT be the removed M8 placeholder bail.
    assert!(
        !m.contains("milestone m8") && !m.contains("not yet wired"),
        "must drive the real pipeline, not the placeholder: {m}"
    );
}

#[test]
fn ensure_central_started_real_path_is_no_longer_the_m8_placeholder() {
    // Drive the REAL production `ensure_central_started` (the wrapper that passes
    // the real `central::*` ops) hermetically and network-free: point the cache
    // at an empty temp dir and pin a version that has no checksum pin. The real
    // pipeline then resolves the (pinned, no-network) version, finds nothing
    // installed, and `central::ensure_installed` fails CLOSED at the mandatory
    // checksum gate ("no pinned sha256 ... refusing to download unverified") —
    // BEFORE any network or child process. The error must be that real central
    // error, NEVER the removed "milestone M8 / not yet wired" placeholder bail.
    let cache = tempfile::tempdir().unwrap();
    let _guard = crate::test_support::EnvVarGuard::set("POVERTY_CACHE_DIR", Some(cache.path()));

    // Pinned (non-blank) => `resolve_version` returns it verbatim, no network.
    // Unknown version => `pinned_sha256` is None => install fails closed.
    let chain = vec![central_rp_with(None, Some("0.0.0-test-nonexistent"))];
    let err = ensure_central_started(&chain).unwrap_err();
    let m = err.to_string().to_lowercase();
    assert!(
        !m.contains("milestone m8") && !m.contains("not yet wired"),
        "real path must not be the M8 placeholder bail: {m}"
    );
    // Positive proof the real install gate was reached.
    assert!(
        m.contains("sha256") || m.contains("checksum") || m.contains("unverified"),
        "real path must reach central::ensure_installed's checksum gate: {m}"
    );
}
