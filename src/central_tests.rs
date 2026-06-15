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

// R20: the secret is read from an external file (`~/.wire/config.json`) and may
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
    assert_eq!(
        up.url.as_str(),
        "http://127.0.0.1:19516/wire/a%23b%3Fc%2Fd%20e%26f%25g"
    );
    // It stays one segment: no fragment, no query, no extra path separators.
    assert_eq!(up.url.fragment(), None);
    assert_eq!(up.url.query(), None);
    assert_eq!(up.path_prefix(), "/wire/a%23b%3Fc%2Fd%20e%26f%25g");
    assert_eq!(up.host_header(), "127.0.0.1:19516");
}

// M8.5: central constants (R4) + `~/.wire/config.json` -> CentralInfo parsing.

#[test]
fn constants_are_default_version_and_tool_dir() {
    assert_eq!(DEFAULT_JBCENTRAL_VERSION, "0.2.9");
    assert_eq!(INSTALL_TOOL_DIR, "jbcentral");
}

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
    assert!(
        !msg.contains("TOPSECRET"),
        "error must not leak the secret: {msg}"
    );
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

// wire URL =====

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
    assert_eq!(
        central_wire_envelope_url(&info),
        "http://127.0.0.1:8080/wire/xyz"
    );
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
    assert!(
        up.url.query().is_none(),
        "secret must not leak into the query"
    );
    assert!(
        up.url.fragment().is_none(),
        "secret must not leak into the fragment"
    );
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

// version resolution (pure) =====

#[test]
fn pinned_version_uses_config_when_set() {
    assert_eq!(pinned_version(Some("1.2.3")), "1.2.3");
    assert_eq!(pinned_version(Some("  9.9.9  ")), "9.9.9"); // trimmed
}

#[test]
fn pinned_version_falls_back_to_default_when_unset_or_blank() {
    assert_eq!(pinned_version(None), DEFAULT_JBCENTRAL_VERSION);
    assert_eq!(pinned_version(Some("")), DEFAULT_JBCENTRAL_VERSION);
    assert_eq!(pinned_version(Some("   ")), DEFAULT_JBCENTRAL_VERSION);
}

#[test]
fn parse_version_txt_takes_first_nonblank_line() {
    assert_eq!(parse_version_txt("0.3.1\n").unwrap(), "0.3.1");
    assert_eq!(parse_version_txt("\n  0.3.2  \nextra\n").unwrap(), "0.3.2");
}

#[test]
fn parse_version_txt_rejects_empty_or_garbage() {
    assert!(parse_version_txt("").is_err());
    assert!(parse_version_txt("   \n  \n").is_err());
    // A line that does not look like a dotted version is rejected.
    assert!(parse_version_txt("not a version!").is_err());
}

#[test]
fn latest_version_url_targets_latest_version_txt() {
    assert_eq!(
        latest_version_url(),
        "https://jetbrains-central-cli.s3.eu-west-1.amazonaws.com/jbcentral/latest/version.txt"
    );
}

// install layout =====

#[test]
fn binary_name_is_platform_specific() {
    let name = jbcentral_binary_name();
    if cfg!(windows) {
        assert_eq!(name, "jbcentral.exe");
    } else {
        assert_eq!(name, "jbcentral");
    }
}

#[test]
fn install_dir_uses_shared_tool_dir_constant() {
    let root = std::path::Path::new("/tmp/pm-cache");
    let dir = install_dir_in(root, "0.2.9");
    let expected = root.join("bin").join(INSTALL_TOOL_DIR).join("0.2.9");
    assert_eq!(dir, expected);
    // The tool dir is the single shared constant — never "central".
    assert!(dir.to_string_lossy().contains("jbcentral"));
    assert!(!dir
        .to_string_lossy()
        .contains(&format!("bin{}central", std::path::MAIN_SEPARATOR)));
}

#[test]
fn is_installed_in_false_when_absent_true_when_flat() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    assert!(!is_installed_in(root, "0.2.9"));

    // Flat layout: binary directly under the version dir.
    let dir = install_dir_in(root, "0.2.9");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(jbcentral_binary_name()), b"fake").unwrap();
    assert!(is_installed_in(root, "0.2.9"));
}

#[test]
fn installed_binary_path_in_resolves_flat_layout() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let dir = install_dir_in(root, "0.2.9");
    std::fs::create_dir_all(&dir).unwrap();
    let flat = dir.join(jbcentral_binary_name());
    std::fs::write(&flat, b"fake").unwrap();

    let resolved = installed_binary_path_in(root, "0.2.9").unwrap();
    assert_eq!(resolved, flat);
}

#[test]
fn installed_binary_path_in_resolves_nested_layout() {
    // Nested layout: binary one dir deep (some archives nest under jbcentral-<ver>/).
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let dir = install_dir_in(root, "0.2.9");
    let nested = dir.join("jbcentral-0.2.9");
    std::fs::create_dir_all(&nested).unwrap();
    let bin = nested.join(jbcentral_binary_name());
    std::fs::write(&bin, b"fake").unwrap();

    // is_installed_in must ALSO see the nested binary (consistency with status/clean in M10).
    assert!(is_installed_in(root, "0.2.9"));
    let resolved = installed_binary_path_in(root, "0.2.9").unwrap();
    assert_eq!(resolved, bin);
}

#[test]
fn installed_binary_path_in_none_when_dir_missing() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(installed_binary_path_in(tmp.path(), "0.2.9").is_none());
}

// login state classification =====

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

#[test]
fn run_status_text_errors_when_binary_is_missing() {
    // R23c name/signature: run_status_text(&Path) -> anyhow::Result<String>, runs `<bin> status`.
    // A non-existent binary path must surface a spawn error (never panic, never silent empty string).
    let missing = std::path::Path::new("/nonexistent/pm-jbcentral-does-not-exist");
    let err = run_status_text(missing).unwrap_err();
    assert!(
        err.to_string().contains("status"),
        "error should name the failed `status` invocation: {err}"
    );
}

/// Write a fake `jbcentral` into `dir` that prints `stdout`, exits with `code`, and ignores
/// its arguments. Cross-platform: a `.bat` on Windows (executed via the full path) and a
/// `chmod +x` shell script elsewhere. Returns the executable's path.
fn write_fake_jbcentral(dir: &std::path::Path, stdout: &str, code: i32) -> std::path::PathBuf {
    let bin = if cfg!(windows) {
        let p = dir.join("jbcentral.bat");
        // `@echo off` so the command line itself is not echoed into stdout.
        std::fs::write(
            &p,
            format!("@echo off\r\necho {stdout}\r\nexit /b {code}\r\n"),
        )
        .unwrap();
        p
    } else {
        let p = dir.join("jbcentral");
        std::fs::write(&p, format!("#!/bin/sh\necho '{stdout}'\nexit {code}\n")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&p).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&p, perms).unwrap();
        }
        p
    };
    bin
}

#[test]
fn run_status_classified_reports_logged_in_when_status_exits_zero() {
    // Genuine wiring test (R20): the real exit code from `jbcentral status` must reach the
    // classifier. A logged-in central exits 0 with a "Logged in" banner and must classify as
    // LoggedIn -- NOT Unknown (which is what dropping the exit code would yield).
    let tmp = tempfile::tempdir().unwrap();
    let bin = write_fake_jbcentral(tmp.path(), "Logged in as user@example.com", 0);
    let state = run_status_classified(&bin).unwrap();
    assert_eq!(
        state,
        CentralLoginState::LoggedIn,
        "exit 0 + banner => logged in"
    );
}

#[test]
fn run_status_classified_reports_logged_out_on_nonzero_exit() {
    // A logged-out central exits non-zero; the real code must drive a LoggedOut classification.
    let tmp = tempfile::tempdir().unwrap();
    let bin = write_fake_jbcentral(tmp.path(), "not logged in; run jbcentral login", 1);
    let state = run_status_classified(&bin).unwrap();
    assert_eq!(
        state,
        CentralLoginState::LoggedOut,
        "non-zero exit => logged out"
    );
}

#[test]
fn run_status_classified_errors_when_binary_is_missing() {
    let missing = std::path::Path::new("/nonexistent/pm-jbcentral-does-not-exist");
    let err = run_status_classified(missing).unwrap_err();
    assert!(
        err.to_string().contains("status"),
        "error should name the failed `status` invocation: {err}"
    );
}

// start/health argv + env =====

#[test]
fn configure_argv_sets_analytics_off_and_threaded_pinned_version() {
    let argvs = configure_commands("0.3.7");
    assert!(
        argvs.iter().any(|a| a
            == &vec![
                "config".to_string(),
                "set".to_string(),
                "google-analytics".to_string(),
                "off".to_string(),
            ]),
        "missing analytics-off config: {argvs:?}"
    );
    // The pinned-version MUST be the threaded version, not a const default.
    assert!(
        argvs.iter().any(|a| a
            == &vec![
                "config".to_string(),
                "set".to_string(),
                "pinned-version".to_string(),
                "0.3.7".to_string(),
            ]),
        "missing/incorrect pinned-version config: {argvs:?}"
    );
}

#[test]
fn proxy_start_argv_is_proxy_start() {
    assert_eq!(
        proxy_start_argv(),
        vec!["proxy".to_string(), "start".to_string()]
    );
}

#[test]
fn proxy_stop_argv_is_proxy_stop() {
    assert_eq!(
        proxy_stop_argv(),
        vec!["proxy".to_string(), "stop".to_string()]
    );
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

#[test]
fn proxy_pid_path_is_under_dot_wire() {
    let p = proxy_pid_path().unwrap();
    assert!(
        p.ends_with(std::path::Path::new(".wire").join("proxy.pid")),
        "{}",
        p.display()
    );
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

// `central status` rendering (FIX-D) =====

#[test]
fn render_central_command_status_installed_running_logged_in() {
    let status = CentralCommandStatus {
        versions: vec!["0.2.9".to_string(), "0.2.10".to_string()],
        running_port: Some(19516),
        login: CentralLoginState::LoggedIn,
    };
    let out = render_central_command_status(&status);
    assert!(out.contains("install: 0.2.9, 0.2.10"), "out:\n{out}");
    assert!(out.contains("state: running on port 19516"), "out:\n{out}");
    assert!(out.contains("login: logged in"), "out:\n{out}");
}

#[test]
fn render_central_command_status_not_installed_forces_unknown_login() {
    // With nothing installed, login is forced Unknown even if a stale state leaks in,
    // and the daemon reads as stopped.
    let status = CentralCommandStatus {
        versions: Vec::new(),
        running_port: None,
        login: CentralLoginState::LoggedIn,
    };
    let out = render_central_command_status(&status);
    assert!(out.contains("install: not installed"), "out:\n{out}");
    assert!(out.contains("state: stopped"), "out:\n{out}");
    assert!(out.contains("login: unknown"), "out:\n{out}");
}

#[test]
fn render_central_command_status_installed_stopped_logged_out() {
    let status = CentralCommandStatus {
        versions: vec!["0.2.9".to_string()],
        running_port: None,
        login: CentralLoginState::LoggedOut,
    };
    let out = render_central_command_status(&status);
    assert!(out.contains("install: 0.2.9"), "out:\n{out}");
    assert!(out.contains("state: stopped"), "out:\n{out}");
    assert!(out.contains("login: logged out"), "out:\n{out}");
}
