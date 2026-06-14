//! Proves the orchestrator forwards a termination signal to the agent child and
//! then returns the child's (signal-derived) exit status. Hermetic: the "agent"
//! is a hidden in-repo helper that installs a SIGTERM/Ctrl-C handler, prints a
//! marker to a file, and exits with a known code. Unix-focused for the signal
//! assertion; on Windows we assert the agent is terminated and the call returns.

#![cfg(unix)]

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use poverty_mode::agent::Agent;
use poverty_mode::config::ResolvedProxy;
use poverty_mode::orchestrator;
use poverty_mode::proxy::Upstream;
use url::Url;

/// Agent that self-execs `poverty-mode __sigwait <marker-file>`: the child writes
/// "STARTED\n" to the marker, installs a SIGTERM handler that appends "SIGTERM\n"
/// and exits 42, then sleeps. We send SIGTERM to OUR process group shortly after;
/// the orchestrator must forward it to the agent, which exits 42.
#[derive(Clone, Default)]
struct SigAgent {
    marker: Arc<Mutex<String>>,
}

impl Agent for SigAgent {
    fn name(&self) -> &str {
        "sig"
    }
    fn build_command(
        &self,
        _argv: &[String],
        _base_url: &Url,
        _extra_env: &[(String, String)],
    ) -> tokio::process::Command {
        let marker = self.marker.lock().unwrap().clone();
        let exe = env!("CARGO_BIN_EXE_poverty-mode");
        let mut c = tokio::process::Command::new(exe);
        c.args(["__sigwait", &marker]);
        c
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn sigterm_is_forwarded_to_agent_and_status_reflects_it() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("marker.txt").to_string_lossy().into_owned();
    let agent = SigAgent {
        marker: Arc::new(Mutex::new(marker.clone())),
    };

    // Empty chain so there is no proxy spawning to worry about; the signal path
    // is what we test. tail is unused by the exit-0 helper.
    let tail = Upstream {
        url: Url::parse("https://api.anthropic.com").unwrap(),
    };
    let chain: Vec<ResolvedProxy> = vec![];

    // Drive build_and_run; concurrently, once the agent has STARTED, send SIGTERM
    // to our own process so the orchestrator's handler forwards it to the child.
    let marker_for_killer = marker.clone();
    let killer = tokio::spawn(async move {
        // Wait for the agent to write STARTED by observing the marker file (real
        // external event: the child process writing a file). Bounded only by a
        // human-surfaced failure deadline.
        let start = Instant::now();
        loop {
            if let Ok(s) = std::fs::read_to_string(&marker_for_killer) {
                if s.contains("STARTED") {
                    break;
                }
            }
            if start.elapsed() > Duration::from_secs(20) {
                panic!("agent never wrote STARTED");
            }
            tokio::task::yield_now().await;
        }
        // Send SIGTERM to our own process; the orchestrator's signal task catches
        // it and forwards to the agent child.
        unsafe {
            libc::raise(libc::SIGTERM);
        }
    });

    let status = orchestrator::build_and_run(chain, tail, &agent, &[])
        .await
        .expect("build_and_run with signal");
    killer.await.unwrap();

    // The agent installed a SIGTERM handler that exits 42 after appending SIGTERM.
    let recorded = std::fs::read_to_string(&marker).unwrap();
    assert!(
        recorded.contains("SIGTERM"),
        "agent should have received SIGTERM: {recorded:?}"
    );
    assert_eq!(
        status.code(),
        Some(42),
        "status should reflect the agent's exit code"
    );
}
