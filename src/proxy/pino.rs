//! pino: prompt-cache breakpoint injection. M1 ships the settings struct and a
//! fail-loud transform stub (R9); the real cache-injection logic lands in M4.

use std::sync::OnceLock;

use anyhow::Result;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::proxy::{BodyTransform, RequestContext};

/// Cache TTL (`5m`/`1h`). Serializes to the short forms `"5m"` / `"1h"`.
///
/// Deserialization is **lenient** (R22/R23k — Node `parseTailTtl` parity,
/// `reference/pino/src/config.js` lines 36-44): the raw value is trimmed and
/// lowercased, then `"5m"` → `FiveMin`, `"1h"` → `OneHour`, and ANY other
/// string falls back to `FiveMin` with a logged `warn!` rather than erroring.
/// M2's config tests assert the fallback; M4 relies on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum CacheTtl {
    #[serde(rename = "5m")]
    FiveMin,
    #[serde(rename = "1h")]
    OneHour,
}

impl<'de> Deserialize<'de> for CacheTtl {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        // Node parseTailTtl: String(raw).trim().toLowerCase() before matching.
        match raw.trim().to_ascii_lowercase().as_str() {
            "1h" => Ok(CacheTtl::OneHour),
            // "5m" and every unrecognized value degrade to 5m (Node behavior).
            "5m" => Ok(CacheTtl::FiveMin),
            other => {
                tracing::warn!(
                    value = other,
                    "invalid cache TTL; falling back to 5m (valid values: 5m, 1h)"
                );
                Ok(CacheTtl::FiveMin)
            }
        }
    }
}

impl CacheTtl {
    /// Wire value written into `cache_control.ttl`.
    pub fn as_str(&self) -> &'static str {
        match self {
            CacheTtl::FiveMin => "5m",
            CacheTtl::OneHour => "1h",
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
    /// Cache TTL applied to main-agent requests (all slots).
    pub main_ttl: CacheTtl,
    /// Cache TTL applied to subagent requests (all slots).
    pub sub_ttl: CacheTtl,
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

/// Outcome of [`ensure_beta_header`]: whether the 1h-cache flag was newly added,
/// already present, or appended to existing flags. Mirrors the string returns of
/// `ensureBetaHeader` (reference/pino/src/cache.js 183-196).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BetaHeaderStatus {
    Added,
    Present,
    Appended,
}

/// Ensures `anthropic-beta` carries the 1h-cache beta flag. HEADER mutation —
/// called by the engine after the body transform (via PinoTransform::apply_headers),
/// NOT inside `transform`. Mirrors ensureBetaHeader (reference/pino/src/cache.js 183-196).
///
/// Finding 9: http::HeaderMap can hold multiple anthropic-beta values; this merges
/// across ALL of them (so a flag in any value is detected and none is dropped) and
/// re-inserts a single canonical comma-joined value.
pub fn ensure_beta_header(headers: &mut HeaderMap) -> BetaHeaderStatus {
    let name = HeaderName::from_static("anthropic-beta");

    // Collect every existing value (the map may hold several lines for this key).
    let existing: Vec<String> = headers
        .get_all(&name)
        .iter()
        .filter_map(|v| v.to_str().ok().map(|s| s.to_string()))
        .collect();

    if existing.is_empty() {
        headers.insert(&name, HeaderValue::from_static(BETA_FLAG));
        return BetaHeaderStatus::Added;
    }

    // Merge the union of all comma-separated tokens across every value line.
    let joined = existing.join(",");
    let already = joined.split(',').map(|s| s.trim()).any(|s| s == BETA_FLAG);

    if already {
        // Collapse multi-value into a single canonical line (idempotent for one value).
        let canonical = HeaderValue::from_str(&joined).expect("beta header value is ascii");
        headers.insert(&name, canonical);
        BetaHeaderStatus::Present
    } else {
        let combined = format!("{},{}", joined, BETA_FLAG);
        headers.insert(
            &name,
            HeaderValue::from_str(&combined).expect("beta header value is ascii"),
        );
        BetaHeaderStatus::Appended
    }
}

impl PinoTransform {
    /// True when at least one body-mutating feature is active. Mirrors reference
    /// pino's `mutate` guard (`AUTO_CACHE || transformFn || MODEL_OVERRIDE`,
    /// reference/pino/src/server.js:59) where `transformFn` is the default
    /// pipeline driven by `drop_tools` / `strip_ansi`. When NONE is active, pino
    /// forwards the original bytes untouched (a TRUE byte passthrough).
    fn has_active_feature(&self) -> bool {
        self.settings.auto_cache
            || !self.settings.drop_tools.is_empty()
            || self.settings.strip_ansi
            || self.settings.model_override.is_some()
    }
}

impl BodyTransform for PinoTransform {
    // FIX-B: pino's byte seam. With NO feature active, return None (TRUE byte
    // passthrough — the engine forwards the original request bytes verbatim,
    // matching reference pino's `mutate=false` arm). With any feature active,
    // parse -> mutate -> serialize -> Some: pino re-serialization is acceptable
    // because the prompt cache relies on cross-turn CONSISTENCY (a stable
    // canonical form per turn), which this preserves.
    fn transform_bytes(&self, raw: &[u8], ctx: &RequestContext) -> Result<Option<Vec<u8>>> {
        if !self.has_active_feature() {
            return Ok(None);
        }
        let mut body: Value = serde_json::from_slice(raw)?;
        self.transform(&mut body, ctx)?;
        Ok(Some(serde_json::to_vec(&body)?))
    }

    fn transform(&self, body: &mut Value, ctx: &RequestContext) -> Result<()> {
        // Only object bodies are mutable in any meaningful way; non-objects pass through.
        if !body.is_object() {
            return Ok(());
        }
        // An already-invalid input (CC's choice) is transformed normally and left for the
        // API to arbitrate; the guard below only catches REGRESSING a valid one.
        let input_valid = messages_structure_is_api_valid(body);
        if !input_valid {
            tracing::debug!("pino: input messages array was already API-invalid before transform");
        }
        // Operation order mirrors reference/pino/src/server.js lines 70-98:
        // 1. model override (replaces body.model + rewrites system self-references).
        if let Some(model) = self.settings.model_override.as_deref() {
            apply_model_override(body, model);
        }
        // 2. built-in default transform pipeline (drop_tools + reminder scrub +
        //    restructureV123 + strip_ansi), in the Node transforms/default.js order.
        apply_default_transform(body, &self.settings);
        // 3. auto-cache: pick the per-agent TTL (subagent vs main) and apply it
        //    uniformly to every injected/normalized cache slot.
        if self.settings.auto_cache {
            let ttl = if ctx.is_subagent {
                self.settings.sub_ttl
            } else {
                self.settings.main_ttl
            };
            apply_auto_cache(body, ttl);
        }
        check_invariant(input_valid, body)
    }

    // R6: apply the 1h-cache beta header only when auto_cache is on (matches
    // server.js `AUTO_CACHE && parsed` guard). The engine calls this after
    // transform() and after Host/Content-Length rewrite, on transformed POSTs.
    fn apply_headers(&self, headers: &mut http::HeaderMap) {
        if self.settings.auto_cache {
            ensure_beta_header(headers);
        }
    }
}

// --- pipeline stages (filled in by later tasks) ----------------------------------------------------------------------

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
    let friendly: String = target_friendly_name(&base).map(|s| s.to_string()).unwrap_or(base);

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

// --- strip_ansi (default.js lines 42-70) -----------------------------------------------------------------------------

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

// --- drop_tools (default.js lines 72-113) ----------------------------------------------------------------------------

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

// --- restructureV123 (default.js lines 126-208) ----------------------------------------------------------------------

// Ported from reference/pino/src/transforms/default.js restructureV123 (lines 126-208):
// normalizes string content to arrays, hoists core-context blocks (ToolSearch / claudeMd
// / .claude paths) into the first user message, removes stale scaffolding from non-tail
// history, dedupes core blocks, and prunes emptied messages. Runs before cache injection.
//
// DELIBERATE divergence from the reference (which the reference lacks because it was only
// ever exercised on Opus 4.7): the reference scavenges every message and prunes anything
// it empties, which on Opus 4.8 traffic corrupts the array three ways — pruning the tail
// task-notification (ends-on-assistant), pruning a mid-conversation role:"system"
// predecessor (orphaned system), and stripping an assistant turn's trailing text to leave
// a `thinking` block. So here: only role:"user" messages are scavenged, load-bearing
// messages are never emptied or pruned (see compute_load_bearing), and core is hoisted
// into the first user message (never a non-user msg0, so a directive/assistant turn is
// never role-stomped). messages_structure_is_api_valid + check_invariant back this with a
// passthrough guard. Panic-free by construction; no try/catch needed.

fn is_core_context(t: &str) -> bool {
    if t.contains("<local-command-stdout>") || t.contains("<local-command-caveat>") {
        return false;
    }
    t.contains("ToolSearch") || t.contains("claudeMd") || t.contains(".claude/projects") || t.contains(".claude/plans")
}

fn is_stale_removable(t: &str) -> bool {
    t.starts_with("<system-reminder>")
        || t.starts_with("<local-command-stdout>")
        || t.starts_with("<local-command-caveat>")
        || t.starts_with("<command-name>")
}

// The three Anthropic structural rules this transform must never violate are mirrored
// from observed 400 responses. Each is CHECKED by `messages_structure_is_api_valid` and
// PREVENTED by a matching enforcement below; a fourth rule must extend BOTH, and the
// `transform_never_regresses_a_valid_array_fuzz` property test is the net that catches
// drift between the two:
//
//   rule                              enforced by
//   last message is `user`            the tail is load-bearing (compute_load_bearing)
//   role:"system" placement           a system and its predecessor are load-bearing
//   assistant not ending in thinking  assistant messages are never scavenged (classify_block)
//
// Load-bearing also preserves CONTENT (the newest tail turn, a system directive), which is
// not itself a validity rule, so it cannot be derived purely from the validator.

/// Flags each message that must not be emptied or pruned by restructuring: the tail, a
/// `role:"system"` message, or the immediate predecessor of one. Divergence from
/// reference pino. (msg0 is intentionally NOT blanket-protected: step-3 assembly already
/// keeps it non-empty when core context exists, and blanket protection would reopen a
/// within-msg0 content duplication.)
fn compute_load_bearing(messages: &[Value]) -> Vec<bool> {
    let n = messages.len();
    let role = |i: usize| messages[i].get("role").and_then(|r| r.as_str());
    (0..n)
        .map(|i| i == n - 1 || role(i) == Some("system") || (i + 1 < n && role(i + 1) == Some("system")))
        .collect()
}

/// What restructuring does with one content block. Computed once per block and reused by
/// the "would anything survive?" pre-check and the partition, so the two can't drift.
enum BlockFate {
    Extract, // core-context: hoist into the first user message
    Drop,    // stale scaffolding in history
    Keep,    // everything else: tool_use / tool_result / image / normal text / tail reminders
}

/// Only `role:"user"` messages are scavenged (`is_user`): core context is user-side
/// (reminders / tool-results), and scavenging an assistant turn can strip its trailing
/// text and leave it ending in a `thinking` block, while scavenging a system message
/// would corrupt a directive. A core block is never dropped as stale — core-ness shields
/// it. Extracted core is hoisted into the first user message (step 3), never into a
/// non-user `messages[0]`, so no directive/assistant turn is ever role-stomped.
fn classify_block(block: &Value, is_tail: bool, is_user: bool) -> BlockFate {
    if !is_user {
        return BlockFate::Keep;
    }
    if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
        if is_core_context(text) {
            return BlockFate::Extract;
        }
        if !is_tail && is_stale_removable(text) {
            return BlockFate::Drop;
        }
    }
    BlockFate::Keep
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
    let load_bearing = compute_load_bearing(messages);
    let mut core_blocks: Vec<Value> = Vec::new();

    // 2. Extract core context / drop stale scaffolding from user messages only — but
    //    NEVER empty a load-bearing message: if processing would leave it with zero
    //    blocks, keep it verbatim.
    for (i, msg) in messages.iter_mut().enumerate() {
        let is_user = msg.get("role").and_then(|r| r.as_str()) == Some("user");
        let content = match msg.get_mut("content").and_then(|c| c.as_array_mut()) {
            Some(c) => c,
            None => continue,
        };
        let is_tail = i == last_index;
        let old = std::mem::take(content);

        let fates: Vec<BlockFate> = old.iter().map(|b| classify_block(b, is_tail, is_user)).collect();
        let survives_any = fates.iter().any(|f| matches!(f, BlockFate::Keep));
        if load_bearing[i] && !survives_any {
            *content = old;
            continue;
        }

        let mut new_content: Vec<Value> = Vec::new();
        for (block, fate) in old.into_iter().zip(fates) {
            match fate {
                BlockFate::Extract => core_blocks.push(block),
                BlockFate::Drop => {}
                BlockFate::Keep => new_content.push(block),
            }
        }
        // Structural invariant; the survives_any pre-check guarantees it, and this stays
        // visible in release (not only the debug build) because a violation would 400.
        if load_bearing[i] && new_content.is_empty() {
            tracing::error!(target: "pino::invariant", "load-bearing message {i} emptied by restructure (pino bug)");
        }
        debug_assert!(
            !load_bearing[i] || !new_content.is_empty(),
            "load-bearing message {i} emptied"
        );
        *content = new_content;
    }

    // 3. Hoist deduped core blocks (first occurrence wins) into the first user message.
    //    Core came from a user message, so a user target always exists.
    if !core_blocks.is_empty() {
        let mut unique_core: Vec<Value> = Vec::new();
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        for b in core_blocks.into_iter() {
            let key = b.get("text").and_then(|t| t.as_str()).unwrap_or("").to_string();
            if seen.insert(key) {
                unique_core.push(b);
            }
        }
        let target = messages
            .iter()
            .position(|m| m.get("role").and_then(|r| r.as_str()) == Some("user"));
        // Core came from a user message, so a user target always exists. Kept visible in
        // release (like the emptied-load-bearing check) — a violation silently drops content.
        if target.is_none() {
            tracing::error!(
                target: "pino::invariant",
                "core extracted but no user message to hoist into (pino bug); {} core blocks dropped",
                unique_core.len()
            );
        }
        debug_assert!(target.is_some(), "core extracted but no user message to hoist into");
        if let Some(obj) = target.and_then(|idx| messages[idx].as_object_mut()) {
            let existing = obj
                .get_mut("content")
                .and_then(|c| c.as_array_mut())
                .map(std::mem::take)
                .unwrap_or_default();
            let mut combined = unique_core;
            // Dedup existing against the hoisted core so a target that already holds the
            // same core text cannot end up with a duplicated block.
            for b in existing.into_iter() {
                let dup = b
                    .get("text")
                    .and_then(|t| t.as_str())
                    .map(|t| seen.contains(t))
                    .unwrap_or(false);
                if !dup {
                    combined.push(b);
                }
            }
            obj.insert("content".to_string(), Value::Array(combined));
        }
    }

    // 4. Prune emptied messages — but NEVER a load-bearing one. `load_bearing` still
    //    aligns with `messages` (steps 2-3 never added or removed a message). This keeps
    //    a directive-only `role:"system"` message that arrived with content:[].
    let mut i = 0usize;
    messages.retain(|m| {
        let keep = load_bearing[i]
            || m.get("content")
                .and_then(|c| c.as_array())
                .map(|a| !a.is_empty())
                .unwrap_or(false);
        i += 1;
        keep
    });
}

/// True when a message carries no meaningful content — matching what step-1
/// normalization produces from an empty string, so the verdict is identical whether the
/// content arrives as a string or an already-normalized array.
fn content_is_empty(message: &Value) -> bool {
    match message.get("content") {
        None => true,
        Some(Value::String(s)) => s.is_empty(),
        Some(Value::Array(blocks)) => blocks.iter().all(|b| {
            b.get("type").and_then(|t| t.as_str()) == Some("text")
                && b.get("text")
                    .and_then(|t| t.as_str())
                    .map(|s| s.is_empty())
                    .unwrap_or(true)
        }),
        Some(_) => false,
    }
}

/// True when `body`'s `messages` array cannot trip the three Anthropic structural rules
/// this transform must never violate — mirrored verbatim from the three 400 response
/// bodies pino was observed to produce, nothing more:
///
/// 1. "must end with a user message" — the last message must have role "user".
/// 2. "role 'system' must follow a 'user' message or an 'assistant' message ending in a
///    server tool result; the directive-only form (content: [] with output_config) is
///    accepted at any position" — a content-bearing system needs a user/assistant
///    predecessor; only the directive-only form (content-empty AND output_config) is
///    legal anywhere.
/// 3. "the final block in an assistant message cannot be `thinking`" (nor the same
///    thinking-class `redacted_thinking`).
///
/// New behavior; reference pino modelled none of these shapes. The assistant-predecessor
/// arm is deliberately lenient (any assistant, not only one "ending in a server tool
/// result"): the precise predicate risks false positives, and load-bearing protection —
/// not this backstop — is what keeps a system predecessor legal. `content_is_empty`
/// matches step-1 normalization so the verdict is identical before and after it.
pub(crate) fn messages_structure_is_api_valid(body: &Value) -> bool {
    let messages = match body.get("messages").and_then(|m| m.as_array()) {
        Some(m) => m,
        None => return true,
    };
    let Some(last) = messages.last() else {
        return false;
    };
    if last.get("role").and_then(|r| r.as_str()) != Some("user") {
        return false;
    }
    for (i, m) in messages.iter().enumerate() {
        match m.get("role").and_then(|r| r.as_str()) {
            Some("system") => {
                // The directive-only form — content: [] WITH output_config — is accepted
                // at any position (string #2). Anything else obeys the predecessor rule.
                if content_is_empty(m) && m.get("output_config").is_some() {
                    continue;
                }
                if i == 0 {
                    return false;
                }
                match messages[i - 1].get("role").and_then(|r| r.as_str()) {
                    Some("user") | Some("assistant") => {}
                    _ => return false,
                }
            }
            Some("assistant") => {
                // "final block cannot be `thinking`" — `redacted_thinking` is the same
                // thinking-class block (see headroom HOT_ZONE_BLOCK_TYPES), so it applies too.
                let last_type = m
                    .get("content")
                    .and_then(|c| c.as_array())
                    .and_then(|a| a.last())
                    .and_then(|b| b.get("type"))
                    .and_then(|t| t.as_str());
                if matches!(last_type, Some("thinking") | Some("redacted_thinking")) {
                    return false;
                }
            }
            _ => {}
        }
    }
    true
}

/// Belt-and-suspenders post-transform guard. Returns `Err` — so the engine forwards the
/// ORIGINAL bytes (known-valid; CC produced them) instead of a 400-shaped body — iff a
/// valid input was regressed to an invalid output. With the load-bearing protection this
/// never fires in practice; a firing is a pino bug, logged loudly (distinct target)
/// rather than swallowed by the engine's generic transform-error warning.
fn check_invariant(input_valid: bool, body: &Value) -> Result<()> {
    if input_valid && !messages_structure_is_api_valid(body) {
        tracing::error!(
            target: "pino::invariant",
            "pino regressed a valid messages array to API-invalid; forwarding original body (pino bug)"
        );
        anyhow::bail!("pino transform regressed a valid messages array to API-invalid");
    }
    Ok(())
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

// --- cache helpers (cache.js lines 28-96) ----------------------------------------------------------------------------

/// Counts every `cache_control.type == "ephemeral"` breakpoint anywhere in the
/// body. Mirrors countCacheBreakpoints in reference/pino/src/cache.js (lines 28-38).
pub fn count_cache_breakpoints(body: &Value) -> usize {
    fn walk(x: &Value, n: &mut usize) {
        match x {
            Value::Array(items) => {
                for it in items {
                    walk(it, n);
                }
            }
            Value::Object(map) => {
                if let Some(cc) = map.get("cache_control") {
                    if cc.get("type").and_then(|t| t.as_str()) == Some("ephemeral") {
                        *n += 1;
                    }
                }
                for (_k, v) in map.iter() {
                    walk(v, n);
                }
            }
            _ => {}
        }
    }
    let mut n = 0;
    walk(body, &mut n);
    n
}

fn block_has_ephemeral(block: &Value) -> bool {
    block
        .get("cache_control")
        .and_then(|cc| cc.get("type"))
        .and_then(|t| t.as_str())
        == Some("ephemeral")
}

/// True if any element of `arr` carries an ephemeral breakpoint.
/// Mirrors hasBreakpoint in reference/pino/src/cache.js (lines 77-81).
fn has_breakpoint(arr: &Value) -> bool {
    match arr.as_array() {
        Some(items) => items.iter().any(block_has_ephemeral),
        None => false,
    }
}

/// True if a tool is a valid target for the tools cache breakpoint. A tool is not
/// a target when it is not a JSON object (nowhere to insert `cache_control`) or
/// when it is deferred (`defer_loading: true`, the ToolSearch catalog). Deferred
/// tools are excluded from the cached prefix and the Anthropic API rejects
/// `cache_control` on them with a 400 (issue #5): "Tool '...' cannot have both
/// defer_loading=true and cache_control set." `defer_loading` is a boolean per the
/// API tool schema; we treat the field as blocking caching unless it is absent or
/// explicitly `false`, so a malformed non-boolean value (out of contract) degrades
/// to skip — never a stamped 400. New behavior: pino had no model of tool
/// deferral, so there is no Node `cache.js` analog to mirror here.
fn is_cacheable_tool(tool: &Value) -> bool {
    if !tool.is_object() {
        return false;
    }
    match tool.get("defer_loading") {
        None => true,
        Some(v) => v == &Value::Bool(false),
    }
}

/// Removes any `cache_control` already sitting on a deferred tool. A breakpoint on
/// a `defer_loading: true` tool 400s (issue #5) regardless of who placed it, so
/// pino must scrub a client-sent one too — otherwise `rewrite_cache_control` would
/// bump its ttl and forward the 400-triggering body. Returns the count stripped.
/// Runs before tools injection so the no-double-injection guard sees only
/// legitimate (non-deferred) breakpoints and can still cache a real tool.
fn strip_deferred_tool_breakpoints(body: &mut Value) -> usize {
    let tools = match body.get_mut("tools").and_then(|t| t.as_array_mut()) {
        Some(t) => t,
        None => return 0,
    };
    let mut stripped = 0;
    for tool in tools.iter_mut() {
        // Non-cacheable objects are exactly the deferred ones (non-objects have no
        // cache_control to remove and short-circuit in as_object_mut below).
        if !is_cacheable_tool(tool) {
            if let Some(obj) = tool.as_object_mut() {
                if obj.remove("cache_control").is_some() {
                    stripped += 1;
                }
            }
        }
    }
    stripped
}

/// Removes client-sent ephemeral cache_control from system blocks shorter than
/// MIN_SYSTEM_CACHE_CHARS. Returns the count stripped.
/// Mirrors stripSmallSystemBreakpoints in reference/pino/src/cache.js (lines 83-96).
///
/// R18 / Finding 5: length is UTF-16 code units (Node String.length) so the
/// boundary decision matches Node exactly even for astral-plane characters.
pub fn strip_small_system_breakpoints(body: &mut Value) -> usize {
    let blocks = match body.get_mut("system").and_then(|s| s.as_array_mut()) {
        Some(b) => b,
        None => return 0,
    };
    let mut stripped = 0;
    for block in blocks.iter_mut() {
        let obj = match block.as_object_mut() {
            Some(o) => o,
            None => continue,
        };
        let is_ephemeral = obj
            .get("cache_control")
            .and_then(|cc| cc.get("type"))
            .and_then(|t| t.as_str())
            == Some("ephemeral");
        if !is_ephemeral {
            continue;
        }
        let len = obj
            .get("text")
            .and_then(|t| t.as_str())
            .map(|s| s.encode_utf16().count()) // Node String.length == UTF-16 code units.
            .unwrap_or(0);
        if len < MIN_SYSTEM_CACHE_CHARS {
            obj.remove("cache_control");
            stripped += 1;
        }
    }
    stripped
}

/// Deletes cache_control from blocks in every message EXCEPT the first and last.
/// Returns the count stripped. Mirrors stripIntermediateMessageBreakpoints
/// (reference/pino/src/cache.js lines 61-75).
pub fn strip_intermediate_message_breakpoints(body: &mut Value) -> usize {
    let messages = match body.get_mut("messages").and_then(|m| m.as_array_mut()) {
        Some(m) => m,
        None => return 0,
    };
    if messages.len() <= 2 {
        return 0;
    }
    let last = messages.len() - 1;
    let mut stripped = 0;
    // Node iterates i in 1..messages.length-1 (every message except the first and
    // last); take(last).skip(1) yields exactly those indices.
    for msg in messages.iter_mut().take(last).skip(1) {
        if let Some(content) = msg.get_mut("content").and_then(|c| c.as_array_mut()) {
            for block in content.iter_mut() {
                if let Some(obj) = block.as_object_mut() {
                    if obj.contains_key("cache_control") {
                        obj.remove("cache_control");
                        stripped += 1;
                    }
                }
            }
        }
    }
    stripped
}

fn cc(ttl: CacheTtl) -> Value {
    json!({ "type": "ephemeral", "ttl": ttl.as_str() })
}

/// Normalizes `messages[idx].content` to an array if it is a non-empty string,
/// then returns the index of the last cacheable block (`text`/`tool_result`/`image`),
/// or None. Mirrors findLastCacheableBlockInMessage (reference/pino/src/cache.js 98-113).
fn find_last_cacheable_index_in_message(message: &mut Value) -> Option<usize> {
    let content = message.get_mut("content")?;
    match content {
        Value::Array(blocks) => {
            for j in (0..blocks.len()).rev() {
                let ty = blocks[j].get("type").and_then(|t| t.as_str());
                if matches!(ty, Some("text") | Some("tool_result") | Some("image")) {
                    return Some(j);
                }
            }
            None
        }
        Value::String(s) if !s.is_empty() => {
            let text = std::mem::take(s);
            *content = json!([ { "type": "text", "text": text } ]);
            Some(0)
        }
        _ => None,
    }
}

/// Injects cache breakpoints within the 4-cap. Returns the JSON-Pointer paths of
/// the tail blocks placed (so the ttl-rewrite can skip them). Mirrors
/// injectBreakpointIfAbsent (reference/pino/src/cache.js 124-181).
pub fn inject_breakpoint_if_absent(body: &mut Value, ttl: CacheTtl) -> Vec<String> {
    let mut tail_paths: Vec<String> = Vec::new();

    // 0. Scrub any breakpoint already on a deferred tool (issue #5): it 400s no
    //    matter who placed it, and stripping it first lets step 2's guard see only
    //    legitimate breakpoints and still cache a real tool.
    strip_deferred_tool_breakpoints(body);

    // 1. Reclaim wasted small-system slots.
    strip_small_system_breakpoints(body);

    // 2. tools: place the breakpoint on the last CACHEABLE tool (a non-deferred
    //    object), only if the array is non-empty and carries no breakpoint yet.
    //    Deferred tools (`defer_loading: true`, the ToolSearch catalog) are
    //    excluded from the cached prefix and the API rejects `cache_control` on
    //    them — so the breakpoint lands on the last cacheable tool, or nowhere if
    //    none qualifies. The no-existing-breakpoint guard is unchanged.
    let tool_target = match body.get("tools").and_then(|t| t.as_array()) {
        Some(tools) if !tools.is_empty() && !tools.iter().any(block_has_ephemeral) => {
            tools.iter().rposition(is_cacheable_tool)
        }
        _ => None,
    };
    if let Some(idx) = tool_target {
        // is_cacheable_tool guaranteed object-ness, so as_object_mut never fails here.
        debug_assert!(body["tools"][idx].is_object());
        if let Some(obj) = body["tools"][idx].as_object_mut() {
            obj.insert("cache_control".to_string(), cc(ttl));
        }
    }

    // 3. system: array -> last block ttl (no existing breakpoint); string -> cached array.
    //    Finding 6: compute the array-arm condition as a bool first to avoid a dead
    //    binding and a double-fetch of body["system"], mirroring the tools pattern.
    let inject_system_array = matches!(body.get("system"), Some(Value::Array(a)) if !a.is_empty())
        && body.get("system").map(|s| !has_breakpoint(s)).unwrap_or(false);
    if inject_system_array {
        if let Some(sys) = body.get_mut("system").and_then(|s| s.as_array_mut()) {
            if let Some(last) = sys.last_mut() {
                if let Some(obj) = last.as_object_mut() {
                    obj.insert("cache_control".to_string(), cc(ttl));
                }
            }
        }
    } else if let Some(Value::String(s)) = body.get("system") {
        if !s.is_empty() {
            let text = s.clone();
            body["system"] = json!([
                { "type": "text", "text": text, "cache_control": cc(ttl) }
            ]);
        }
    }

    // 4. msg[0] dedicated breakpoint, only with a distinct tail message and under the cap.
    let has_multiple_messages = body
        .get("messages")
        .and_then(|m| m.as_array())
        .map(|a| a.len() > 1)
        .unwrap_or(false);
    if has_multiple_messages && count_cache_breakpoints(body) < BREAKPOINT_CEILING {
        if let Some(messages) = body.get_mut("messages").and_then(|m| m.as_array_mut()) {
            if let Some(first) = messages.first_mut() {
                if let Some(idx) = find_last_cacheable_index_in_message(first) {
                    let block = &mut first["content"][idx];
                    if block.get("cache_control").is_none() {
                        if let Some(obj) = block.as_object_mut() {
                            obj.insert("cache_control".to_string(), cc(ttl));
                        }
                    }
                }
            }
        }
    }

    // 5. Rolling tail: last cacheable block across ALL messages -> ttl.
    if count_cache_breakpoints(body) < BREAKPOINT_CEILING {
        let msg_count = body
            .get("messages")
            .and_then(|m| m.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        let mut placed: Option<(usize, usize)> = None;
        if let Some(messages) = body.get_mut("messages").and_then(|m| m.as_array_mut()) {
            // findLastCacheableMessageBlock: scan messages from the end.
            for i in (0..msg_count).rev() {
                if let Some(idx) = find_last_cacheable_index_in_message(&mut messages[i]) {
                    placed = Some((i, idx));
                    break;
                }
            }
        }
        if let Some((i, idx)) = placed {
            let block = &mut body["messages"][i]["content"][idx];
            if block.get("cache_control").is_none() {
                if let Some(obj) = block.as_object_mut() {
                    obj.insert("cache_control".to_string(), cc(ttl));
                }
                tail_paths.push(format!("/messages/{}/content/{}", i, idx));
            }
        }
    }

    tail_paths
}

// --- tail normalization + ttl rewrite with skip-set (cache.js 3-26, 44-59) -------------------------------------------

use std::collections::HashSet;

/// Forces every ephemeral breakpoint inside the LAST message to `ttl` and
/// returns their JSON-Pointer (block) paths. Mirrors normalizeTailBreakpoints
/// (reference/pino/src/cache.js 44-59).
pub fn normalize_tail_breakpoints(body: &mut Value, ttl: CacheTtl) -> Vec<String> {
    let mut out = Vec::new();
    let msg_count = body
        .get("messages")
        .and_then(|m| m.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    if msg_count == 0 {
        return out;
    }
    let last = msg_count - 1;
    let base = format!("/messages/{}", last);
    if let Some(messages) = body.get_mut("messages").and_then(|m| m.as_array_mut()) {
        normalize_walk(&mut messages[last], &base, ttl, &mut out);
    }
    out
}

fn normalize_walk(node: &mut Value, path: &str, ttl: CacheTtl, out: &mut Vec<String>) {
    match node {
        Value::Object(map) => {
            let is_ephemeral = map
                .get("cache_control")
                .and_then(|cc| cc.get("type"))
                .and_then(|t| t.as_str())
                == Some("ephemeral");
            if is_ephemeral {
                if let Some(cc) = map.get_mut("cache_control").and_then(|c| c.as_object_mut()) {
                    cc.insert("ttl".to_string(), Value::String(ttl.as_str().to_string()));
                }
                out.push(path.to_string());
            }
            // Node recurses into every key EXCEPT cache_control (cache.js line 55).
            let keys: Vec<String> = map.keys().filter(|k| *k != "cache_control").cloned().collect();
            for k in keys {
                let child_path = format!("{}/{}", path, k);
                if let Some(child) = map.get_mut(&k) {
                    normalize_walk(child, &child_path, ttl, out);
                }
            }
        }
        Value::Array(items) => {
            for (i, item) in items.iter_mut().enumerate() {
                let child_path = format!("{}/{}", path, i);
                normalize_walk(item, &child_path, ttl, out);
            }
        }
        _ => {}
    }
}

/// Recursively bumps every ephemeral `cache_control.ttl` to `ttl` except nodes
/// whose JSON-Pointer (block) path is in `skip`. Mirrors rewriteCacheControl
/// (reference/pino/src/cache.js 3-26).
pub fn rewrite_cache_control(body: &mut Value, skip: &HashSet<String>, ttl: CacheTtl) {
    rewrite_walk(body, String::new(), skip, ttl);
}

fn rewrite_walk(node: &mut Value, path: String, skip: &HashSet<String>, ttl: CacheTtl) {
    match node {
        Value::Object(map) => {
            let is_ephemeral = map
                .get("cache_control")
                .and_then(|cc| cc.get("type"))
                .and_then(|t| t.as_str())
                == Some("ephemeral");
            if is_ephemeral && !skip.contains(&path) {
                if let Some(cc) = map.get_mut("cache_control").and_then(|c| c.as_object_mut()) {
                    cc.insert("ttl".to_string(), Value::String(ttl.as_str().to_string()));
                }
            }
            let keys: Vec<String> = map.keys().cloned().collect();
            for k in keys {
                let child_path = format!("{}/{}", path, k);
                if let Some(child) = map.get_mut(&k) {
                    rewrite_walk(child, child_path, skip, ttl);
                }
            }
        }
        Value::Array(items) => {
            for (i, item) in items.iter_mut().enumerate() {
                let child_path = format!("{}/{}", path, i);
                rewrite_walk(item, child_path, skip, ttl);
            }
        }
        _ => {}
    }
}

fn apply_auto_cache(body: &mut Value, ttl: CacheTtl) {
    // Mirrors the AUTO_CACHE block of reference/pino/src/server.js (lines 88-98).
    // One TTL (selected per agent) is applied uniformly to every cache slot.
    strip_intermediate_message_breakpoints(body);
    let injected_tail = inject_breakpoint_if_absent(body, ttl);
    let client_tail = normalize_tail_breakpoints(body, ttl);
    let mut skip: HashSet<String> = HashSet::new();
    skip.extend(injected_tail);
    skip.extend(client_tail);
    rewrite_cache_control(body, &skip, ttl);
}

#[cfg(test)]
#[path = "pino_tests.rs"]
mod pino_tests;
