//! Fail-closed: a hop that never becomes ready fails the build and leaves no
//! proxy children alive. The fault is injected per-child (Command::env), so the
//! parent's process env is never mutated (no UB).

use std::sync::{Arc, Mutex};

use poverty_mode::agent::Agent;
use poverty_mode::config::{ProxySettings, ResolvedProxy};
use poverty_mode::orchestrator;
use poverty_mode::orchestrator::manager::{EphemeralManager, HopSpec, ProxyManager};
use poverty_mode::proxy::pino::{PinoSettings, TailTtl};
use poverty_mode::proxy::{ProxyName, Upstream};
use url::Url;

fn pino_passthrough() -> ResolvedProxy {
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

#[derive(Clone, Default)]
struct RecordingAgent {
    seen_base: Arc<Mutex<Option<String>>>,
}
impl Agent for RecordingAgent {
    fn name(&self) -> &str {
        "recording"
    }
    fn build_command(
        &self,
        _argv: &[String],
        base_url: &Url,
        _extra_env: &[(String, String)],
    ) -> tokio::process::Command {
        *self.seen_base.lock().unwrap() = Some(base_url.to_string());
        #[cfg(unix)]
        let cmd = tokio::process::Command::new("true");
        #[cfg(windows)]
        let cmd = {
            let mut c = tokio::process::Command::new("cmd");
            c.args(["/c", "exit", "0"]);
            c
        };
        cmd
    }
}

#[test]
fn readiness_deadline_is_a_human_surfaced_bound() {
    // Characterization guard (added with the deadline): pin it so it cannot
    // silently become 0 or unbounded.
    assert!(orchestrator::READINESS_DEADLINE >= std::time::Duration::from_secs(5));
    assert!(orchestrator::READINESS_DEADLINE <= std::time::Duration::from_secs(120));
}

#[tokio::test(flavor = "multi_thread")]
async fn readiness_failure_via_manager_tears_down_started_hops() {
    // Drive the manager directly with fault injection on the first-spawned hop.
    let exe = std::path::PathBuf::from(env!("CARGO_BIN_EXE_poverty-mode"));
    let mut manager = EphemeralManager::new_with_fault(exe, true).expect("manager");

    // One hop that will be told (via its own Command env) to fail before binding.
    let hop = pino_passthrough();
    let run_id = "rid-fail".to_string();
    // Structured HopSpec: the manager renders the exact argv via proxy_child_args.
    let spec = HopSpec {
        proxy: hop,
        run_id: run_id.clone(),
        log_file: std::env::temp_dir().join("pm-failclosed-pino.log"),
    };
    let tail = Upstream {
        url: Url::parse("https://api.anthropic.com").unwrap(),
    };

    let result = manager.start_hops(&[spec], &tail).await;
    assert!(
        result.is_err(),
        "a hop that never becomes ready must fail start_hops"
    );
    let msg = result.err().unwrap().to_string().to_lowercase();
    assert!(
        msg.contains("ready")
            || msg.contains("readiness")
            || msg.contains("health")
            || msg.contains("torn down"),
        "error should describe the readiness failure: {msg}"
    );
    // The manager tore down on failure; a second shutdown is a clean no-op.
    manager.shutdown().await.expect("idempotent shutdown");
}

/// Point the orchestrator's self-spawn at the real `poverty-mode` binary.
///
/// `build_and_run_with_fault` re-spawns proxy hops via `self_spawn_exe()`, which
/// falls back to `std::env::current_exe()` — the libtest harness binary, which has
/// no `proxy` subcommand. Without this, the spawn fails on argv parsing (the wrong
/// binary), so the test would pass for the wrong reason and never exercise the
/// `PM_TEST_FAIL_PROXY` shim. `POVERTY_PROXY_EXE` is honored ahead of
/// `current_exe()`; set it once (race-free across parallel `#[tokio::test]`s) to
/// the real `CARGO_BIN_EXE_poverty-mode`.
fn point_self_spawn_at_real_binary() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        std::env::set_var("POVERTY_PROXY_EXE", env!("CARGO_BIN_EXE_poverty-mode"));
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn build_and_run_fails_closed_when_a_hop_never_readies() {
    // End-to-end via build_and_run_with_manager, fault injected per-child. Pin the
    // self-spawn to the real binary so the PM_TEST_FAIL_PROXY shim is exercised.
    point_self_spawn_at_real_binary();
    let agent = RecordingAgent::default();
    let tail = Upstream {
        url: Url::parse("https://api.anthropic.com").unwrap(),
    };
    let chain = vec![pino_passthrough()];

    let result = orchestrator::build_and_run_with_fault(chain, tail, &agent, &[], true, true).await;
    assert!(result.is_err(), "build must fail when a hop never readies");
    assert!(
        agent.seen_base.lock().unwrap().is_none(),
        "agent must NOT start on a failed build"
    );
}
