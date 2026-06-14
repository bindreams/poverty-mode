use super::*;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command as StdCommand, Stdio};
use std::time::{Duration, Instant};

/// Path to the `poverty-mode` binary under test. `CARGO_BIN_EXE_*` is only set
/// for integration tests under `tests/`, NOT for `--lib` unit tests, so resolve
/// it from the running test executable: it lives in `target/<profile>/deps/`,
/// and the binary lives one directory up in `target/<profile>/`.
///
/// `cargo test --lib` does not list the binary as a build prerequisite of the
/// lib-test target, so the on-disk `poverty-mode` may be stale (missing the
/// hidden `__spawn-holder`/`__sleep` arms). Build it once per test process via a
/// `OnceLock` (the build runs to completion before any spawn — a real prerequisite,
/// not a timed wait) so these tests pass whether invoked in isolation or as part
/// of a full `cargo test`.
fn exe() -> PathBuf {
    use std::sync::OnceLock;
    static BIN: OnceLock<PathBuf> = OnceLock::new();
    BIN.get_or_init(|| {
        let test_exe = std::env::current_exe().expect("current_exe");
        // .../target/<profile>/deps/<test-bin>  ->  .../target/<profile>/
        let bin_dir = test_exe
            .parent()
            .and_then(|deps| deps.parent())
            .expect("target/<profile> dir")
            .to_path_buf();
        let name = if cfg!(windows) {
            "poverty-mode.exe"
        } else {
            "poverty-mode"
        };
        let path = bin_dir.join(name);

        // Ensure the binary is current. `cargo build` is a no-op when fresh, so
        // this only rebuilds when needed and always completes before we spawn.
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
        let status = StdCommand::new(cargo)
            .args(["build", "--bin", "poverty-mode"])
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .status()
            .expect("run cargo build --bin poverty-mode");
        assert!(status.success(), "cargo build --bin poverty-mode failed");
        assert!(path.exists(), "binary not found at {}", path.display());
        path
    })
    .clone()
}

/// True iff a process with `pid` currently exists.
fn pid_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        let out = StdCommand::new("kill")
            .args(["-0", &pid.to_string()])
            .output()
            .expect("run kill -0");
        out.status.success()
    }
    #[cfg(windows)]
    {
        let out = StdCommand::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output()
            .expect("run tasklist");
        let s = String::from_utf8_lossy(&out.stdout);
        s.contains(&pid.to_string())
    }
}

/// Poll until `pid` is gone or the human-surfaced deadline elapses. The deadline
/// bounds an EXTERNAL event (a descendant the OS must reap after a parent death)
/// — the sanctioned timeout exception — reported as a failure if hit.
fn assert_pid_gone_within(pid: u32, deadline: Duration) {
    let start = Instant::now();
    loop {
        if !pid_alive(pid) {
            return;
        }
        if start.elapsed() >= deadline {
            panic!("pid {pid} still alive after {deadline:?} since parent death (orphan!)");
        }
        std::thread::yield_now();
    }
}

fn kill_pid(pid: u32) {
    #[cfg(unix)]
    {
        StdCommand::new("kill")
            .args(["-KILL", &pid.to_string()])
            .status()
            .expect("kill -KILL holder");
    }
    #[cfg(windows)]
    {
        StdCommand::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .status()
            .expect("taskkill holder");
    }
}

#[tokio::test]
async fn group_spawn_exposes_pid_and_stdout() {
    let mut group = ProxyGroup::new().expect("create group");
    let spawned = group
        .spawn(&exe(), &["__sleep".to_string()])
        .expect("spawn into group");
    assert!(spawned.pid > 0, "must report a real child pid");
    assert!(
        spawned.stdout.is_some(),
        "stdout must be piped for READY read"
    );
    group.kill_all().expect("kill group");
    group.wait_all_exited().await.expect("await group exit");
}

#[tokio::test]
async fn kill_all_reaps_grouped_child_no_orphans() {
    let mut group = ProxyGroup::new().expect("create group");
    let spawned = group
        .spawn(&exe(), &["__sleep".to_string()])
        .expect("spawn sleeper");
    let pid = spawned.pid;
    assert!(pid_alive(pid), "sleeper should be alive right after spawn");

    group.kill_all().expect("kill group");
    group.wait_all_exited().await.expect("await group exit");

    assert!(
        !pid_alive(pid),
        "sleeper pid {pid} must be gone after kill_all (no orphans)"
    );
}

/// THE R16 GUARANTEE: kill the HOLDER parent (it mem::forgets the group, so no
/// Drop/kill_all runs in it), then assert the OS reaps the grouped child.
#[test]
fn os_reaps_grouped_child_when_parent_is_killed_without_cleanup() {
    let mut holder = StdCommand::new(exe())
        .arg("__spawn-holder")
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn holder");

    let stdout = holder.stdout.take().expect("holder stdout piped");
    let mut lines = BufReader::new(stdout).lines();
    let child_pid: u32 = lines
        .next()
        .expect("holder printed a pid line")
        .expect("read pid line")
        .trim()
        .parse()
        .expect("pid parses");
    let ready = lines
        .next()
        .expect("holder printed ready")
        .expect("read ready");
    assert_eq!(ready.trim(), "HOLDER_READY");

    assert!(
        pid_alive(child_pid),
        "grouped child alive while holder lives"
    );

    // Kill the HOLDER outright. No Drop/kill_all runs inside it.
    kill_pid(holder.id());
    let _ = holder.wait();

    // The OS must reap the grouped child without any explicit kill of the child:
    // Unix via PR_SET_PDEATHSIG + death-pipe; Windows via kill-on-job-close.
    assert_pid_gone_within(child_pid, Duration::from_secs(30));
}

/// THE R16 macOS-BACKSTOP GUARANTEE (death-pipe path, exercised on Linux/CI).
/// PR_SET_PDEATHSIG is DISABLED for the holder's grouped child (via the child's
/// OWN Command env `PM_DISABLE_PDEATHSIG=1` — never the test process's global
/// env, so no env UB), so the ONLY mechanism that can reap the child when the
/// holder is killed is the death-pipe EOF watcher (the path macOS relies on).
/// Without this test the death-pipe code would be dead/untested on the CI
/// platform (it would otherwise be masked by PDEATHSIG).
#[cfg(unix)]
#[test]
fn death_pipe_alone_reaps_grouped_child_when_pdeathsig_disabled() {
    let mut holder = StdCommand::new(exe())
        .arg("__spawn-holder")
        // Force the death-pipe-only path in the HOLDER's ProxyGroup::spawn (this
        // env is read by the holder, inherited by its grouped child's pre_exec).
        .env("PM_DISABLE_PDEATHSIG", "1")
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn holder");

    let stdout = holder.stdout.take().expect("holder stdout piped");
    let mut lines = BufReader::new(stdout).lines();
    let child_pid: u32 = lines
        .next()
        .expect("holder printed a pid line")
        .expect("read pid line")
        .trim()
        .parse()
        .expect("pid parses");
    let ready = lines
        .next()
        .expect("holder printed ready")
        .expect("read ready");
    assert_eq!(ready.trim(), "HOLDER_READY");

    assert!(
        pid_alive(child_pid),
        "grouped child alive while holder lives"
    );

    // Kill the HOLDER outright. With PDEATHSIG disabled, only the death-pipe EOF
    // watcher can reap the child — proving the macOS backstop actually works.
    kill_pid(holder.id());
    let _ = holder.wait();

    assert_pid_gone_within(child_pid, Duration::from_secs(30));
}
