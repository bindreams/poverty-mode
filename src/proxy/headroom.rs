//! headroom: context compression. M1 ships the settings struct and a fail-loud
//! transform stub (R9); the real compression logic lands in M5.

use serde::{Deserialize, Serialize};

use crate::proxy::BodyTransform;

/// headroom transform settings (config + CLI). FILLED behavior lands in M5; this
/// shape is never redefined.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeadroomSettings {
    /// Enable context compression. `false` => byte-faithful pass-through.
    pub compression: bool,
}

/// The headroom body transform. The engine runs this on a `spawn_blocking`
/// worker (R20/R23d), so the CPU-heavy compress never blocks the executor.
pub struct HeadroomTransform {
    /// The settings governing this transform.
    pub settings: HeadroomSettings,
}

impl BodyTransform for HeadroomTransform {
    fn transform(&self, body: &mut serde_json::Value) -> anyhow::Result<()> {
        use headroom_core::transforms::live_zone::DEFAULT_MODEL;
        use headroom_core::transforms::{compress_anthropic_live_zone, AuthMode, LiveZoneOutcome};

        if !self.settings.compression {
            let _ = body;
            return Ok(());
        }

        // Serialize the in-place body to the byte form the dispatcher consumes.
        // This serialize -> dispatch -> re-parse bridge is the ONLY place
        // poverty-mode round-trips numbers; it is byte/precision-faithful
        // because M1 pins serde_json with preserve_order + arbitrary_precision
        // (R2), and the fork copies untouched bytes verbatim via byte-range
        // surgery (VERIFIED API FACT 5) so it never collapses our numbers.
        let body_bytes = serde_json::to_vec(body)?;

        // frozen_message_count = 0, auth_mode = Payg: both are verified-inert
        // for the cache-hot guarantee (VERIFIED API FACT 7). frozen=0 sets the
        // floor to 0 (excludes nothing by floor) but the dispatcher's ceiling
        // is unconditionally the latest user message, and HOT_ZONE_BLOCK_TYPES
        // (tool_use/thinking/redacted_thinking/compaction) plus the top-level
        // system/tools fields are never rewritten. AuthMode is documented
        // "taken in B3 but unused"; Payg is the dispatcher's stable default.
        // model = DEFAULT_MODEL selects the Claude arithmetic estimator.
        let outcome = compress_anthropic_live_zone(&body_bytes, 0, AuthMode::Payg, DEFAULT_MODEL)
            .map_err(|e| anyhow::anyhow!("headroom live-zone dispatch failed: {e}"))?;

        match outcome {
            LiveZoneOutcome::NoChange { .. } => {
                // Nothing shrank: leave `body` exactly as it was (byte-equal).
                Ok(())
            }
            LiveZoneOutcome::Modified { new_body, .. } => {
                // Re-parse the dispatcher's rewritten JSON text and overwrite
                // the in-place body. `new_body.get()` is the JSON source of the
                // rewritten document.
                *body = serde_json::from_str(new_body.get())
                    .map_err(|e| anyhow::anyhow!("headroom produced invalid JSON: {e}"))?;
                Ok(())
            }
        }
    }
}

#[cfg(test)]
#[path = "headroom_tests.rs"]
mod headroom_tests;
