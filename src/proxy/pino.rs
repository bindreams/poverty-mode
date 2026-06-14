//! pino: prompt-cache breakpoint injection. M1 ships the settings struct and a
//! fail-loud transform stub (R9); the real cache-injection logic lands in M4.

use std::sync::OnceLock;

use anyhow::Result;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::proxy::BodyTransform;

/// Rolling-tail cache TTL. Serializes to the short forms `"5m"` / `"1h"`.
///
/// Deserialization is **lenient** (R22/R23k — Node `parseTailTtl` parity,
/// `reference/pino/src/config.js` lines 36-44): the raw value is trimmed and
/// lowercased, then `"5m"` → `FiveMin`, `"1h"` → `OneHour`, and ANY other
/// string falls back to `FiveMin` with a logged `warn!` rather than erroring.
/// M2's config tests assert the fallback; M4 relies on it.
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
        // Node parseTailTtl: String(raw).trim().toLowerCase() before matching.
        match raw.trim().to_ascii_lowercase().as_str() {
            "1h" => Ok(TailTtl::OneHour),
            // "5m" and every unrecognized value degrade to 5m (Node behavior).
            "5m" => Ok(TailTtl::FiveMin),
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

impl TailTtl {
    /// Wire value written into `cache_control.ttl`.
    pub fn as_str(&self) -> &'static str {
        match self {
            TailTtl::FiveMin => "5m",
            TailTtl::OneHour => "1h",
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

/// The Anthropic API allows at most 4 cache breakpoints per request.
pub const BREAKPOINT_CEILING: usize = 4;

/// Client-sent breakpoints on system blocks smaller than this waste a slot.
pub const MIN_SYSTEM_CACHE_CHARS: usize = 500;

/// `anthropic-beta` flag required for 1h cache TTL. This is an HTTP HEADER, not a
/// body field, so the engine path (apply_headers / ensure_beta_header) applies it,
/// never `transform`. Mirrors BETA_FLAG in reference/pino/src/config.js.
pub const BETA_FLAG: &str = "extended-cache-ttl-2025-04-11";

impl BodyTransform for PinoTransform {
    fn transform(&self, body: &mut Value) -> Result<()> {
        // Only object bodies are mutable in any meaningful way; non-objects pass through.
        if !body.is_object() {
            return Ok(());
        }
        // Operation order mirrors reference/pino/src/server.js lines 70-98:
        // 1. model override (replaces body.model + rewrites system self-references).
        if let Some(model) = self.settings.model_override.as_deref() {
            apply_model_override(body, model);
        }
        // 2. built-in default transform pipeline (drop_tools + reminder scrub +
        //    restructureV123 + strip_ansi), in the Node transforms/default.js order.
        apply_default_transform(body, &self.settings);
        // 3. auto-cache: inject breakpoints within the 4-cap, force 1h except tail.
        if self.settings.auto_cache {
            apply_auto_cache(body, self.settings.tail_ttl);
        }
        Ok(())
    }

    // R6: the engine calls this AFTER transform() and AFTER Host/Content-Length
    // rewrite, only on a transformed POST /v1/messages. pino applies the 1h-cache
    // beta header here (NOT in the body) when auto_cache is on. Wired in M4.10.
    fn apply_headers(&self, _headers: &mut http::HeaderMap) {
        // Implemented in Task M4.10.
    }
}

// --- pipeline stages (filled in by later tasks) -----

// Source model that Claude Code self-identifies as; rewritten to the override.
// Ported verbatim from reference/pino/src/model.js SOURCE_ID_PATTERN (the JS /g
// flag => replace_all). Note: no end-anchor; matches anywhere.
fn source_id_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"claude-opus-4-7(?:-\d{8})?").unwrap())
}

// SOURCE_NAME_PATTERN /Opus 4\.7/g.
fn source_name_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"Opus 4\.7").unwrap())
}

// /-\d{8}$/ — strips a trailing date suffix from the override to get the base id.
fn date_suffix_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"-\d{8}$").unwrap())
}

/// Maps a target model base id to its friendly display name. Mirrors
/// TARGET_FRIENDLY_NAMES in reference/pino/src/model.js.
fn target_friendly_name(base: &str) -> Option<&'static str> {
    match base {
        "claude-opus-4-6" => Some("Opus 4.6"),
        "claude-opus-4-5" => Some("Opus 4.5"),
        "claude-sonnet-4-6" => Some("Sonnet 4.6"),
        "claude-sonnet-4-5" => Some("Sonnet 4.5"),
        "claude-haiku-4-5" => Some("Haiku 4.5"),
        _ => None,
    }
}

fn apply_model_override(body: &mut Value, model: &str) {
    let obj = match body.as_object_mut() {
        Some(o) => o,
        None => return,
    };
    // Replace the top-level model field (server.js: parsed.model = MODEL_OVERRIDE).
    obj.insert("model".to_string(), Value::String(model.to_string()));

    // Compute the replacement strings (model.js: base/friendly).
    let base = date_suffix_re().replace(model, "").into_owned();
    let friendly: String = target_friendly_name(&base)
        .map(|s| s.to_string())
        .unwrap_or(base);

    // R18 / Finding 3: closure replacements so a '$' in the override (or friendly)
    // is emitted literally and NOT expanded as a regex capture template.
    let model_owned = model.to_string();
    let rewrite = |text: &str| -> String {
        let step1 = source_id_re().replace_all(text, |_: &regex::Captures| model_owned.clone());
        source_name_re()
            .replace_all(&step1, |_: &regex::Captures| friendly.clone())
            .into_owned()
    };

    match obj.get_mut("system") {
        Some(Value::String(s)) => {
            *s = rewrite(s);
        }
        Some(Value::Array(blocks)) => {
            for blk in blocks.iter_mut() {
                if let Some(Value::String(text)) = blk.get_mut("text") {
                    *text = rewrite(text);
                }
            }
        }
        _ => {}
    }
}

fn apply_default_transform(_body: &mut Value, _settings: &PinoSettings) {
    // Implemented in Tasks M4.4 (strip_ansi), M4.5 (drop_tools + reminder scrub),
    // and M4.5b (restructureV123).
}

fn apply_auto_cache(_body: &mut Value, _tail_ttl: TailTtl) {
    // Implemented in Tasks M4.6-M4.9.
}

#[cfg(test)]
#[path = "pino_tests.rs"]
mod pino_tests;
