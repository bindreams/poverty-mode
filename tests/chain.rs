//! Integration tests for orchestrator::build_and_run.

use std::sync::{Arc, Mutex};

use poverty_mode::agent::Agent;
use poverty_mode::config::{CentralSettings, ProxySettings, ResolvedProxy};
use poverty_mode::orchestrator;
use poverty_mode::proxy::headroom::HeadroomSettings;
use poverty_mode::proxy::pino::{PinoSettings, TailTtl};
use poverty_mode::proxy::{ProxyName, Upstream};
use url::Url;

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
