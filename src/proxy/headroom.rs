//! headroom: context compression via the byte-surgical live-zone dispatcher.

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
    // FIX-B: the byte-fidelity seam. Run the live-zone dispatcher on the ORIGINAL
    // request bytes and return its byte-surgical output VERBATIM (never
    // round-tripped through `serde_json::Value`), so the cache-hot zone
    // (system/tools/history/thinking) the engine forwards is byte-for-byte
    // identical to what the client sent and the prompt cache survives.
    fn transform_bytes(&self, raw: &[u8]) -> anyhow::Result<Option<Vec<u8>>> {
        use headroom_core::transforms::{compress_anthropic_live_zone, AuthMode, LiveZoneOutcome};

        // Disabled => NO change: the engine forwards the original bytes verbatim.
        if !self.settings.compression {
            return Ok(None);
        }

        // Tokenizer gate: read the upstream model from `body["model"]` (FIX-B);
        // fall back to the dispatcher's DEFAULT_MODEL only when the body carries
        // no string `model`. Parsing the model field does NOT round-trip the body
        // through `Value` for forwarding — the bytes we forward are the
        // dispatcher's byte-surgical output (or the original on NoChange).
        let model_owned: Option<String> = serde_json::from_slice::<serde_json::Value>(raw)
            .ok()
            .and_then(|v| v.get("model").and_then(|m| m.as_str()).map(str::to_owned));
        let model: &str = model_owned
            .as_deref()
            .unwrap_or(headroom_core::transforms::live_zone::DEFAULT_MODEL);

        // frozen_message_count = 0, auth_mode = Payg: both are verified-inert for
        // the cache-hot guarantee. frozen=0 sets the floor to 0 but the
        // dispatcher's ceiling is unconditionally the latest user message, and
        // HOT_ZONE_BLOCK_TYPES (tool_use/thinking/redacted_thinking/compaction)
        // plus the top-level system/tools fields are never rewritten. AuthMode is
        // documented "taken in B3 but unused"; Payg is the stable default.
        let outcome = compress_anthropic_live_zone(raw, 0, AuthMode::Payg, model)
            .map_err(|e| anyhow::anyhow!("headroom live-zone dispatch failed: {e}"))?;

        match outcome {
            // Nothing shrank: None => the engine forwards the original bytes.
            LiveZoneOutcome::NoChange { .. } => Ok(None),
            // `new_body.get()` is the dispatcher's byte-surgical JSON document:
            // only the compressed block byte-ranges changed; the cache-hot zone
            // is copied verbatim from `raw`. Forward those bytes exactly.
            LiveZoneOutcome::Modified { new_body, .. } => {
                Ok(Some(new_body.get().as_bytes().to_vec()))
            }
        }
    }

    // Legacy `Value`-in-place hook (in-process callers / unit tests). Delegates
    // to the byte-faithful `transform_bytes` and re-parses on a change. NOT on
    // the engine forward path (the engine calls `transform_bytes` directly), so
    // this re-parse never re-canonicalizes the bytes the upstream receives.
    fn transform(&self, body: &mut serde_json::Value) -> anyhow::Result<()> {
        let raw = serde_json::to_vec(body)?;
        if let Some(bytes) = self.transform_bytes(&raw)? {
            *body = serde_json::from_slice(&bytes)
                .map_err(|e| anyhow::anyhow!("headroom produced invalid JSON: {e}"))?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "headroom_tests.rs"]
mod headroom_tests;
