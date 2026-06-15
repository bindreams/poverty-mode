//! Validates the CI workflow YAML structurally (does not run Actions).

use std::path::Path;

fn ci_text() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(".github")
        .join("workflows")
        .join("ci.yaml");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

fn workflow() -> serde_yaml::Value {
    serde_yaml::from_str(&ci_text()).expect("ci.yaml must be valid YAML")
}

#[test]
fn ci_build_test_job_covers_four_docker_style_platforms() {
    let wf = workflow();
    let jobs = wf.get("jobs").expect("jobs key");
    let bt = jobs.get("build-test").expect("build-test job");
    let matrix = bt
        .get("strategy")
        .and_then(|s| s.get("matrix"))
        .and_then(|m| m.get("include"))
        .expect("matrix.include")
        .as_sequence()
        .expect("include is a sequence");

    let platforms: Vec<(String, String)> = matrix
        .iter()
        .filter_map(|e| {
            let os = e.get("os")?.as_str()?.to_string();
            let arch = e.get("arch")?.as_str()?.to_string();
            Some((os, arch))
        })
        .collect();

    for (os, arch) in [
        ("windows", "amd64"),
        ("darwin", "arm64"),
        ("linux", "amd64"),
        ("linux", "arm64"),
    ] {
        assert!(
            platforms.iter().any(|(o, a)| o == os && a == arch),
            "missing platform {os}/{arch}; got {platforms:?}"
        );
    }
    assert_eq!(platforms.len(), 4, "exactly four platforms; got {platforms:?}");
}

#[test]
fn ci_runs_cargo_test() {
    assert!(ci_text().contains("cargo test"), "CI must run cargo test");
}

#[test]
fn ci_runs_lint_via_prek_not_per_platform_fmt() {
    let wf = workflow();
    let jobs = wf.get("jobs").expect("jobs key");
    let lint = jobs.get("lint").expect("dedicated lint job");
    let text = serde_yaml::to_string(lint).unwrap();
    assert!(
        text.contains("prek run"),
        "lint job must run prek (which owns cargo fmt + the other linters)"
    );
    // Linting runs once in the lint job, never per matrix platform.
    let bt = jobs.get("build-test").expect("build-test job");
    let bt_text = serde_yaml::to_string(bt).unwrap();
    assert!(
        !bt_text.contains("cargo fmt") && !bt_text.contains("prek"),
        "fmt/prek must not run per platform"
    );
}

#[test]
fn ci_triggers_on_push_and_pull_request() {
    let wf = workflow();
    // `on` is a reserved YAML key serde_yaml may parse as the boolean key `true`.
    let on = wf
        .get("on")
        .or_else(|| wf.get(serde_yaml::Value::Bool(true)))
        .expect("on: key");
    assert!(on.get("push").is_some(), "must trigger on push");
    assert!(on.get("pull_request").is_some(), "must trigger on pull_request");
}

#[test]
fn ci_never_runs_ignored_tests_or_central_login() {
    // R7: CI must NOT run --include-ignored, must NOT provision jbcentral, and must
    // NOT attempt a central login (no non-interactive --token form exists).
    let text = ci_text();
    assert!(!text.contains("--include-ignored"), "R7: CI must not run ignored tests");
    assert!(!text.contains("central login"), "R7: CI must not attempt central login");
    assert!(
        !text.to_lowercase().contains("jbcentral_token"),
        "R7: CI must not reference a jbcentral login token"
    );
    assert!(
        !text.contains("central: true") && !text.contains("central: false"),
        "R7: the unused `central` matrix column must be removed"
    );
}
