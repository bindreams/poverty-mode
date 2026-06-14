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

/// The headroom body transform. M1 stub: `transform` fails loud. M5 implements
/// it (running the CPU-heavy compress via `spawn_blocking`, per R20).
pub struct HeadroomTransform {
    /// The settings governing this transform.
    pub settings: HeadroomSettings,
}

impl BodyTransform for HeadroomTransform {
    fn transform(&self, body: &mut serde_json::Value) -> anyhow::Result<()> {
        if !self.settings.compression {
            // Disabled: byte-faithful passthrough. Do not touch `body`.
            let _ = body;
            return Ok(());
        }
        // Enabled path is implemented in Task M5.3 (calls
        // headroom_core::transforms::compress_anthropic_live_zone). Until then
        // it stays fail-loud rather than a silent no-op (R9): a config that
        // enables compression but hits this stub must error, never silently
        // forward uncompressed bytes as if compression "succeeded".
        anyhow::bail!("headroom compression enabled but transform not implemented");
    }
}

#[cfg(test)]
#[path = "headroom_tests.rs"]
mod headroom_tests;
