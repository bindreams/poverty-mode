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
    let up = central_wire_upstream(&info);
    assert_eq!(
        up.url.as_str(),
        "http://127.0.0.1:19516/wire/a%23b%3Fc%2Fd%20e%26f%25g/claude-code/anthropic"
    );
    // It stays one segment: no fragment, no query, no extra path separators.
    assert_eq!(up.url.fragment(), None);
    assert_eq!(up.url.query(), None);
    assert_eq!(
        up.path_prefix(),
        "/wire/a%23b%3Fc%2Fd%20e%26f%25g/claude-code/anthropic"
    );
    assert_eq!(up.host_header(), "127.0.0.1:19516");
}
