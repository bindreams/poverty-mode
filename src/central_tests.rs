use super::*;

// Characterization guard (R12): `central_wire_upstream` renders the JetBrains
// wire URL the orchestrator's tail resolution depends on (design §6). Labeled as
// a guard, not a red->green step — the behavior exists in this same change.
#[test]
fn central_wire_upstream_renders_jetbrains_wire_url() {
    let info = CentralInfo {
        port: 19516,
        secret: "abc123".to_string(),
    };
    let up = central_wire_upstream(&info).unwrap();
    assert_eq!(up.url.as_str(), "http://127.0.0.1:19516/wire/abc123");
    // The wire path is carried as the upstream's path prefix (no trailing slash).
    assert_eq!(up.path_prefix(), "/wire/abc123");
    assert_eq!(up.host_header(), "127.0.0.1:19516");
}

// R20: the secret is read from an external file (central's `config.json`) and may
// contain URL-significant bytes. It MUST be percent-encoded into a single path
// segment — raw interpolation silently mis-routes the central hop (a `#` truncates
// the path into a fragment; a `?` injects a query string that later 502s every
// forwarded request). `/`, `?`, `#`, space, and other delimiters must all encode.
#[test]
fn central_wire_upstream_percent_encodes_special_secret() {
    let info = CentralInfo {
        port: 19516,
        secret: "a#b?c/d e&f%g".to_string(),
    };
    let up = central_wire_upstream(&info).unwrap();
    assert_eq!(up.url.as_str(), "http://127.0.0.1:19516/wire/a%23b%3Fc%2Fd%20e%26f%25g");
    // It stays one segment: no fragment, no query, no extra path separators.
    assert_eq!(up.url.fragment(), None);
    assert_eq!(up.url.query(), None);
    assert_eq!(up.path_prefix(), "/wire/a%23b%3Fc%2Fd%20e%26f%25g");
    assert_eq!(up.host_header(), "127.0.0.1:19516");
}

// M8.5: central constants (R4) + central `config.json` -> CentralInfo parsing.

#[test]
fn parses_proxy_port_and_secret() {
    let json = r#"{
        "proxy_port": 19516,
        "proxy_secret": "abc123DEF",
        "some_other_field": "ignored"
    }"#;
    let info = parse_wire_config(json).unwrap();
    assert_eq!(info.port, 19516);
    assert_eq!(info.secret, "abc123DEF");
}

#[test]
fn errors_when_proxy_port_missing() {
    let json = r#"{ "proxy_secret": "abc" }"#;
    let err = parse_wire_config(json).unwrap_err();
    assert!(err.to_string().contains("proxy_port"), "{err}");
}

#[test]
fn errors_when_proxy_secret_missing() {
    let json = r#"{ "proxy_port": 1234 }"#;
    let err = parse_wire_config(json).unwrap_err();
    assert!(err.to_string().contains("proxy_secret"), "{err}");
}

#[test]
fn errors_on_unparseable_json_without_leaking_contents() {
    let json = r#"{ "proxy_secret": "TOPSECRET", "#; // truncated/invalid
    let err = parse_wire_config(json).unwrap_err();
    let msg = err.to_string();
    assert!(!msg.contains("TOPSECRET"), "error must not leak the secret: {msg}");
    assert!(
        msg.contains("not valid JSON"),
        "error should name the failure mode: {msg}"
    );
}

#[test]
fn accepts_string_port_coerced_to_u16() {
    // Some jbcentral builds write proxy_port as a string; accept it.
    let json = r#"{ "proxy_port": "8123", "proxy_secret": "s" }"#;
    let info = parse_wire_config(json).unwrap();
    assert_eq!(info.port, 8123);
}

// wire URL ============================================================================================================

use crate::proxy::Upstream;

#[test]
fn builds_wire_upstream_url() {
    let info = CentralInfo {
        port: 19516,
        secret: "abc123".to_string(),
    };
    let up: Upstream = central_wire_upstream(&info).unwrap();
    assert_eq!(up.url.as_str(), "http://127.0.0.1:19516/wire/abc123");
}

#[test]
fn wire_upstream_path_prefix_excludes_v1_messages() {
    // path_prefix() must be the wire path (no trailing slash); the engine appends /v1/messages.
    let info = CentralInfo {
        port: 7000,
        secret: "S".to_string(),
    };
    let up = central_wire_upstream(&info).unwrap();
    assert_eq!(up.path_prefix(), "/wire/S");
    assert_eq!(up.host_header(), "127.0.0.1:7000");
}

#[test]
fn wire_url_string_helper_matches_upstream() {
    let info = CentralInfo {
        port: 8080,
        secret: "xyz".to_string(),
    };
    assert_eq!(central_wire_envelope_url(&info), "http://127.0.0.1:8080/wire/xyz");
}

#[test]
fn wire_secret_with_url_significant_chars_is_percent_encoded() {
    // A secret containing '?', '#', space, '/', and a non-ASCII char must NOT bleed into the query,
    // fragment, or split the path. It is percent-encoded as a single path segment.
    let info = CentralInfo {
        port: 9000,
        secret: "a b/c?d#e\u{00e9}".to_string(),
    };
    let url = central_wire_envelope_url(&info);
    assert_eq!(url, "http://127.0.0.1:9000/wire/a%20b%2Fc%3Fd%23e%C3%A9");
    // It parses without panicking and the secret stays inside the path (no query/fragment leaked).
    let up = central_wire_upstream(&info).unwrap();
    assert!(up.url.query().is_none(), "secret must not leak into the query");
    assert!(up.url.fragment().is_none(), "secret must not leak into the fragment");
    assert_eq!(up.url.path(), "/wire/a%20b%2Fc%3Fd%23e%C3%A9");
}

#[test]
fn wire_url_helper_and_upstream_agree_on_encoded_secret() {
    let info = CentralInfo {
        port: 1234,
        secret: "x y".to_string(),
    };
    let up = central_wire_upstream(&info).unwrap();
    assert_eq!(up.url.as_str(), central_wire_envelope_url(&info));
}

// login state classification ==========================================================================================

#[test]
fn login_state_logged_in_on_success_exit() {
    let st = classify_login_status(Some(0), "Logged in as user@example.com", "");
    assert_eq!(st, CentralLoginState::LoggedIn);
}

#[test]
fn login_state_logged_out_on_nonzero_with_login_hint() {
    let st = classify_login_status(Some(1), "", "not logged in; run `jbcentral login`");
    assert_eq!(st, CentralLoginState::LoggedOut);
}

#[test]
fn login_state_logged_out_when_stdout_says_not_authenticated() {
    let st = classify_login_status(Some(0), "Status: not authenticated", "");
    assert_eq!(st, CentralLoginState::LoggedOut);
}

#[test]
fn login_state_unknown_on_killed_process() {
    let st = classify_login_status(None, "", "");
    assert_eq!(st, CentralLoginState::Unknown);
}

// These wiring tests (R20) exec a fake `jbcentral` generated at BUILD time by build.rs (path via
// `cargo:rustc-env`, which reaches unit tests too), not a script written here. The old in-test
// write-then-exec raced a concurrent fork and intermittently failed with ETXTBSY ("Text file
// busy") under parallel load. The spawn-error test below has no write-then-exec (the path never
// exists), so it stays a plain unit test.

#[test]
fn run_status_classified_reports_logged_in_when_status_exits_zero() {
    // Genuine wiring test (R20): the real exit code from `jbcentral status` must reach the
    // classifier. A logged-in central exits 0 with a "Logged in" banner and must classify as
    // LoggedIn -- NOT Unknown (which is what dropping the exit code would yield).
    let bin = std::path::Path::new(env!("PM_FAKE_JBCENTRAL_LOGGED_IN"));
    let state = run_status_classified(bin).unwrap();
    assert_eq!(state, CentralLoginState::LoggedIn, "exit 0 + banner => logged in");
}

#[test]
fn run_status_classified_reports_logged_out_on_nonzero_exit() {
    // A logged-out central exits non-zero; the real code must drive a LoggedOut classification.
    let bin = std::path::Path::new(env!("PM_FAKE_JBCENTRAL_LOGGED_OUT"));
    let state = run_status_classified(bin).unwrap();
    assert_eq!(state, CentralLoginState::LoggedOut, "non-zero exit => logged out");
}

#[test]
fn run_status_classified_errors_when_binary_is_missing() {
    // A missing binary is "central is not installed", not "the status call failed". An EXPLICIT path
    // must not claim PATH was searched — it never is, for a path.
    let missing = std::path::Path::new("/nonexistent/pm-central-does-not-exist");
    let err = format!("{:#}", run_status_classified(missing).unwrap_err());
    assert!(err.contains("pm-central-does-not-exist"), "{err}");
    assert!(err.contains("does not exist"), "{err}");
    assert!(
        !err.contains("on PATH"),
        "an explicit path is never searched on PATH: {err}"
    );
}

// start/health argv + env =============================================================================================

#[test]
fn proxy_start_argv_is_proxy_start() {
    assert_eq!(proxy_start_argv(), vec!["proxy".to_string(), "start".to_string()]);
}

#[test]
fn proxy_stop_argv_is_proxy_stop() {
    assert_eq!(proxy_stop_argv(), vec!["proxy".to_string(), "stop".to_string()]);
}

#[test]
fn start_env_sets_wire_proxy_port_when_requested() {
    let env = start_env(Some(19999));
    assert_eq!(
        env.iter()
            .find(|(k, _)| k == "WIRE_PROXY_PORT")
            .map(|(_, v)| v.as_str()),
        Some("19999")
    );
}

#[test]
fn start_env_omits_wire_proxy_port_when_none() {
    let env = start_env(None);
    assert!(env.iter().all(|(k, _)| k != "WIRE_PROXY_PORT"));
}

#[test]
fn health_url_targets_loopback_health_route() {
    assert_eq!(health_url(19516), "http://127.0.0.1:19516/health");
}

// central state dir (#33) =============================================================================================
// central was renamed from `jbcentral` at 1.0 and moved its state dir from `~/.wire` to
// `~/.jetbrains-central`. Resolution is single-path: the legacy dir is never consulted.

#[test]
fn state_files_resolve_under_dot_jetbrains_central() {
    // Assert on components, not a joined string: a `/`-separated literal fails on Windows, and
    // rebuilding the expectation with the same `join` calls the implementation uses would restate
    // the code and pass whichever directory it picked.
    let home = std::path::Path::new("/home/someone");
    for (path, file) in [
        (wire_config_path_in(home), "config.json"),
        (proxy_pid_path_in(home), "proxy.pid"),
    ] {
        assert!(path.starts_with(home), "{}", path.display());
        assert_eq!(path.file_name().unwrap(), file, "{}", path.display());
        assert_eq!(
            path.parent().unwrap().file_name().unwrap(),
            ".jetbrains-central",
            "{}",
            path.display()
        );
    }
}

#[test]
fn real_home_resolution_matches_the_injectable_seam() {
    // Pins `home_dir()` into the tested path: the no-arg resolvers must be the `_in` variants
    // applied to $HOME, not a second, drifting copy of the layout rule.
    let home = home_dir().unwrap();
    assert_eq!(wire_config_path().unwrap(), wire_config_path_in(&home));
    assert_eq!(proxy_pid_path().unwrap(), proxy_pid_path_in(&home));
}

#[test]
fn absent_config_is_none_but_corrupt_config_is_an_error() {
    let home = tempfile::tempdir().unwrap();
    // Never written: nothing to reuse.
    assert_eq!(existing_config_in(home.path()).unwrap(), None);

    // Present but unparseable: an error, not a silent "start a second daemon over it".
    let dir = home.path().join(".jetbrains-central");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("config.json"), "{ not json").unwrap();
    let err = existing_config_in(home.path()).unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("config.json"), "error must name the file: {msg}");

    // Well-formed: parsed through.
    std::fs::write(
        dir.join("config.json"),
        r#"{ "proxy_port": 19516, "proxy_secret": "s" }"#,
    )
    .unwrap();
    assert_eq!(existing_config_in(home.path()).unwrap().unwrap().port, 19516);
}

#[test]
fn read_failures_name_the_file_they_came_from() {
    let home = tempfile::tempdir().unwrap();
    let missing = format!("{:#}", read_wire_config_in(home.path()).unwrap_err());
    assert!(missing.contains("config.json"), "{missing}");

    let dir = home.path().join(".jetbrains-central");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("config.json"), "{ \"proxy_port\": 1 }").unwrap();
    let corrupt = format!("{:#}", read_wire_config_in(home.path()).unwrap_err());
    assert!(corrupt.contains("config.json"), "{corrupt}");
    assert!(corrupt.contains("proxy_secret"), "{corrupt}");
}

#[test]
fn legacy_dot_wire_layout_is_not_consulted() {
    // A pre-1.0 `~/.wire` next to no `~/.jetbrains-central` must NOT be picked up: resolution
    // names the post-1.0 path regardless of what else is on disk.
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(home.path().join(".wire")).unwrap();
    std::fs::write(
        home.path().join(".wire").join("config.json"),
        r#"{ "proxy_port": 19516, "proxy_secret": "legacy" }"#,
    )
    .unwrap();

    let resolved = wire_config_path_in(home.path());
    assert_eq!(resolved, home.path().join(".jetbrains-central").join("config.json"));
    assert!(!resolved.exists(), "legacy dir must not satisfy resolution");
}

#[cfg(unix)]
#[test]
fn compat_symlink_layout_resolves_through_to_the_live_config() {
    // Compat install (this repo's dev Mac): `~/.jetbrains-central` is a symlink to `~/.wire`.
    // Resolution must read the live config through it.
    let home = tempfile::tempdir().unwrap();
    let legacy = home.path().join(".wire");
    std::fs::create_dir_all(&legacy).unwrap();
    std::fs::write(
        legacy.join("config.json"),
        r#"{ "proxy_port": 19516, "proxy_secret": "live" }"#,
    )
    .unwrap();
    std::os::unix::fs::symlink(&legacy, home.path().join(".jetbrains-central")).unwrap();

    let resolved = wire_config_path_in(home.path());
    let info = parse_wire_config(&std::fs::read_to_string(&resolved).unwrap()).unwrap();
    assert_eq!(info.port, 19516);
    assert_eq!(info.secret, "live");
}

#[test]
fn start_reuse_keeps_live_daemon_port() {
    // On singleton reuse the LIVE daemon's port wins; a caller's requested port is NOT consulted (we
    // never rebind a shared daemon). `reuse_decision` is the pure seam `start` uses for this choice.
    let live = CentralInfo {
        port: 19516,
        secret: "live".to_string(),
    };
    // Healthy + existing => reuse, returning the live daemon's CentralInfo verbatim (port 19516),
    // regardless of any port the caller would have requested.
    assert_eq!(reuse_decision(Some(live.clone()), true), Some(live));
    // Unhealthy => no reuse (start proceeds to (re)configure + start with the requested port).
    assert_eq!(
        reuse_decision(
            Some(CentralInfo {
                port: 1,
                secret: "x".into()
            }),
            false
        ),
        None
    );
    // No existing config => no reuse.
    assert_eq!(reuse_decision(None, true), None);
}

// central executable resolution =======================================================================================
// central is an external tool found on PATH. `central_executable` yields the NAME to spawn (execvp
// does the lookup); `locate_executable*` is ADVISORY reporting only and must never gate a run.

#[test]
fn central_executable_defaults_to_central() {
    assert_eq!(central_executable(None), std::path::Path::new("central"));
    assert_eq!(central_executable(Some("")), std::path::Path::new("central"));
    assert_eq!(central_executable(Some("   ")), std::path::Path::new("central"));
    assert_eq!(central_executable(Some(" mine ")), std::path::Path::new("mine"));
}

#[test]
fn missing_central_error_names_the_executable() {
    let msg = format!("{:#}", missing_central_error(std::path::Path::new("central")));
    assert!(msg.contains("central"), "{msg}");
    assert!(msg.contains("PATH"), "{msg}");
}

#[test]
fn explicit_path_is_never_searched_on_path() {
    // A name carrying a separator is a path: it resolves only if it is a file on disk, and a
    // same-named file sitting on PATH must not satisfy it.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("central"), "x").unwrap();
    let path_var = std::ffi::OsString::from(dir.path());

    let explicit = std::path::Path::new("nested/central");
    assert_eq!(locate_executable_in(explicit, Some(&path_var)), None);

    let real = dir.path().join("central");
    assert_eq!(locate_executable_in(&real, Some(&path_var)), Some(real.clone()));
}

#[test]
fn bare_name_is_found_across_path_entries() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    // The fixture must carry the platform's executable suffix: on Windows a dotless bare name is
    // probed ONLY as `<name>.exe`, so an extensionless file is (correctly) not a candidate.
    let found = second.path().join(format!("central{}", std::env::consts::EXE_SUFFIX));
    std::fs::write(&found, "x").unwrap();

    let path_var = std::env::join_paths([first.path(), second.path()]).unwrap();
    assert_eq!(
        locate_executable_in(std::path::Path::new("central"), Some(&path_var)),
        Some(found)
    );
}

#[test]
fn absent_name_and_absent_path_var_resolve_to_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path_var = std::ffi::OsString::from(dir.path());
    assert_eq!(
        locate_executable_in(std::path::Path::new("central"), Some(&path_var)),
        None
    );
    assert_eq!(locate_executable_in(std::path::Path::new("central"), None), None);
}

#[cfg(windows)]
#[test]
fn bare_name_exe_probing_matches_std() {
    // std's rule (`sys::process::windows::resolve_exe`) is "any dot means it already has an
    // extension": a dotted bare name is NOT given `.exe`. Probing `central-0.6.0.exe` here would
    // report a binary `Command` cannot spawn.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("central-0.6.0.exe"), "x").unwrap();
    let path_var = std::ffi::OsString::from(dir.path());
    assert_eq!(
        locate_executable_in(std::path::Path::new("central-0.6.0"), Some(&path_var)),
        None,
        "std would not append .exe to a dotted bare name"
    );

    // A dotless bare name is probed ONLY as `<name>.exe`: std calls `set_extension`, so an
    // extensionless file on PATH is not a candidate and must not be reported.
    let extensionless = dir.path().join("plaincentral");
    std::fs::write(&extensionless, "x").unwrap();
    assert_eq!(
        locate_executable_in(std::path::Path::new("plaincentral"), Some(&path_var)),
        None,
        "std probes only plaincentral.exe, which does not exist"
    );

    let found = dir.path().join("central.exe");
    std::fs::write(&found, "x").unwrap();
    assert_eq!(
        locate_executable_in(std::path::Path::new("central"), Some(&path_var)),
        Some(found)
    );
}

#[cfg(windows)]
#[test]
fn explicit_path_gets_the_exe_suffix_like_std() {
    // For an explicit path std DOES append `.exe`; not probing it would warn about a central that
    // actually runs.
    let dir = tempfile::tempdir().unwrap();
    let with_exe = dir.path().join("central.exe");
    std::fs::write(&with_exe, "x").unwrap();
    let explicit = dir.path().join("central"); // no `.exe`, does not exist as-is
    assert_eq!(locate_executable_in(&explicit, None), Some(with_exe));
}

#[test]
fn absence_is_reported_by_the_os_not_a_pre_check() {
    // A bare name that nothing on PATH satisfies. `stop` reports it as NotInstalled (there is
    // nothing to stop, which is not a failure); `run_status_classified` errors, naming the binary
    // and PATH so the user knows where it was looked for.
    let missing = std::path::Path::new("poverty-mode-no-such-central-xyz");

    assert_eq!(stop(missing).unwrap(), StopOutcome::NotInstalled);

    let status_err = format!("{:#}", run_status_classified(missing).unwrap_err());
    assert!(status_err.contains("no-such-central-xyz"), "{status_err}");
    assert!(status_err.contains("PATH"), "{status_err}");

    // And presence is answered by an actual spawn, never by an is_file walk.
    assert_eq!(probe_presence(missing), Presence::Absent);
}
