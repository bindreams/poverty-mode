//! pino: prompt-cache breakpoint injection. M1 ships the settings struct and a
//! fail-loud transform stub (R9); the real cache-injection logic lands in M4.

use serde::{Deserialize, Serialize};

use crate::proxy::BodyTransform;

/// Rolling-tail cache TTL. Serializes to the short forms `"5m"` / `"1h"`.
///
/// Deserialization is **lenient** (R22/R23k — Node `parseTailTtl` parity):
/// `"5m"` → `FiveMin`, `"1h"` → `OneHour`, and ANY other string falls back to
/// `FiveMin` with a logged `warn!` rather than erroring. M2's config tests
/// assert the fallback; M4 relies on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum TailTtl {
    #[serde(rename = "5m")]
    FiveMin,
    #[serde(rename = "1h")]
    OneHour,
}

impl<'de> Deserialize<'de> for TailTtl {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        match raw.as_str() {
            "5m" => Ok(TailTtl::FiveMin),
            "1h" => Ok(TailTtl::OneHour),
            other => {
                tracing::warn!(
                    value = other,
                    "invalid tail_ttl; falling back to 5m (valid values: 5m, 1h)"
                );
                Ok(TailTtl::FiveMin)
            }
        }
    }
}

/// pino transform settings (config + CLI). FILLED behavior lands in M4; this
/// shape is never redefined.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PinoSettings {
    /// Enable cache-breakpoint injection.
    pub auto_cache: bool,
    /// Rolling-tail cache TTL.
    pub tail_ttl: TailTtl,
    /// Tool names to drop from `tools` and scrub from reminders.
    pub drop_tools: Vec<String>,
    /// Strip ANSI escape sequences from text content.
    pub strip_ansi: bool,
    /// Override the requested model identifier.
    pub model_override: Option<String>,
}

/// The pino body transform. M1 stub: `transform` fails loud; `apply_headers`
/// uses the trait default (no-op). M4 implements both (the `apply_headers`
/// override calls `ensure_beta_header` when `auto_cache`, per R6).
pub struct PinoTransform {
    /// The settings governing this transform.
    pub settings: PinoSettings,
}

impl BodyTransform for PinoTransform {
    fn transform(&self, _body: &mut serde_json::Value) -> anyhow::Result<()> {
        anyhow::bail!("pino transform not implemented")
    }
}

#[cfg(test)]
#[path = "pino_tests.rs"]
mod pino_tests;
