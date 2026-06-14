//! Validates the CI workflow YAML structurally (does not run Actions).

use std::path::Path;

fn ci_text() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(".github")
        .join("workflows")
        .join("ci.yml");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

fn workflow() -> serde_yaml::Value {
    serde_yaml::from_str(&ci_text()).expect("ci.yml must be valid YAML")
}

#[test]
fn ci_has_build_test_job_with_five_targets() {
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

    let targets: Vec<String> = matrix
        .iter()
        .filter_map(|e| e.get("target"))
        .filter_map(|t| t.as_str())
        .map(|s| s.to_string())
        .collect();

    for expected in [
        "x86_64-pc-windows-msvc",
        "x86_64-apple-darwin",
        "aarch64-apple-darwin",
        "x86_64-unknown-linux-gnu",
        "aarch64-unknown-linux-gnu",
    ] {
        assert!(
            targets.iter().any(|t| t == expected),
            "missing target {expected}; got {targets:?}"
        );
    }
}

#[test]
fn ci_runs_cargo_test() {
    assert!(ci_text().contains("cargo test"), "CI must run cargo test");
}

#[test]
fn ci_runs_fmt_check_in_a_single_lint_job() {
    let wf = workflow();
    let jobs = wf.get("jobs").expect("jobs key");
    let lint = jobs.get("lint").expect("dedicated lint job");
    let text = serde_yaml::to_string(lint).unwrap();
    assert!(
        text.contains("cargo fmt"),
        "lint job must run cargo fmt --check"
    );
    // fmt must NOT be duplicated inside the per-target build-test job.
    let bt = jobs.get("build-test").expect("build-test job");
    let bt_text = serde_yaml::to_string(bt).unwrap();
    assert!(
        !bt_text.contains("cargo fmt"),
        "fmt must run once in the lint job, not per matrix target"
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
    assert!(
        on.get("pull_request").is_some(),
        "must trigger on pull_request"
    );
}

#[test]
fn ci_never_runs_ignored_tests_or_central_login() {
    // R7: CI must NOT run --include-ignored, must NOT provision jbcentral, and must
    // NOT attempt a central login (no non-interactive --token form exists).
    let text = ci_text();
    assert!(
        !text.contains("--include-ignored"),
        "R7: CI must not run ignored tests"
    );
    assert!(
        !text.contains("central login"),
        "R7: CI must not attempt central login"
    );
    assert!(
        !text.to_lowercase().contains("jbcentral_token"),
        "R7: CI must not reference a jbcentral login token"
    );
    assert!(
        !text.contains("central: true") && !text.contains("central: false"),
        "R7: the unused `central` matrix column must be removed"
    );
}
