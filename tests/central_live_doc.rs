//! Non-ignored guard (R7): EMPIRICAL_GATES.md must document the Central live suite, its run command,
//! and the JetBrains AI Pro prerequisite. Mirrors M7's include_str! agent-gate assertion.

const GATES: &str = include_str!("EMPIRICAL_GATES.md");

#[test]
fn documents_central_live_suite() {
    assert!(
        GATES.contains("Central live suite"),
        "missing 'Central live suite' heading"
    );
    assert!(
        GATES.contains("central_live"),
        "must reference the central_live test target"
    );
    assert!(
        GATES.contains("--ignored"),
        "must show the --ignored run invocation"
    );
    assert!(
        GATES.contains("AI Pro"),
        "must document the JetBrains AI Pro login prerequisite"
    );
}
