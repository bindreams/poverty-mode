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
    let up = central_wire_upstream(&info);
    assert_eq!(
        up.url.as_str(),
        "http://127.0.0.1:19516/wire/abc123/claude-code/anthropic"
    );
    // The wire path is carried as the upstream's path prefix (no trailing slash).
    assert_eq!(up.path_prefix(), "/wire/abc123/claude-code/anthropic");
    assert_eq!(up.host_header(), "127.0.0.1:19516");
}
