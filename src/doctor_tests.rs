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
    assert_eq!(
        findings[0].found_value.as_deref(),
        Some("https://corp.example/api")
    );
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
    assert_eq!(
        findings[0].found_value.as_deref(),
        Some("https://other.example")
    );
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
    let values: Vec<&str> = findings
        .iter()
        .filter_map(|f| f.found_value.as_deref())
        .collect();
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
