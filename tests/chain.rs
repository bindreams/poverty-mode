//! Integration tests for orchestrator::build_and_run.

mod common; // the canonical M3 stub helper at tests/common/ (R3)

use std::sync::{Arc, Mutex};

use common::stub::start_stub;
use poverty_mode::agent::Agent;
use poverty_mode::config::{CentralSettings, ProxySettings, ResolvedProxy};
use poverty_mode::orchestrator;
use poverty_mode::proxy::headroom::HeadroomSettings;
use poverty_mode::proxy::pino::{PinoSettings, TailTtl};
use poverty_mode::proxy::{ProxyName, Upstream};
use url::Url;

/// Point the orchestrator's self-spawn at the real `poverty-mode` binary.
///
/// `build_and_run` re-spawns proxy hops via `std::env::current_exe()`, which in
/// an integration test is the libtest harness binary (no `proxy` subcommand), not
/// `poverty-mode`. The orchestrator honors `POVERTY_PROXY_EXE` ahead of
/// `current_exe()` precisely for this; set it once (race-free across the parallel
/// `#[tokio::test]`s) to `CARGO_BIN_EXE_poverty-mode`.
fn point_self_spawn_at_real_binary() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        std::env::set_var("POVERTY_PROXY_EXE", env!("CARGO_BIN_EXE_poverty-mode"));
    });
}

pub fn pino_passthrough() -> ResolvedProxy {
    ResolvedProxy {
        name: ProxyName::Pino,
        settings: ProxySettings::Pino(PinoSettings {
            auto_cache: false,
            tail_ttl: TailTtl::FiveMin,
            drop_tools: vec![],
            strip_ansi: false,
            model_override: None,
        }),
    }
}

pub fn headroom_passthrough() -> ResolvedProxy {
    ResolvedProxy {
        name: ProxyName::Headroom,
        settings: ProxySettings::Headroom(HeadroomSettings { compression: false }),
    }
}

pub fn central_rp() -> ResolvedProxy {
    ResolvedProxy {
        name: ProxyName::Central,
        settings: ProxySettings::Central(CentralSettings {
            port: None,
            pinned_version: None,
        }),
    }
}

/// A fake agent that records the base_url + extra_env it was handed and builds a
/// command that exits 0 with no real binary.
#[derive(Clone, Default)]
pub struct RecordingAgent {
    pub seen_base: Arc<Mutex<Option<String>>>,
    pub seen_env: Arc<Mutex<Vec<(String, String)>>>,
}

impl Agent for RecordingAgent {
    fn name(&self) -> &str {
        "recording"
    }
    fn build_command(
        &self,
        _argv: &[String],
        base_url: &Url,
        extra_env: &[(String, String)],
    ) -> tokio::process::Command {
        *self.seen_base.lock().unwrap() = Some(base_url.to_string());
        *self.seen_env.lock().unwrap() = extra_env.to_vec();
        #[cfg(unix)]
        let mut cmd = tokio::process::Command::new("true");
        #[cfg(windows)]
        let mut cmd = {
            let mut c = tokio::process::Command::new("cmd");
            c.args(["/c", "exit", "0"]);
            c
        };
        cmd.env("ANTHROPIC_BASE_URL", base_url.as_str());
        for (k, v) in extra_env {
            cmd.env(k, v);
        }
        cmd
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn empty_chain_execs_agent_pointed_at_tail_unchanged() {
    let agent = RecordingAgent::default();
    let tail = Upstream {
        url: Url::parse("https://api.anthropic.com").unwrap(),
    };
    let chain: Vec<ResolvedProxy> = vec![];
    let argv = vec!["--print".to_string(), "hi".to_string()];

    let status = orchestrator::build_and_run(chain, tail, &agent, &argv)
        .await
        .expect("build_and_run empty chain");
    assert!(status.success(), "agent exit status should be success");

    assert_eq!(
        agent.seen_base.lock().unwrap().as_deref(),
        Some("https://api.anthropic.com/")
    );
    let env = agent.seen_env.lock().unwrap().clone();
    assert!(env
        .iter()
        .any(|(k, v)| k == "POVERTY_PROXY_CHAIN" && v.is_empty()));
    assert!(env
        .iter()
        .any(|(k, v)| k == "ENABLE_TOOL_SEARCH" && v == "true"));
    assert!(env.iter().all(|(k, _)| k != "ANTHROPIC_AUTH_TOKEN"));
}

#[tokio::test(flavor = "multi_thread")]
async fn central_only_chain_execs_agent_at_wire_url_with_auth_token() {
    // chain = [central]; no first-party hops. tail_upstream is the wire URL.
    let agent = RecordingAgent::default();
    let tail = Upstream {
        url: Url::parse("http://127.0.0.1:19000/wire/SECRET/claude-code/anthropic").unwrap(),
    };
    let chain = vec![central_rp()];

    let status = orchestrator::build_and_run(chain, tail, &agent, &[])
        .await
        .expect("build_and_run central-only");
    assert!(status.success());

    // Agent pointed straight at the wire URL (central is the external daemon).
    assert_eq!(
        agent.seen_base.lock().unwrap().as_deref(),
        Some("http://127.0.0.1:19000/wire/SECRET/claude-code/anthropic")
    );
    // central tail => dummy auth token set; chain reflects central.
    let env = agent.seen_env.lock().unwrap().clone();
    assert!(env
        .iter()
        .any(|(k, v)| k == "ANTHROPIC_AUTH_TOKEN" && v == "wire-proxy"));
    assert!(env
        .iter()
        .any(|(k, v)| k == "POVERTY_PROXY_CHAIN" && v == "central"));
}

/// A fake agent that POSTs /v1/messages to its base_url (the chain head) via a
/// real loopback HTTP client, so we can assert the request flowed head->...->tail
/// and landed on the stub upstream. Uses an in-process reqwest blocking client on
/// a blocking thread (no external `curl` dependency).
#[derive(Clone, Default)]
pub struct PostingAgent {
    pub seen_base: Arc<Mutex<Option<String>>>,
}

impl Agent for PostingAgent {
    fn name(&self) -> &str {
        "posting"
    }
    fn build_command(
        &self,
        _argv: &[String],
        base_url: &Url,
        _extra_env: &[(String, String)],
    ) -> tokio::process::Command {
        *self.seen_base.lock().unwrap() = Some(base_url.to_string());
        // Self-exec the test binary's helper is not available; use the hidden
        // `poverty-mode __post <url>` helper added in M6.10 so the "agent" is a
        // real child process making one POST and exiting with the HTTP success.
        let target = format!("{}v1/messages", base_url.as_str());
        let exe = env!("CARGO_BIN_EXE_poverty-mode");
        let mut c = tokio::process::Command::new(exe);
        c.args(["__post", &target]);
        c
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn two_hop_chain_wires_head_to_tail_and_request_reaches_upstream() {
    point_self_spawn_at_real_binary();
    let stub = start_stub(r#"{"ok":true}"#);
    let tail = Upstream {
        url: Url::parse(&format!("http://127.0.0.1:{}", stub.port)).unwrap(),
    };
    let agent = PostingAgent::default();
    let chain = vec![pino_passthrough(), headroom_passthrough()];

    let status = orchestrator::build_and_run(chain, tail, &agent, &[])
        .await
        .expect("build_and_run two-hop");
    assert!(
        status.success(),
        "posting agent should succeed through the chain"
    );

    let base = agent.seen_base.lock().unwrap().clone().unwrap();
    assert!(base.starts_with("http://127.0.0.1:"), "head base: {base}");

    let cap = stub.last().expect("stub recorded a request");
    assert_eq!(cap.method, "POST");
    assert_eq!(cap.uri, "/v1/messages");
    assert_eq!(cap.x_api_key.as_deref(), Some("sk-test"));
}

#[tokio::test(flavor = "multi_thread")]
async fn trailing_central_strips_hop_and_carries_secret_path_to_tail() {
    point_self_spawn_at_real_binary();
    // chain = [pino, central]; tail_upstream is the wire URL. The central entry is
    // NOT spawned as a hop; only pino is. The request must land at
    // <stub>/wire/SECRET/claude-code/anthropic/v1/messages.
    let stub = start_stub(r#"{"ok":true}"#);
    let tail = Upstream {
        url: Url::parse(&format!(
            "http://127.0.0.1:{}/wire/SECRET/claude-code/anthropic",
            stub.port
        ))
        .unwrap(),
    };
    let agent = PostingAgent::default();
    let chain = vec![pino_passthrough(), central_rp()];

    let status = orchestrator::build_and_run(chain, tail, &agent, &[])
        .await
        .expect("build_and_run trailing-central");
    assert!(status.success());

    // Exactly one hop request reached the stub (pino), at the secret path.
    let cap = stub.last().expect("stub recorded a request");
    assert_eq!(
        cap.uri, "/wire/SECRET/claude-code/anthropic/v1/messages",
        "secret path must be prepended at the last first-party hop"
    );
    assert_eq!(
        stub.count(),
        1,
        "exactly one hop (pino) forwarded to the stub"
    );
}
