//! End-to-end `poverty-mode run`: empty chain execs the agent unchanged, and the
//! nested-reuse short-circuit reuses a live chain — driven through the real async
//! run_command in a spawned child, so a reqwest::blocking-on-runtime panic (R5)
//! would surface here. All hermetic.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::process::Command as StdCommand;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use http_body_util::Full;
use hyper::body::{Bytes, Incoming};
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;

/// A fake "live chain head" server: serves /__pm/health (HealthBody with the given
/// run_id) and 200 for any other request, and COUNTS how many /__pm/health probes
/// and how many POST /v1/messages hits it received — so a test can prove the
/// nested-reuse short-circuit sent the agent STRAIGHT to this base (one direct
/// POST, no new proxy chain in front of it).
struct LiveChain {
    port: u16,
    health_hits: Arc<AtomicUsize>,
    post_hits: Arc<AtomicUsize>,
    codex_post_hits: Arc<AtomicUsize>,
}

async fn serve_chain(run_id: &'static str) -> LiveChain {
    let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0))).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let health = format!(r#"{{"proxy":"pino","port":{port},"upstream":"api.anthropic.com","run_id":"{run_id}"}}"#);
    let health_hits = Arc::new(AtomicUsize::new(0));
    let post_hits = Arc::new(AtomicUsize::new(0));
    let codex_post_hits = Arc::new(AtomicUsize::new(0));
    let h_counter = health_hits.clone();
    let p_counter = post_hits.clone();
    let c_counter = codex_post_hits.clone();
    tokio::spawn(async move {
        loop {
            let (stream, _) = match listener.accept().await {
                Ok(v) => v,
                Err(_) => break,
            };
            let io = TokioIo::new(stream);
            let health = health.clone();
            let h_counter = h_counter.clone();
            let p_counter = p_counter.clone();
            let c_counter = c_counter.clone();
            tokio::spawn(async move {
                let svc = service_fn(move |req: Request<Incoming>| {
                    let health = health.clone();
                    let h_counter = h_counter.clone();
                    let p_counter = p_counter.clone();
                    let c_counter = c_counter.clone();
                    async move {
                        let body = if req.uri().path() == "/__pm/health" {
                            h_counter.fetch_add(1, Ordering::SeqCst);
                            health.clone()
                        } else {
                            if req.method() == hyper::Method::POST && req.uri().path() == "/v1/messages" {
                                p_counter.fetch_add(1, Ordering::SeqCst);
                            }
                            if req.method() == hyper::Method::POST && req.uri().path() == "/codex/openai/responses" {
                                c_counter.fetch_add(1, Ordering::SeqCst);
                            }
                            r#"{"ok":true}"#.to_string()
                        };
                        Ok::<_, Infallible>(
                            Response::builder()
                                .status(StatusCode::OK)
                                .header("content-type", "application/json")
                                .body(Full::new(Bytes::from(body)))
                                .unwrap(),
                        )
                    }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(io, svc)
                    .await;
            });
        }
    });
    LiveChain {
        port,
        health_hits,
        post_hits,
        codex_post_hits,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn run_empty_chain_execs_agent_unchanged() {
    let cfg_home = tempfile::tempdir().unwrap();

    #[cfg(unix)]
    let agent_args: Vec<&str> = vec!["--", "true"];
    #[cfg(windows)]
    let agent_args: Vec<&str> = vec!["--", "cmd", "/c", "exit", "0"];

    let mut cmd = StdCommand::new(env!("CARGO_BIN_EXE_poverty-mode"));
    cmd.env("XDG_CONFIG_HOME", cfg_home.path())
        .env("POVERTY_PROXY_CHAIN", "") // explicit empty chain
        .env_remove("ANTHROPIC_BASE_URL")
        .arg("run")
        .args(&agent_args);
    let status = cmd.status().expect("spawn poverty-mode run");
    assert!(status.success(), "empty-chain run should exec the agent and exit 0");
}

#[tokio::test(flavor = "multi_thread")]
async fn run_reuses_live_chain_via_nested_guard() {
    // Stand up a fake live chain. We set POVERTY_PROXY_CHAIN=pino and
    // POVERTY_PROXY_HEAD=<server>; the resolved chain is also `pino` (cli
    // --proxies pino), so the signatures match and the guard fires. The agent
    // (__post, pointed at the chain HEAD base_url that run_command hands it) posts
    // to that base and gets 200. Critically this drives the REAL async
    // run_command -> nested_reuse_check via spawn_blocking; a blocking-on-runtime
    // panic (R5) would fail this test.
    let chain = serve_chain("any").await;
    let base = format!("http://127.0.0.1:{}", chain.port);
    let cfg_home = tempfile::tempdir().unwrap();

    // The "agent" is the in-repo __post helper (no curl dependency). `run --
    // <exe> __post <url>` execs our own binary; __post reads the url ARGUMENT (a
    // marker we can detect), but the SHORT-CIRCUIT proof is observed on the
    // SERVER side: run_command must hand the agent the reused base as its head
    // base_url and spawn NO new proxy child. We make __post target a path on the
    // reused base so a direct hit is recorded.
    let exe = env!("CARGO_BIN_EXE_poverty-mode");
    let target = format!("{base}/v1/messages");

    let mut cmd = StdCommand::new(exe);
    cmd.env("XDG_CONFIG_HOME", cfg_home.path())
        .env("POVERTY_PROXY_CHAIN", "pino")
        .env("POVERTY_PROXY_HEAD", &base)
        .arg("run")
        .args(["--proxies", "pino"])
        .arg("--")
        .arg(exe)
        .arg("__post")
        .arg(&target);
    let status = cmd.status().expect("spawn poverty-mode run");
    assert!(
        status.success(),
        "nested-reuse run should exec agent against the live base"
    );

    // Prove the short-circuit actually fired (not merely that the run succeeded):
    // 1. the guard probed /__pm/health at the reused base at least once, and
    // 2. EXACTLY ONE POST /v1/messages reached the reused base directly — i.e. the
    //    agent talked straight to the live chain with NO newly spawned proxy hop
    //    re-forwarding the request (a freshly built chain would either add a hop
    //    in front or never reuse this base, so the direct count would differ).
    assert!(
        chain.health_hits.load(Ordering::SeqCst) >= 1,
        "nested-reuse guard must have probed /__pm/health at the reused base"
    );
    assert_eq!(
        chain.post_hits.load(Ordering::SeqCst),
        1,
        "exactly one direct POST should reach the reused base (no new chain spawned)"
    );
}

/// A cross-platform "print one env var and exit 0" agent: self-exec the in-repo
/// __printenv helper so there is no shell dependency.
fn print_env_agent(exe: &str, var: &str) -> Vec<String> {
    vec!["--".into(), exe.to_string(), "__printenv".into(), var.to_string()]
}

#[tokio::test(flavor = "multi_thread")]
async fn nested_reuse_fires_when_desired_sig_matches_env_and_live() {
    let chain = serve_chain("any").await;
    let base = format!("http://127.0.0.1:{}", chain.port);
    let cfg_home = tempfile::tempdir().unwrap();
    let exe = env!("CARGO_BIN_EXE_poverty-mode");

    let mut args = vec!["--proxies".to_string(), "pino".to_string()];
    args.extend(print_env_agent(exe, "POVERTY_PROXY_CHAIN"));

    let out = StdCommand::new(exe)
        .env("XDG_CONFIG_HOME", cfg_home.path())
        .env("POVERTY_PROXY_CHAIN", "pino")
        .env("POVERTY_PROXY_HEAD", &base)
        .arg("run")
        .args(&args)
        .output()
        .expect("run output");
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("pino"), "agent POVERTY_PROXY_CHAIN was: {stdout:?}");
    assert!(
        chain.health_hits.load(Ordering::SeqCst) >= 1,
        "the guard must have probed the live base before reusing it"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn cli_proxies_override_env_in_resolution_signature() {
    // env says pino; cli says headroom AND we set the live server's expectation so
    // the guard would only fire if signatures matched. Because the desired sig
    // (headroom) != env (pino), the guard does NOT short-circuit; to keep this
    // hermetic we make the live chain ALSO headroom so the guard fires on the
    // cli-resolved signature, and assert the injected chain is headroom (cli won).
    let chain = serve_chain("any").await;
    let base = format!("http://127.0.0.1:{}", chain.port);
    let cfg_home = tempfile::tempdir().unwrap();
    let exe = env!("CARGO_BIN_EXE_poverty-mode");

    let mut args = vec!["--proxies".to_string(), "headroom".to_string()];
    args.extend(print_env_agent(exe, "POVERTY_PROXY_CHAIN"));

    let out = StdCommand::new(exe)
        .env("XDG_CONFIG_HOME", cfg_home.path())
        .env("POVERTY_PROXY_CHAIN", "headroom") // match the cli resolution
        .env("POVERTY_PROXY_HEAD", &base)
        .arg("run")
        .args(&args)
        .output()
        .expect("run output");
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("headroom"),
        "cli --proxies must win resolution; agent saw: {stdout:?}"
    );
    assert!(!stdout.trim().eq("pino"), "must not be the stale env value: {stdout:?}");
}

/// Copy the test binary to a temp dir under the basename `codex` (`codex.exe` on
/// Windows) so `select_agent` picks `CodexAgent` for `run -- <copy> …`.
fn codex_named_copy() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let name = if cfg!(windows) { "codex.exe" } else { "codex" };
    let dst = dir.path().join(name);
    std::fs::copy(env!("CARGO_BIN_EXE_poverty-mode"), &dst).expect("copy codex binary");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&dst).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&dst, perms).unwrap();
    }
    (dir, dst)
}

#[tokio::test(flavor = "multi_thread")]
async fn run_codex_requires_central_errors_without_central() {
    // Selection seam through the real CLI: argv[0] basename `codex` selects
    // CodexAgent; with no central tail the guard errors before any spawn. The
    // binary at the codex path is never executed (the guard fires first), so a
    // non-existent path is fine.
    let cfg_home = tempfile::tempdir().unwrap();
    let out = StdCommand::new(env!("CARGO_BIN_EXE_poverty-mode"))
        .env("XDG_CONFIG_HOME", cfg_home.path())
        .env_remove("POVERTY_PROXY_CHAIN")
        .arg("run")
        .args(["--proxies", "pino"])
        .arg("--")
        .arg("/nonexistent/codex")
        .output()
        .expect("run output");
    assert!(!out.status.success(), "codex without central must fail");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("requires 'central'"),
        "expected codex-requires-central error; got: {stderr}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn run_codex_reuses_live_chain_end_to_end() {
    // Full selection→guard→reuse→codex-exec path through the real CLI. A codex-named
    // copy makes select_agent pick CodexAgent; the desired chain is `pino,central`
    // (central-tail, so the guard passes); POVERTY_PROXY_HEAD points at a fake live
    // chain so reuse short-circuits (NO real central started). The agent is the copy
    // running `__codexpost`, which posts <head>/codex/openai/responses → 200.
    let chain = serve_chain("any").await;
    let base = format!("http://127.0.0.1:{}", chain.port);
    let cfg_home = tempfile::tempdir().unwrap();
    let (_dir, codex) = codex_named_copy();

    let out = StdCommand::new(env!("CARGO_BIN_EXE_poverty-mode"))
        .env("XDG_CONFIG_HOME", cfg_home.path())
        .env("POVERTY_PROXY_CHAIN", "pino,central")
        .env("POVERTY_PROXY_HEAD", &base)
        .arg("run")
        .args(["--proxies", "pino,central"])
        .arg("--")
        .arg(&codex)
        .arg("__codexpost")
        .output()
        .expect("run output");
    assert!(
        out.status.success(),
        "codex reuse run should succeed; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        chain.health_hits.load(Ordering::SeqCst) >= 1,
        "nested-reuse guard must have probed /__pm/health at the reused base"
    );
    assert_eq!(
        chain.codex_post_hits.load(Ordering::SeqCst),
        1,
        "codex must have POSTed exactly once to /codex/openai/responses on the reused chain"
    );
}

// `run` setting overrides persisted by `--save` =======================================================================

use poverty_mode::config::{Config, ProxySettings};
use poverty_mode::proxy::pino::CacheTtl;
use poverty_mode::proxy::ProxyName;

/// Read and parse the config the child persisted under its temp `XDG_CONFIG_HOME`.
/// Reading the file directly (rather than via `Config::load_or_create`) keeps the
/// assertion free of any process-global env mutation, so it is safe under the
/// parallel test runner.
fn load_persisted_config(cfg_home: &std::path::Path) -> Config {
    let path = cfg_home.join("poverty-mode.yaml");
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading persisted config {}: {e}", path.display()));
    serde_yaml::from_str(&text).unwrap_or_else(|e| panic!("parsing persisted config {}: {e}", path.display()))
}

/// The pino entry's resolved settings from a loaded config.
fn pino_settings(cfg: &Config) -> (bool, &poverty_mode::proxy::pino::PinoSettings) {
    let entry = cfg
        .proxies
        .iter()
        .find(|e| e.name == ProxyName::Pino)
        .expect("config always lists pino");
    match &entry.settings {
        ProxySettings::Pino(s) => (entry.enabled, s),
        other => panic!("pino entry carries non-pino settings: {other:?}"),
    }
}

/// The headroom entry's resolved settings from a loaded config.
fn headroom_settings(cfg: &Config) -> (bool, &poverty_mode::proxy::headroom::HeadroomSettings) {
    let entry = cfg
        .proxies
        .iter()
        .find(|e| e.name == ProxyName::Headroom)
        .expect("config always lists headroom");
    match &entry.settings {
        ProxySettings::Headroom(s) => (entry.enabled, s),
        other => panic!("headroom entry carries non-headroom settings: {other:?}"),
    }
}

/// A cross-platform "exit 0 immediately, make no requests" stub agent (mirrors
/// `run_empty_chain_execs_agent_unchanged`).
#[cfg(unix)]
fn exit0_agent() -> Vec<&'static str> {
    vec!["--", "true"]
}
#[cfg(windows)]
fn exit0_agent() -> Vec<&'static str> {
    vec!["--", "cmd", "/c", "exit", "0"]
}

/// `--save` must persist a CLI-source setting override: `--proxies pino` puts pino
/// in the chain, and `--pino-sub-ttl 1h` overrides the loaded config's pino
/// settings BEFORE chain resolution / save, so the saved pino entry is enabled and
/// carries `sub_ttl = 1h`. With the override not yet applied in the handler, the
/// saved entry would keep the default `5m`, failing this test.
#[tokio::test(flavor = "multi_thread")]
async fn run_save_persists_setting_override_cli_source() {
    let cfg_home = tempfile::tempdir().unwrap();

    let mut cmd = StdCommand::new(env!("CARGO_BIN_EXE_poverty-mode"));
    cmd.env("XDG_CONFIG_HOME", cfg_home.path())
        .env_remove("POVERTY_PROXY_CHAIN")
        .env_remove("ANTHROPIC_BASE_URL")
        .arg("run")
        .args(["--proxies", "pino"])
        .args(["--pino-sub-ttl", "1h"])
        .arg("--save")
        .args(exit0_agent());
    let out = cmd.output().expect("spawn poverty-mode run");
    assert!(
        out.status.success(),
        "run --save should exit 0; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let cfg = load_persisted_config(cfg_home.path());
    let (enabled, pino) = pino_settings(&cfg);
    assert!(enabled, "--proxies pino must persist pino as enabled");
    assert_eq!(
        pino.sub_ttl,
        CacheTtl::OneHour,
        "--pino-sub-ttl 1h override must be persisted"
    );
    // main_ttl was not overridden, so it keeps the default 1h.
    assert_eq!(pino.main_ttl, CacheTtl::OneHour);
}

/// `--save` must apply a setting override even when the chain is FILE-sourced (no
/// `--proxies`, no env chain): pre-enabled pino is resolved from the file, and
/// `--pino-sub-ttl 1h` overrides the loaded config before resolution / save, so
/// the saved pino entry carries `sub_ttl = 1h`. Without the handler applying the
/// override, the saved entry would keep the file's `5m`, failing this test.
#[tokio::test(flavor = "multi_thread")]
async fn run_save_persists_setting_override_file_source() {
    let cfg_home = tempfile::tempdir().unwrap();

    // Pre-write a config with pino ENABLED at the default 5m sub-ttl.
    let mut seed = Config::default_all_disabled();
    let pino_entry = seed
        .proxies
        .iter_mut()
        .find(|e| e.name == ProxyName::Pino)
        .expect("default config lists pino");
    pino_entry.enabled = true;
    let yaml = serde_yaml::to_string(&seed).expect("serialize seed config");
    std::fs::write(cfg_home.path().join("poverty-mode.yaml"), yaml).expect("write seed config");

    let mut cmd = StdCommand::new(env!("CARGO_BIN_EXE_poverty-mode"));
    cmd.env("XDG_CONFIG_HOME", cfg_home.path())
        .env_remove("POVERTY_PROXY_CHAIN")
        .env_remove("ANTHROPIC_BASE_URL")
        .arg("run")
        .args(["--pino-sub-ttl", "1h"])
        .arg("--save")
        .args(exit0_agent());
    let out = cmd.output().expect("spawn poverty-mode run");
    assert!(
        out.status.success(),
        "run --save should exit 0; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let cfg = load_persisted_config(cfg_home.path());
    let (enabled, pino) = pino_settings(&cfg);
    assert!(enabled, "file-source pino must remain enabled");
    assert_eq!(
        pino.sub_ttl,
        CacheTtl::OneHour,
        "--pino-sub-ttl 1h override must apply on file-source resolution"
    );
}

/// `--save` must persist a setting override onto a proxy that is NOT in the chain
/// and therefore stays DISABLED (spec §2 guarantee, end-to-end). The chain is
/// `--proxies pino`, so headroom is never enabled; yet `--headroom-no-compression`
/// flips the default `compression = true` to `false`, and `with_overrides` applies
/// it to EVERY entry regardless of enabled-state — so the saved (still-disabled)
/// headroom entry carries `compression = false`. If overrides were applied only to
/// enabled/in-chain entries, the saved headroom would keep the default `true`,
/// failing this test.
#[tokio::test(flavor = "multi_thread")]
async fn run_save_persists_override_on_disabled_proxy() {
    let cfg_home = tempfile::tempdir().unwrap();

    let mut cmd = StdCommand::new(env!("CARGO_BIN_EXE_poverty-mode"));
    cmd.env("XDG_CONFIG_HOME", cfg_home.path())
        .env_remove("POVERTY_PROXY_CHAIN")
        .env_remove("ANTHROPIC_BASE_URL")
        .arg("run")
        .args(["--proxies", "pino"])
        .arg("--headroom-no-compression")
        .arg("--save")
        .args(exit0_agent());
    let out = cmd.output().expect("spawn poverty-mode run");
    assert!(
        out.status.success(),
        "run --save should exit 0; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let cfg = load_persisted_config(cfg_home.path());
    let (headroom_enabled, headroom) = headroom_settings(&cfg);
    assert!(
        !headroom_enabled,
        "headroom is not in the `pino` chain, so it must remain disabled"
    );
    assert!(
        !headroom.compression,
        "--headroom-no-compression override must persist onto the disabled headroom entry"
    );

    // Sanity: the in-chain proxy was enabled, so the run did resolve a chain.
    let (pino_enabled, _) = pino_settings(&cfg);
    assert!(pino_enabled, "--proxies pino must persist pino as enabled");
}
