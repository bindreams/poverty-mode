//! Validates the release workflow YAML structurally.

use std::path::Path;

fn rel_text() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(".github")
        .join("workflows")
        .join("release.yaml");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

fn workflow() -> serde_yaml::Value {
    serde_yaml::from_str(&rel_text()).expect("release.yaml must be valid YAML")
}

#[test]
fn release_triggers_on_version_tags() {
    let wf = workflow();
    let on = wf
        .get("on")
        .or_else(|| wf.get(serde_yaml::Value::Bool(true)))
        .expect("on: key");
    let tags = on
        .get("push")
        .and_then(|p| p.get("tags"))
        .and_then(|t| t.as_sequence())
        .expect("push.tags sequence");
    let tag_strs: Vec<String> = tags.iter().filter_map(|t| t.as_str()).map(|s| s.to_string()).collect();
    assert!(
        tag_strs.iter().any(|t| t.starts_with('v')),
        "release must trigger on v* tags; got {tag_strs:?}"
    );
}

#[test]
fn release_builds_four_targets_and_uploads() {
    let wf = workflow();
    let jobs = wf.get("jobs").expect("jobs");
    let rel = jobs.get("release").expect("release job");
    let matrix = rel
        .get("strategy")
        .and_then(|s| s.get("matrix"))
        .and_then(|m| m.get("include"))
        .and_then(|i| i.as_sequence())
        .expect("matrix.include sequence");

    let targets: Vec<String> = matrix
        .iter()
        .filter_map(|e| e.get("target"))
        .filter_map(|t| t.as_str())
        .map(|s| s.to_string())
        .collect();
    for expected in [
        "x86_64-pc-windows-msvc",
        "aarch64-apple-darwin",
        "x86_64-unknown-linux-gnu",
        "aarch64-unknown-linux-gnu",
    ] {
        assert!(targets.iter().any(|t| t == expected), "missing {expected}");
    }
    assert!(
        !targets.iter().any(|t| t == "x86_64-apple-darwin"),
        "Intel macOS dropped per the four-platform decision; got {targets:?}"
    );
    assert_eq!(targets.len(), 4, "exactly four release targets; got {targets:?}");

    assert!(
        rel_text().contains("softprops/action-gh-release"),
        "release must upload assets"
    );
}

#[test]
fn release_has_write_contents_permission() {
    let wf = workflow();
    let perms = wf.get("permissions").expect("permissions");
    assert_eq!(
        perms.get("contents").and_then(|v| v.as_str()),
        Some("write"),
        "publishing a release needs contents: write"
    );
}

#[test]
fn release_uses_uniform_shasum_and_does_not_silence_license_copy() {
    let text = rel_text();
    // Uniform checksum tool across OSes; no certutil divergence.
    assert!(
        text.contains("shasum -a 256"),
        "checksums must use shasum -a 256 on all platforms"
    );
    assert!(
        !text.contains("certutil"),
        "certutil produces a non-uniform sidecar; do not use it"
    );
    // LICENSE/README copy must fail loudly if missing -- no `|| true`.
    assert!(
        !text.contains("|| true"),
        "missing LICENSE/README must fail the release, not be silenced"
    );
    assert!(
        text.contains("LICENSE-MIT") && text.contains("LICENSE-APACHE"),
        "both license files must be packaged"
    );
}
