use super::*;
use std::fs;
use std::path::Path;

fn touch(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

// Two valid ULIDs (26 Crockford-base32 chars). Lexical order == chronological order.
const OLDER: &str = "01HXXXXXXXXXXXXXXXXXXXXXXA";
const NEWER: &str = "01HXXXXXXXXXXXXXXXXXXXXXXB";

#[test]
fn enumerate_runs_empty_when_runs_dir_missing() {
    let tmp = tempfile::tempdir().unwrap();
    // <tmp>/runs does not exist at all.
    let runs = enumerate_runs(&tmp.path().join("runs")).unwrap();
    assert!(runs.is_empty());
}

#[test]
fn enumerate_runs_empty_when_runs_dir_present_but_empty() {
    let tmp = tempfile::tempdir().unwrap();
    let runs_root = tmp.path().join("runs");
    fs::create_dir_all(&runs_root).unwrap();
    let runs = enumerate_runs(&runs_root).unwrap();
    assert!(runs.is_empty());
}

#[test]
fn enumerate_runs_collects_proxy_logs_sorted_by_run_id() {
    let tmp = tempfile::tempdir().unwrap();
    let runs_root = tmp.path().join("runs");

    touch(&runs_root.join(NEWER).join("pino-51001.log"), "log\n");
    touch(&runs_root.join(NEWER).join("headroom-51002.log"), "log\n");
    touch(&runs_root.join(OLDER).join("central-9100.log"), "log\n");

    // A stray non-directory entry and a non-.log file must be ignored.
    touch(&runs_root.join("stray.txt"), "ignore me");
    touch(&runs_root.join(OLDER).join("notes.md"), "ignore me");

    let runs = enumerate_runs(&runs_root).unwrap();
    assert_eq!(runs.len(), 2);

    // Sorted ascending by run_id => older first.
    assert_eq!(runs[0].run_id, OLDER);
    assert_eq!(runs[1].run_id, NEWER);

    // older run: one proxy log (central-9100).
    assert_eq!(runs[0].proxies.len(), 1);
    assert_eq!(runs[0].proxies[0].name, "central");
    assert_eq!(runs[0].proxies[0].port, 9100);
    assert_eq!(
        runs[0].proxies[0].log,
        runs_root.join(OLDER).join("central-9100.log")
    );

    // newer run: two proxy logs, sorted by name within the run.
    assert_eq!(runs[1].proxies.len(), 2);
    assert_eq!(runs[1].proxies[0].name, "headroom");
    assert_eq!(runs[1].proxies[0].port, 51002);
    assert_eq!(runs[1].proxies[1].name, "pino");
    assert_eq!(runs[1].proxies[1].port, 51001);
}

#[test]
fn enumerate_runs_skips_logs_without_port_suffix() {
    let tmp = tempfile::tempdir().unwrap();
    let runs_root = tmp.path().join("runs");

    touch(&runs_root.join(OLDER).join("pino-51001.log"), "ok");
    // Malformed: no "-<port>" segment, or a non-numeric/over-u16 port -> skipped.
    touch(&runs_root.join(OLDER).join("garbage.log"), "skip");
    touch(&runs_root.join(OLDER).join("pino-notaport.log"), "skip");
    touch(&runs_root.join(OLDER).join("pino-99999.log"), "skip"); // > u16::MAX

    let runs = enumerate_runs(&runs_root).unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].proxies.len(), 1);
    assert_eq!(runs[0].proxies[0].name, "pino");
    assert_eq!(runs[0].proxies[0].port, 51001);
}

#[test]
fn enumerate_runs_skips_non_ulid_directories() {
    // A directory under runs/ whose name is NOT a valid ULID must be ignored,
    // so it can never be enumerated (and thus never pruned by `clean`).
    let tmp = tempfile::tempdir().unwrap();
    let runs_root = tmp.path().join("runs");

    touch(&runs_root.join(NEWER).join("pino-51001.log"), "real run");
    // Human-created stray dir that is not a ULID.
    fs::create_dir_all(runs_root.join("my-scratch-notes")).unwrap();
    fs::write(runs_root.join("my-scratch-notes").join("a.log"), "x").unwrap();
    // A 26-char-but-invalid-base32 name (contains 'I', 'L', 'O', 'U' which Crockford excludes).
    fs::create_dir_all(runs_root.join("ILOUILOUILOUILOUILOUILOUIL")).unwrap();

    let runs = enumerate_runs(&runs_root).unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].run_id, NEWER);
}

fn probe(running: bool, login: CentralLogin, port: Option<u16>) -> CentralProbe {
    CentralProbe {
        running,
        login,
        port,
    }
}

#[test]
fn central_install_state_reflects_cache_presence() {
    let tmp = tempfile::tempdir().unwrap();
    let cache = tmp.path().join("cache");
    // No bin/jbcentral dir yet -> NotInstalled.
    let report = build_status_report(
        &cache,
        &tmp.path().join("runs"),
        &probe(false, CentralLogin::Unknown, None),
    )
    .unwrap();
    assert_eq!(report.central.install, CentralInstall::NotInstalled);
    assert_eq!(report.central.run, CentralRun::Stopped);
    assert_eq!(report.central.login, CentralLogin::Unknown);

    // Now place a versioned central binary dir at the canonical install path (R4).
    let v = cache.join("bin").join("jbcentral").join("0.2.9");
    fs::create_dir_all(&v).unwrap();
    touch(&v.join("jbcentral"), "#!/bin/sh\n");

    let report = build_status_report(
        &cache,
        &tmp.path().join("runs"),
        &probe(false, CentralLogin::Unknown, None),
    )
    .unwrap();
    assert_eq!(
        report.central.install,
        CentralInstall::Installed {
            versions: vec!["0.2.9".to_string()]
        }
    );
}

#[test]
fn central_versions_are_sorted_semantically_not_lexically() {
    // 0.2.10 is NEWER than 0.2.9, but a lexicographic sort puts "0.2.10" first
    // (because "1" < "9"). R23f requires (major, minor, patch) ordering so the
    // newest version is last (and `newest_central_binary` picks the real newest).
    let tmp = tempfile::tempdir().unwrap();
    let cache = tmp.path().join("cache");
    for ver in ["0.2.9", "0.2.10", "0.10.0", "0.2.2"] {
        let v = cache.join("bin").join("jbcentral").join(ver);
        fs::create_dir_all(&v).unwrap();
        touch(&v.join("jbcentral"), "bin");
    }

    let report = build_status_report(
        &cache,
        &tmp.path().join("runs"),
        &probe(false, CentralLogin::Unknown, None),
    )
    .unwrap();
    assert_eq!(
        report.central.install,
        CentralInstall::Installed {
            versions: vec![
                "0.2.2".to_string(),
                "0.2.9".to_string(),
                "0.2.10".to_string(),
                "0.10.0".to_string(),
            ]
        },
        "versions must be ordered semantically, newest last"
    );
}

#[test]
fn central_run_and_login_state_come_from_probe() {
    let tmp = tempfile::tempdir().unwrap();
    let cache = tmp.path().join("cache");
    let v = cache.join("bin").join("jbcentral").join("9.9.9");
    fs::create_dir_all(&v).unwrap();
    touch(&v.join("jbcentral"), "bin");

    let report = build_status_report(
        &cache,
        &tmp.path().join("runs"),
        &probe(true, CentralLogin::LoggedIn, Some(53117)),
    )
    .unwrap();
    assert_eq!(report.central.run, CentralRun::Running { port: 53117 });
    assert_eq!(report.central.login, CentralLogin::LoggedIn);
}

#[test]
fn central_login_logged_out_is_preserved_when_installed() {
    // A daemon can be running while the OAuth session is expired/logged-out.
    // The probe (from `jbcentral status`) carries LoggedOut and we must report it
    // faithfully -- no "secret present => logged in" heuristic.
    let tmp = tempfile::tempdir().unwrap();
    let cache = tmp.path().join("cache");
    let v = cache.join("bin").join("jbcentral").join("0.2.9");
    fs::create_dir_all(&v).unwrap();
    touch(&v.join("jbcentral"), "bin");

    let report = build_status_report(
        &cache,
        &tmp.path().join("runs"),
        &probe(true, CentralLogin::LoggedOut, Some(53117)),
    )
    .unwrap();
    assert_eq!(report.central.login, CentralLogin::LoggedOut);
}

#[test]
fn central_login_is_unknown_when_not_installed_regardless_of_probe() {
    // Even if a stale probe somehow says LoggedIn, an absent install forces Unknown.
    let tmp = tempfile::tempdir().unwrap();
    let cache = tmp.path().join("cache");
    let report = build_status_report(
        &cache,
        &tmp.path().join("runs"),
        &probe(false, CentralLogin::LoggedIn, None),
    )
    .unwrap();
    assert_eq!(report.central.install, CentralInstall::NotInstalled);
    assert_eq!(report.central.login, CentralLogin::Unknown);
}

#[test]
fn first_party_components_always_compiled_in() {
    let tmp = tempfile::tempdir().unwrap();
    let cache = tmp.path().join("cache");
    let report = build_status_report(
        &cache,
        &tmp.path().join("runs"),
        &probe(false, CentralLogin::Unknown, None),
    )
    .unwrap();
    // pino + headroom are compiled into the binary -> always "Builtin".
    assert_eq!(
        report.first_party,
        vec!["pino".to_string(), "headroom".to_string()]
    );
}

#[test]
fn report_includes_live_runs() {
    let tmp = tempfile::tempdir().unwrap();
    let cache = tmp.path().join("cache");
    let runs_root = tmp.path().join("runs");
    touch(&runs_root.join(NEWER).join("pino-40001.log"), "x");

    let report = build_status_report(
        &cache,
        &runs_root,
        &probe(false, CentralLogin::Unknown, None),
    )
    .unwrap();
    assert_eq!(report.runs.len(), 1);
    assert_eq!(report.runs[0].run_id, NEWER);
}

// --- render_status (pure) -----

#[test]
fn render_status_lists_components_central_and_runs() {
    let report = StatusReport {
        first_party: vec!["pino".to_string(), "headroom".to_string()],
        central: CentralStatus {
            install: CentralInstall::Installed {
                versions: vec!["0.2.9".to_string()],
            },
            run: CentralRun::Running { port: 53117 },
            login: CentralLogin::LoggedIn,
        },
        runs: vec![RunRecord {
            run_id: NEWER.to_string(),
            dir: PathBuf::from("/state/runs").join(NEWER),
            proxies: vec![ProxyLog {
                name: "pino".to_string(),
                port: 40001,
                log: PathBuf::from("/state/runs")
                    .join(NEWER)
                    .join("pino-40001.log"),
            }],
        }],
    };

    let out = render_status(&report);
    assert!(out.contains("pino (built-in)"), "got: {out}");
    assert!(out.contains("headroom (built-in)"), "got: {out}");
    assert!(out.contains("central: installed 0.2.9"), "got: {out}");
    assert!(out.contains("running on port 53117"), "got: {out}");
    assert!(out.contains("logged in"), "got: {out}");
    assert!(out.contains(NEWER), "got: {out}");
    assert!(out.contains("pino:40001"), "got: {out}");
}

#[test]
fn render_status_handles_not_installed_and_no_runs() {
    let report = StatusReport {
        first_party: vec!["pino".to_string(), "headroom".to_string()],
        central: CentralStatus {
            install: CentralInstall::NotInstalled,
            run: CentralRun::Stopped,
            login: CentralLogin::Unknown,
        },
        runs: vec![],
    };
    let out = render_status(&report);
    assert!(out.contains("central: not installed"), "got: {out}");
    assert!(out.contains("no live runs"), "got: {out}");
}

// --- probe assembly permutations (pure) -----

#[test]
fn assemble_probe_no_install_yields_dead_probe() {
    // Even if a wire config exists, with no install we never probe.
    let wire = WireConfig { port: Some(53117) };
    let probe = assemble_probe(false, Some(wire), CentralLogin::LoggedIn);
    assert!(!probe.running);
    assert_eq!(probe.port, None);
    assert_eq!(probe.login, CentralLogin::Unknown);
}

#[test]
fn assemble_probe_installed_no_wire_config() {
    // Installed but no ~/.wire/config.json: no port, so not running; login from arg.
    let probe = assemble_probe(true, None, CentralLogin::LoggedOut);
    assert_eq!(probe.port, None);
    assert!(!probe.running);
    assert_eq!(probe.login, CentralLogin::LoggedOut);
}

#[test]
fn assemble_probe_installed_with_wire_config_carries_port_and_login() {
    let wire = WireConfig { port: Some(53117) };
    let probe = assemble_probe(true, Some(wire), CentralLogin::LoggedIn);
    assert_eq!(probe.port, Some(53117));
    // `running` is decided by the caller's health check, fed via the WireConfig path
    // in run_status; assemble_probe records the port and login and leaves running
    // for the health-probe step, so here running stays false until health runs.
    assert_eq!(probe.login, CentralLogin::LoggedIn);
}

// --- R5-safe async entry: blocking probe must not panic on the runtime thread -----

#[tokio::test]
async fn run_status_async_entry_does_not_panic_on_blocking_probe() {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    // A throwaway HTTP/1.1 server that answers any request with 200 on /health.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        // Answer exactly one connection then stop (the probe makes one GET).
        if let Ok((mut sock, _)) = listener.accept() {
            let mut buf = [0u8; 1024];
            let _ = sock.read(&mut buf);
            let _ = sock
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
        }
    });

    // Drive the R5-safe spawn_blocking probe wrapper directly: if `central::health`
    // were called on the runtime thread it would panic; awaiting the wrapper must not.
    let running = probe_health_blocking(port).await.unwrap();
    assert!(running, "fake /health should report running");
    server.join().unwrap();
}
