//! Confirms the trimmed `headroom-core` vendored crate links and exposes
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
