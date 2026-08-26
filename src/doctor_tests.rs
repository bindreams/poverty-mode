use super::*;
use serde_json::json;

fn layer(name: SettingsLayer, value: serde_json::Value) -> SettingsSource {
    SettingsSource {
        layer: name,
        json: Some(value),
    }
}

#[test]
fn no_findings_when_no_base_url_anywhere() {
    let sources = vec![
        layer(SettingsLayer::UserSettings, json!({"theme": "dark"})),
        SettingsSource {
            layer: SettingsLayer::ProjectSettings,
            json: None,
        },
    ];
    let findings = analyze_base_url(&sources, "http://127.0.0.1:40001");
    assert!(findings.is_empty(), "got: {findings:?}");
}

#[test]
fn detects_top_level_base_url_conflict_in_user_settings() {
    let sources = vec![layer(
        SettingsLayer::UserSettings,
        json!({"ANTHROPIC_BASE_URL": "https://corp.example/api"}),
    )];
    let findings = analyze_base_url(&sources, "http://127.0.0.1:40001");
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].domain, FindingDomain::Settings);
    assert_eq!(findings[0].layer, Some(SettingsLayer::UserSettings));
    assert_eq!(findings[0].severity, Severity::Warn);
    assert_eq!(findings[0].found_value.as_deref(), Some("https://corp.example/api"));
    assert!(findings[0].message.contains("ANTHROPIC_BASE_URL"));
}

#[test]
fn detects_env_block_base_url_conflict_in_project_settings() {
    let sources = vec![layer(
        SettingsLayer::ProjectSettings,
        json!({"env": {"ANTHROPIC_BASE_URL": "https://other.example"}}),
    )];
    let findings = analyze_base_url(&sources, "http://127.0.0.1:40001");
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].layer, Some(SettingsLayer::ProjectSettings));
    assert_eq!(findings[0].found_value.as_deref(), Some("https://other.example"));
}

#[test]
fn managed_layer_is_error_severity_not_warn() {
    let sources = vec![layer(
        SettingsLayer::Managed,
        json!({"env": {"ANTHROPIC_BASE_URL": "https://locked.example"}}),
    )];
    let findings = analyze_base_url(&sources, "http://127.0.0.1:40001");
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].layer, Some(SettingsLayer::Managed));
    assert_eq!(findings[0].severity, Severity::Error);
    assert!(
        findings[0].message.to_lowercase().contains("managed"),
        "got: {}",
        findings[0].message
    );
}

#[test]
fn matching_value_is_not_a_conflict() {
    // A layer already pointing at OUR injected URL is harmless.
    let ours = "http://127.0.0.1:40001";
    let sources = vec![layer(
        SettingsLayer::UserSettings,
        json!({"env": {"ANTHROPIC_BASE_URL": ours}}),
    )];
    let findings = analyze_base_url(&sources, ours);
    assert!(findings.is_empty(), "got: {findings:?}");
}

#[test]
fn both_top_level_and_env_block_yield_two_findings() {
    let sources = vec![layer(
        SettingsLayer::UserSettings,
        json!({
            "ANTHROPIC_BASE_URL": "https://a.example",
            "env": {"ANTHROPIC_BASE_URL": "https://b.example"}
        }),
    )];
    let findings = analyze_base_url(&sources, "http://127.0.0.1:40001");
    assert_eq!(findings.len(), 2);
    let values: Vec<&str> = findings.iter().filter_map(|f| f.found_value.as_deref()).collect();
    assert!(values.contains(&"https://a.example"));
    assert!(values.contains(&"https://b.example"));
}

#[test]
fn null_or_missing_json_layer_yields_nothing() {
    let sources = vec![SettingsSource {
        layer: SettingsLayer::UserSettings,
        json: None,
    }];
    let findings = analyze_base_url(&sources, "http://127.0.0.1:40001");
    assert!(findings.is_empty());
}

#[test]
fn supported_targets_are_recognized() {
    assert!(target_is_supported("windows", "x86_64"));
    assert!(target_is_supported("macos", "x86_64"));
    assert!(target_is_supported("macos", "aarch64"));
    assert!(target_is_supported("linux", "x86_64"));
    assert!(target_is_supported("linux", "aarch64"));
}

#[test]
fn unsupported_targets_are_rejected() {
    assert!(!target_is_supported("windows", "aarch64"));
    assert!(!target_is_supported("freebsd", "x86_64"));
    assert!(!target_is_supported("linux", "riscv64"));
}

/// An absolute path guaranteed to exist and be executable on every platform: the
/// running test binary itself. `central::locate_executable` resolves an explicit path verbatim when
/// it is a file, so this is a hermetic stand-in for a resolvable central binary (no dependency on
/// ambient `$PATH`).
fn resolvable_executable() -> std::path::PathBuf {
    std::env::current_exe().expect("test binary path")
}

#[test]
fn resolvable_central_is_quiet_on_every_target() {
    // Central readiness is a PATH lookup now, so it no longer varies by target.
    let exe = resolvable_executable();
    let findings = analyze_central(Some(exe.to_str().unwrap()));
    assert!(findings.is_empty(), "a resolvable central is quiet: {findings:?}");
}

#[test]
fn external_mode_warns_when_executable_unresolvable() {
    // A binary that resolves on neither PATH nor the filesystem warns. Use an explicit
    // non-existent path so the test never depends on ambient $PATH.
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("nonexistent-central");
    let findings = analyze_central(Some(missing.to_str().unwrap()));
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].domain, FindingDomain::Toolchain);
    assert!(findings[0].layer.is_none());
    assert_eq!(findings[0].severity, Severity::Warn);
    assert!(findings[0].message.contains("not found on PATH or filesystem"));
}

#[test]
fn assemble_findings_merges_toolchain_and_central() {
    // The pure assembly seam used by `run_doctor`: no settings layers, an unsupported target, and
    // an unresolvable central -> both the unsupported-target Error and the central Warn surface.
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("nonexistent-central");
    let findings = assemble_findings(
        &[],
        "windows",
        "aarch64",
        Ok(Some(missing.to_string_lossy().into_owned())),
    );
    assert!(findings
        .iter()
        .any(|f| f.severity == Severity::Error && f.message.to_lowercase().contains("unsupported")));
    assert!(findings.iter().any(|f| f.message.contains("not found on PATH")));
}

#[test]
fn assemble_findings_resolvable_central_adds_no_central_warning() {
    // On the windows/aarch64 hole the unsupported-target finding still surfaces, but a resolvable
    // central contributes nothing.
    let exe = resolvable_executable();
    let findings = assemble_findings(&[], "windows", "aarch64", Ok(Some(exe.to_string_lossy().into_owned())));
    assert!(findings
        .iter()
        .any(|f| f.severity == Severity::Error && f.message.to_lowercase().contains("unsupported")));
    assert!(!findings.iter().any(|f| f.message.contains("not found on PATH")));
}

#[test]
fn assemble_findings_warns_on_unparseable_config() {
    // A config that could not be loaded must NOT abort doctor: it surfaces a Warn
    // finding describing the failure and skips the central readiness check (no
    // "no jbcentral asset" / executable findings), while toolchain findings still
    // surface for a supported target (none here).
    let findings = assemble_findings(
        &[],
        "linux",
        "x86_64",
        Err("parsing config /x/poverty-mode.yaml: bad".to_string()),
    );
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].domain, FindingDomain::Toolchain);
    assert_eq!(findings[0].severity, Severity::Warn);
    assert!(findings[0].message.contains("could not read config"));
    assert!(findings[0].message.contains("parsing config"));
    assert!(!findings[0].message.contains("no jbcentral asset"));
}

#[test]
fn run_doctor_uses_on_disk_config_without_creating_one() {
    // Hermetic: isolate the config home so `run_doctor`'s `load_or_default` reads
    // only our temp dir. With no config file present, doctor must NOT create one
    // (read-only contract), and must complete without panicking.
    let guard = crate::test_support::ConfigHomeGuard::new();
    let ok = run_doctor().expect("run_doctor must not error");
    let _ = ok; // exit code depends on host target; we only assert it runs.
    assert!(!guard.config_file().exists(), "doctor must not create a config file");
}

#[test]
fn run_doctor_does_not_abort_on_unparseable_config() {
    // Regression: doctor reads the config READ-ONLY to resolve the central source.
    // A malformed/incompatible on-disk config must NOT abort the whole command --
    // doctor degrades to a Warn finding and still reports everything else.
    let guard = crate::test_support::ConfigHomeGuard::new();
    std::fs::write(guard.config_file(), "not: [valid, config\n").unwrap();
    let ok = run_doctor().expect("run_doctor must not error on a broken config");
    let _ = ok;
}

#[test]
fn toolchain_finding_emitted_for_unsupported_target() {
    // The central-asset check now lives in `analyze_central` (see
    // `download_mode_warns_on_missing_asset`), so the unsupported target yields
    // exactly the one unsupported-build-target finding.
    let findings = analyze_toolchain("windows", "aarch64");
    assert_eq!(findings.len(), 1);
    // Every toolchain finding is domain Toolchain with no settings layer.
    assert!(findings
        .iter()
        .all(|f| f.domain == FindingDomain::Toolchain && f.layer.is_none()));
    // The sole finding: unsupported build target (Error).
    assert!(findings
        .iter()
        .any(|f| f.severity == Severity::Error && f.message.to_lowercase().contains("unsupported")));
}

#[test]
fn toolchain_finding_empty_for_supported_target_with_asset() {
    let findings = analyze_toolchain("linux", "x86_64");
    assert!(findings.is_empty(), "got: {findings:?}");
}

#[test]
fn render_findings_groups_errors_before_warnings_and_reports_ok() {
    let none: Vec<Finding> = vec![];
    let out = render_findings(&none);
    assert!(out.contains("no problems detected"), "got: {out}");

    let some = vec![
        Finding {
            domain: FindingDomain::Settings,
            layer: Some(SettingsLayer::UserSettings),
            severity: Severity::Warn,
            message: "warn msg".to_string(),
            found_value: Some("v".to_string()),
        },
        Finding {
            domain: FindingDomain::Toolchain,
            layer: None,
            severity: Severity::Error,
            message: "error msg".to_string(),
            found_value: None,
        },
    ];
    let out = render_findings(&some);
    let err_pos = out.find("error msg").unwrap();
    let warn_pos = out.find("warn msg").unwrap();
    assert!(err_pos < warn_pos, "errors must come first: {out}");
    assert!(out.contains("ERROR"));
    assert!(out.contains("WARN"));
}

// #34: central readiness is a PATH lookup, not an asset check.
#[test]
fn doctor_warns_once_when_central_is_missing() {
    let findings = analyze_central(Some("poverty-mode-no-such-central-xyz"));
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0].severity, Severity::Warn);
    assert_eq!(findings[0].domain, FindingDomain::Toolchain);
    assert!(findings[0].message.contains("no-such-central-xyz"), "{findings:?}");
}

#[test]
fn doctor_is_silent_when_central_resolves() {
    // An explicit path is checked on disk, so this needs no PATH manipulation.
    let dir = tempfile::tempdir().unwrap();
    let exe = dir.path().join("central");
    std::fs::write(&exe, "x").unwrap();
    assert!(analyze_central(Some(exe.to_str().unwrap())).is_empty());
}
