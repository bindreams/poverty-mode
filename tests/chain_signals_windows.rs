//! Windows signal-forwarding coverage. We cannot synthesize a console Ctrl-C in a
//! hermetic test without killing the test process, but the OS-observable EFFECT of
//! the Ctrl-C select arm — the TerminateProcess backstop reaping an agent that
//! ignored Ctrl-C — IS testable. This drives `windows_ctrlc_backstop` against a
//! real long-lived agent child and asserts the child is gone afterward.

#![cfg(windows)]

use std::process::Command as StdCommand;

/// True iff a process with `pid` currently exists (via tasklist).
fn pid_alive(pid: u32) -> bool {
    let out = StdCommand::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH"])
        .output()
        .expect("run tasklist");
    String::from_utf8_lossy(&out.stdout).contains(&pid.to_string())
}

#[tokio::test(flavor = "multi_thread")]
async fn windows_ctrlc_backstop_reaps_a_non_cooperative_agent() {
    let exe = env!("CARGO_BIN_EXE_poverty-mode");
    // A long sleeper that ignores Ctrl-C (the hidden __sleep helper just sleeps).
    let mut child = tokio::process::Command::new(exe)
        .arg("__sleep")
        .spawn()
        .expect("spawn agent");
    let pid = child.id().expect("agent pid");
    assert!(pid_alive(pid), "agent should be alive right after spawn");

    // Exercise the exact backstop the Ctrl-C select arm runs.
    poverty_mode::orchestrator::windows_ctrlc_backstop(&mut child);

    // The backstop TerminateProcess'd the agent; await its real exit (no timer).
    let _ = child.wait().await;
    assert!(
        !pid_alive(pid),
        "backstop must reap a non-cooperative agent (pid {pid})"
    );
}
