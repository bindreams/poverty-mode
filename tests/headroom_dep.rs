//! Confirms the `headroom-core` git dependency links and exposes
//! exactly the live-zone surface poverty-mode depends on -- and nothing that
//! would pull an ONNX runtime. This is a compile + link test: if the dep is
//! missing, the feature is wrong, or the re-exports moved, the crate fails to
//! build.

use headroom_core::transforms::live_zone::DEFAULT_MODEL;
use headroom_core::transforms::{compress_anthropic_live_zone, AuthMode, LiveZoneOutcome};

#[test]
fn trimmed_headroom_core_exposes_live_zone_dispatcher() {
    // A minimal valid Anthropic body with an empty messages array. The
    // dispatcher must return Ok(NoChange) (nothing to compress), proving the
    // function is linked and callable with the verified signature.
    let body = br#"{"model":"claude-3-5-sonnet-20241022","messages":[]}"#;
    let outcome = compress_anthropic_live_zone(body, 0, AuthMode::Payg, DEFAULT_MODEL)
        .expect("dispatcher returns Ok on a valid empty-messages body");
    match outcome {
        LiveZoneOutcome::NoChange { .. } => {}
        LiveZoneOutcome::Modified { .. } => panic!("empty messages must not be modified"),
    }
}

/// The resolved dependency graph must never contain the ONNX / embedding
/// stack. Guards against `default-features = false` being dropped from the
/// `headroom-core` dependency (which would re-enable upstream's default `ml`
/// feature) or another crate unifying it back on.
#[test]
fn lockfile_has_no_onnx_or_embedding_crates() {
    let lock =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.lock")).expect("Cargo.lock is readable");
    let forbidden = ["ort", "ort-sys", "fastembed", "magika"];
    let present: Vec<&str> = lock
        .lines()
        .filter_map(|l| l.strip_prefix("name = \"")?.strip_suffix('"'))
        .filter(|n| forbidden.contains(n))
        .collect();
    assert!(present.is_empty(), "ONNX/embedding crates in Cargo.lock: {present:?}");
}
