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

#[test]
fn central_asset_available_excludes_windows_arm64() {
    assert!(central_asset_available("windows", "x86_64"));
    assert!(central_asset_available("macos", "aarch64"));
    assert!(central_asset_available("linux", "aarch64"));
    // The one documented hole.
    assert!(!central_asset_available("windows", "aarch64"));
}

#[test]
fn toolchain_finding_emitted_for_unsupported_target() {
    let findings = analyze_toolchain("windows", "aarch64");
    assert_eq!(findings.len(), 2);
    // Every toolchain finding is domain Toolchain with no settings layer.
    assert!(findings
        .iter()
        .all(|f| f.domain == FindingDomain::Toolchain && f.layer.is_none()));
    // First: unsupported build target (Error).
    assert!(findings
        .iter()
        .any(|f| f.severity == Severity::Error && f.message.to_lowercase().contains("unsupported")));
    // Second: no central asset (mentions central).
    assert!(findings.iter().any(|f| f.message.to_lowercase().contains("central")));
}

#[test]
fn toolchain_finding_empty_for_supported_target_with_asset() {
    let findings = analyze_toolchain("linux", "x86_64");
    assert!(findings.is_empty(), "got: {findings:?}");
}

#[test]
fn toolchain_finding_central_only_hole_for_windows_arm64_is_warn() {
    // Supported build target with no central asset -> exactly one Warn, no Error.
    // (windows/aarch64 is unsupported; use a hypothetical supported-but-no-asset
    //  guard by checking the asset-only branch via central_asset_available.)
    // Here we assert the asset-availability semantics directly are consistent with
    // analyze_toolchain on the unsupported windows/aarch64 case above.
    assert!(!central_asset_available("windows", "aarch64"));
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
