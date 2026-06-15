//! Proves no proxy-hop log line reaches the terminal: a hop child's stderr (its
//! tracing sink) is captured into a per-hop file, not inherited. Hermetic.

use std::path::Path;

use poverty_mode::orchestrator::teardown::ProxyGroup;

#[tokio::test(flavor = "multi_thread")]
async fn proxy_group_redirects_child_stderr_to_per_hop_file() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("pino.log");
    let exe = env!("CARGO_BIN_EXE_poverty-mode");

    let mut group = ProxyGroup::new().unwrap();
    group
        .spawn_with_stderr(
            Path::new(exe),
            &["__emit-warn".to_string(), "LEAKMARKER42".to_string()],
            &[],
            Some(&log),
        )
        .expect("spawn child with redirected stderr");
    // Real exit (no timer): wait for the child to finish.
    group.wait_all_exited().await.expect("reap child");

    let contents = std::fs::read_to_string(&log).expect("per-hop stderr log must exist");
    assert!(
        contents.contains("LEAKMARKER42"),
        "the child's stderr warn must be captured in the per-hop file, got: {contents:?}"
    );
    // is_terminal()-driven ANSI: a redirected (non-TTY) stderr must be plain text.
    assert!(
        !contents.contains('\u{1b}'),
        "redirected hop log must contain no ANSI escapes, got: {contents:?}"
    );
}

use std::process::Command as StdCommand;

#[cfg(unix)]
fn exit0_agent() -> Vec<&'static str> {
    vec!["--", "true"]
}
#[cfg(windows)]
fn exit0_agent() -> Vec<&'static str> {
    vec!["--", "cmd", "/c", "exit", "0"]
}

#[test]
fn run_writes_all_session_logs_to_one_dir_under_log_dir() {
    let log_home = tempfile::tempdir().unwrap();
    let cfg_home = tempfile::tempdir().unwrap();
    let exe = env!("CARGO_BIN_EXE_poverty-mode");

    let out = StdCommand::new(exe)
        .env("POVERTY_LOG_DIR", log_home.path())
        .env("XDG_CONFIG_HOME", cfg_home.path())
        .env_remove("POVERTY_PROXY_CHAIN")
        .env_remove("ANTHROPIC_BASE_URL")
        .arg("run")
        .args(["--proxies", "pino"])
        .args(exit0_agent())
        .output()
        .expect("spawn poverty-mode run");
    assert!(
        out.status.success(),
        "run should exit 0; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Exactly one session dir, directly under the log dir.
    let mut sessions: Vec<_> = std::fs::read_dir(log_home.path())
        .unwrap()
        .map(|e| e.unwrap())
        .filter(|e| e.file_type().unwrap().is_dir())
        .map(|e| e.path())
        .collect();
    assert_eq!(
        sessions.len(),
        1,
        "expected one session dir, got {sessions:?}"
    );
    let session = sessions.pop().unwrap();
    let name = session.file_name().unwrap().to_string_lossy().into_owned();

    // Findable name ending in a 26-char ULID segment.
    let last = name.rsplit('-').next().unwrap();
    assert_eq!(last.len(), 26, "session name must end with a ULID: {name}");
    assert!(
        name.len() > 27,
        "session name must carry a stem + timestamp prefix: {name}"
    );

    // Parent + hop logs live together in that dir.
    assert!(
        session.join("main.log").exists(),
        "parent tracing log must exist"
    );
    assert!(
        session.join("pino.log").exists(),
        "hop stderr log must exist"
    );

    // The parent must not have leaked tracing onto its own terminal (stderr). Match
    // a tracing TARGET line (`poverty_mode::<module>`), which only a tracing event
    // emits — not the binary's own name (`poverty-mode`, hyphenated) in usage/errors.
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("poverty_mode::"),
        "no tracing line should reach the terminal: {stderr:?}"
    );
}
