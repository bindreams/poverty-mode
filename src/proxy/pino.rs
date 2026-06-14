//! pino: prompt-cache breakpoint injection. M1 ships the settings struct and a
//! fail-loud transform stub (R9); the real cache-injection logic lands in M4.

use std::sync::OnceLock;

use anyhow::Result;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

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
    // JS `\d` (no `u` flag) is ASCII-only; the Rust regex crate's `\d` is
    // Unicode-aware by default, so use `[0-9]` for Node parity (R18).
    RE.get_or_init(|| Regex::new(r"claude-opus-4-7(?:-[0-9]{8})?").unwrap())
}

// SOURCE_NAME_PATTERN /Opus 4\.7/g.
fn source_name_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"Opus 4\.7").unwrap())
}

// /-\d{8}$/ — strips a trailing date suffix from the override to get the base id.
fn date_suffix_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // ASCII-only `[0-9]` matches JS `\d` (no `u` flag); see source_id_re (R18).
    RE.get_or_init(|| Regex::new(r"-[0-9]{8}$").unwrap())
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

// --- strip_ansi (default.js lines 42-70) -----

// Matches a CSI/SGR sequence: ESC '[' <params> <final letter>. Port of the Node
// ANSI_RE /\x1b\[[0-9;]*[A-Za-z]/g; only this exact form is scrubbed.
fn ansi_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\x1b\[[0-9;]*[A-Za-z]").unwrap())
}

fn strip_ansi_str(s: &str) -> String {
    ansi_re().replace_all(s, "").into_owned()
}

// Scrubs ANSI escapes from m.content (string), each block's b.text, each block's
// b.content (string), and each nested rc.text when b.content is an array.
fn strip_ansi_from_messages(body: &mut Value) {
    let messages = match body.get_mut("messages").and_then(|m| m.as_array_mut()) {
        Some(m) => m,
        None => return,
    };
    for msg in messages.iter_mut() {
        let content = match msg.get_mut("content") {
            Some(c) => c,
            None => continue,
        };
        match content {
            Value::String(s) => {
                *s = strip_ansi_str(s);
            }
            Value::Array(blocks) => {
                for blk in blocks.iter_mut() {
                    if !blk.is_object() {
                        continue;
                    }
                    if let Some(Value::String(text)) = blk.get_mut("text") {
                        *text = strip_ansi_str(text);
                    }
                    match blk.get_mut("content") {
                        Some(Value::String(inner)) => {
                            *inner = strip_ansi_str(inner);
                        }
                        Some(Value::Array(inner_blocks)) => {
                            for rc in inner_blocks.iter_mut() {
                                if rc.is_object() {
                                    if let Some(Value::String(text)) = rc.get_mut("text") {
                                        *text = strip_ansi_str(text);
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
}

// --- drop_tools (default.js lines 72-113) -----

// Matches a <system-reminder>...</system-reminder> block (non-greedy). Port of
// the Node REMINDER_RE /<system-reminder>([\s\S]*?)<\/system-reminder>/g; JS
// `[\s\S]*?` (dot matches newline, non-greedy) == Rust `(?s).*?`.
fn reminder_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?s)<system-reminder>(.*?)</system-reminder>").unwrap())
}

// Node: /deferred tools|ToolSearch/i.test(inner). Case-insensitive on both literals.
fn advertises_deferred_tools(inner: &str) -> bool {
    let lower = inner.to_ascii_lowercase();
    lower.contains("deferred tools") || lower.contains("toolsearch")
}

fn drop_tools_from_tools(body: &mut Value, drop: &[String]) {
    if drop.is_empty() {
        return;
    }
    if let Some(tools) = body.get_mut("tools").and_then(|t| t.as_array_mut()) {
        // Node: body.tools.filter((t) => !DROP_TOOLS.has(t?.name)). A tool with no
        // string name has name === undefined, never in the Set => kept.
        tools.retain(|t| match t.get("name").and_then(|n| n.as_str()) {
            Some(name) => !drop.iter().any(|d| d == name),
            None => true,
        });
    }
}

fn scrub_reminder_text(text: &str, drop: &[String]) -> String {
    if drop.is_empty() {
        return text.to_string();
    }
    reminder_re()
        .replace_all(text, |caps: &regex::Captures| {
            let full = caps[0].to_string();
            let inner = &caps[1];
            if !advertises_deferred_tools(inner) {
                return full;
            }
            // Node: inner.split("\n").filter(line => !DROP_TOOLS.has(line.trim())).join("\n").
            let cleaned: Vec<&str> = inner
                .split('\n')
                .filter(|line| !drop.iter().any(|d| d == line.trim()))
                .collect();
            format!("<system-reminder>{}</system-reminder>", cleaned.join("\n"))
        })
        .into_owned()
}

fn scrub_reminders_from_messages(body: &mut Value, drop: &[String]) {
    if drop.is_empty() {
        return;
    }
    let messages = match body.get_mut("messages").and_then(|m| m.as_array_mut()) {
        Some(m) => m,
        None => return,
    };
    for msg in messages.iter_mut() {
        match msg.get_mut("content") {
            Some(Value::String(s)) => {
                *s = scrub_reminder_text(s, drop);
            }
            Some(Value::Array(blocks)) => {
                for blk in blocks.iter_mut() {
                    if blk.is_object() {
                        if let Some(Value::String(text)) = blk.get_mut("text") {
                            *text = scrub_reminder_text(text, drop);
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

// --- restructureV123 (default.js lines 126-208) -----

// Ported verbatim from reference/pino/src/transforms/default.js restructureV123
// (lines 126-208). Normalizes string content to arrays, extracts core-context
// blocks (ToolSearch / claudeMd / .claude paths) into messages[0], removes stale
// scaffolding from non-tail history, dedupes core blocks, sets msg0.role=user, and
// prunes emptied messages. R19: full parity, runs before cache injection. The Node
// source wraps the body in try/catch (logs and swallows); this port is panic-free
// by construction (pure serde_json::Value manipulation), so no catch is needed —
// the only cosmetic divergence is the absence of console logging.

fn is_core_context(t: &str) -> bool {
    if t.contains("<local-command-stdout>") || t.contains("<local-command-caveat>") {
        return false;
    }
    t.contains("ToolSearch")
        || t.contains("claudeMd")
        || t.contains(".claude/projects")
        || t.contains(".claude/plans")
}

fn is_stale_removable(t: &str) -> bool {
    t.starts_with("<system-reminder>")
        || t.starts_with("<local-command-stdout>")
        || t.starts_with("<local-command-caveat>")
        || t.starts_with("<command-name>")
}

fn restructure_v123(body: &mut Value) {
    let messages = match body.get_mut("messages").and_then(|m| m.as_array_mut()) {
        Some(m) => m,
        None => return,
    };
    // Node: if (!Array.isArray(body.messages) || body.messages.length < 2) return;
    if messages.len() < 2 {
        return;
    }

    // 1. Normalize all message contents to arrays.
    for msg in messages.iter_mut() {
        if let Some(content) = msg.get_mut("content") {
            if let Value::String(s) = content {
                let text = std::mem::take(s);
                *content = json!([ { "type": "text", "text": text } ]);
            }
        }
    }

    let last_index = messages.len() - 1;
    let mut core_blocks: Vec<Value> = Vec::new();

    // 2. Process ALL messages: extract core context, drop stale scaffolding from history.
    for (i, msg) in messages.iter_mut().enumerate() {
        let content = match msg.get_mut("content").and_then(|c| c.as_array_mut()) {
            Some(c) => c,
            None => continue, // Node: if (!Array.isArray(msg.content)) continue;
        };
        let is_tail = i == last_index;
        let old = std::mem::take(content);
        let mut new_content: Vec<Value> = Vec::new();
        for block in old.into_iter() {
            if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                if is_core_context(text) {
                    core_blocks.push(block);
                    continue;
                }
                if !is_tail && is_stale_removable(text) {
                    continue;
                }
            }
            // Preserve everything else: tool_results, normal text, tool_use, tail reminders.
            new_content.push(block);
        }
        *content = new_content;
    }

    // 3. Assemble msg0 with deduped core blocks (first occurrence wins, order preserved).
    if !core_blocks.is_empty() {
        let mut unique_core: Vec<Value> = Vec::new();
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        for b in core_blocks.into_iter() {
            let key = b
                .get("text")
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_string();
            if seen.insert(key) {
                unique_core.push(b);
            }
        }
        if let Some(msg0) = messages.get_mut(0) {
            if let Some(obj) = msg0.as_object_mut() {
                let existing = obj
                    .get_mut("content")
                    .and_then(|c| c.as_array_mut())
                    .map(std::mem::take)
                    .unwrap_or_default();
                let mut combined = unique_core;
                combined.extend(existing);
                obj.insert("content".to_string(), Value::Array(combined));
                obj.insert("role".to_string(), Value::String("user".to_string()));
            }
        }
    }

    // 4. Remove completely empty messages (Node: m.content && m.content.length > 0).
    messages.retain(|m| {
        m.get("content")
            .and_then(|c| c.as_array())
            .map(|a| !a.is_empty())
            .unwrap_or(false)
    });
}

fn apply_default_transform(body: &mut Value, settings: &PinoSettings) {
    // Node transforms/default.js transform() order verbatim:
    //   trimTools -> trimReminders -> trimSystem(inert) -> restructureV123 -> stripAnsiFromMessages.
    drop_tools_from_tools(body, &settings.drop_tools);
    scrub_reminders_from_messages(body, &settings.drop_tools);
    // trimSystem is an inert commented-out example in the Node source — not ported.
    restructure_v123(body);
    if settings.strip_ansi {
        strip_ansi_from_messages(body);
    }
}

fn apply_auto_cache(_body: &mut Value, _tail_ttl: TailTtl) {
    // Implemented in Tasks M4.6-M4.9.
}

#[cfg(test)]
#[path = "pino_tests.rs"]
mod pino_tests;
