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
