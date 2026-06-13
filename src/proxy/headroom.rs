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
    fn transform(&self, _body: &mut serde_json::Value) -> anyhow::Result<()> {
        anyhow::bail!("headroom transform not implemented")
    }
}

#[cfg(test)]
#[path = "headroom_tests.rs"]
mod headroom_tests;
