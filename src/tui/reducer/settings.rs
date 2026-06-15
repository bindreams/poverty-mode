//! Per-setting value model for the interactive picker's expanded rows.
//!
//! Each [`SettingId`] names one editable field of a proxy's [`ProxySettings`].
//! [`SettingKind`] classifies how it is edited; the free functions/methods read
//! and mutate the live `ProxySettings` so the render layer and reducer never
//! re-format or duplicate value logic.

use crate::config::ProxySettings;
use crate::proxy::pino::CacheTtl;
use crate::proxy::ProxyName;

#[cfg(test)]
#[path = "settings_tests.rs"]
mod settings_tests;

/// Identity of one editable per-proxy setting. Nine variants spanning all three
/// proxies; [`settings_of`] returns the fixed display order for each proxy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingId {
    // pino
    AutoCache,
    MainTtl,
    SubTtl,
    DropTools,
    StripAnsi,
    ModelOverride,
    // headroom
    Compression,
    // central
    Port,
    PinnedVersion,
}

/// How a setting is edited and rendered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingKind {
    /// On/off toggle (`[x]` / `[ ]`).
    Bool,
    /// Fixed cycle of values (`‹ 5m ›`).
    Enum,
    /// Free single-line text; empty clears to `None`.
    Text,
    /// Comma-separated list of names.
    List,
    /// A `u16` (port); empty clears to `None`.
    Number,
}

const PINO_SETTINGS: &[SettingId] = &[
    SettingId::AutoCache,
    SettingId::MainTtl,
    SettingId::SubTtl,
    SettingId::DropTools,
    SettingId::StripAnsi,
    SettingId::ModelOverride,
];
const HEADROOM_SETTINGS: &[SettingId] = &[SettingId::Compression];
const CENTRAL_SETTINGS: &[SettingId] = &[SettingId::Port, SettingId::PinnedVersion];

/// The fixed, ordered list of editable settings for a proxy.
pub fn settings_of(name: ProxyName) -> &'static [SettingId] {
    match name {
        ProxyName::Pino => PINO_SETTINGS,
        ProxyName::Headroom => HEADROOM_SETTINGS,
        ProxyName::Central => CENTRAL_SETTINGS,
    }
}

impl SettingId {
    /// The human-facing field name shown to the left of the value in an expanded
    /// setting row.
    pub fn label(self) -> &'static str {
        match self {
            SettingId::AutoCache => "auto-cache",
            SettingId::MainTtl => "main-ttl",
            SettingId::SubTtl => "sub-ttl",
            SettingId::DropTools => "drop-tools",
            SettingId::StripAnsi => "strip-ansi",
            SettingId::ModelOverride => "model",
            SettingId::Compression => "compression",
            SettingId::Port => "port",
            SettingId::PinnedVersion => "version",
        }
    }

    /// How this setting is edited/rendered.
    pub fn kind(self) -> SettingKind {
        match self {
            SettingId::AutoCache | SettingId::StripAnsi | SettingId::Compression => {
                SettingKind::Bool
            }
            SettingId::MainTtl | SettingId::SubTtl => SettingKind::Enum,
            SettingId::DropTools => SettingKind::List,
            SettingId::ModelOverride | SettingId::PinnedVersion => SettingKind::Text,
            SettingId::Port => SettingKind::Number,
        }
    }

    /// Flip a `Bool` setting in place. Debug-asserts the id is a `Bool`.
    pub fn toggle(self, s: &mut ProxySettings) {
        debug_assert_eq!(self.kind(), SettingKind::Bool, "toggle on non-bool setting");
        match (self, s) {
            (SettingId::AutoCache, ProxySettings::Pino(p)) => p.auto_cache = !p.auto_cache,
            (SettingId::StripAnsi, ProxySettings::Pino(p)) => p.strip_ansi = !p.strip_ansi,
            (SettingId::Compression, ProxySettings::Headroom(h)) => h.compression = !h.compression,
            _ => debug_assert!(false, "toggle: setting/proxy mismatch"),
        }
    }

    /// Step an `Enum` setting by `dir` (each enum, `MainTtl`/`SubTtl`, has two
    /// values so any nonzero `dir` flips it). Debug-asserts the id is an `Enum`.
    pub fn cycle(self, s: &mut ProxySettings, dir: i8) {
        debug_assert_eq!(self.kind(), SettingKind::Enum, "cycle on non-enum setting");
        let _ = dir;
        match (self, s) {
            (SettingId::MainTtl, ProxySettings::Pino(p)) => {
                p.main_ttl = match p.main_ttl {
                    CacheTtl::FiveMin => CacheTtl::OneHour,
                    CacheTtl::OneHour => CacheTtl::FiveMin,
                };
            }
            (SettingId::SubTtl, ProxySettings::Pino(p)) => {
                p.sub_ttl = match p.sub_ttl {
                    CacheTtl::FiveMin => CacheTtl::OneHour,
                    CacheTtl::OneHour => CacheTtl::FiveMin,
                };
            }
            _ => debug_assert!(false, "cycle: setting/proxy mismatch"),
        }
    }

    /// The initial edit buffer for a `Text`/`List`/`Number` setting: the current
    /// value rendered as raw text (empty for an unset `Option`).
    pub fn edit_buffer(self, s: &ProxySettings) -> String {
        match (self, s) {
            (SettingId::ModelOverride, ProxySettings::Pino(p)) => {
                p.model_override.clone().unwrap_or_default()
            }
            (SettingId::DropTools, ProxySettings::Pino(p)) => p.drop_tools.join(","),
            (SettingId::Port, ProxySettings::Central(c)) => {
                c.port.map(|n| n.to_string()).unwrap_or_default()
            }
            (SettingId::PinnedVersion, ProxySettings::Central(c)) => {
                c.pinned_version.clone().unwrap_or_default()
            }
            _ => String::new(),
        }
    }

    /// Commit an editor buffer to the live settings. `Text`: empty ⇒ `None`.
    /// `List`: split on `,`, trim, drop empties. `Number`: empty ⇒ `None`, else
    /// `parse::<u16>()` (`Err("port must be 0–65535")` on garbage).
    pub fn commit_edit(self, s: &mut ProxySettings, buf: &str) -> anyhow::Result<()> {
        match (self, s) {
            (SettingId::ModelOverride, ProxySettings::Pino(p)) => {
                p.model_override = text_to_option(buf);
            }
            (SettingId::PinnedVersion, ProxySettings::Central(c)) => {
                c.pinned_version = text_to_option(buf);
            }
            (SettingId::DropTools, ProxySettings::Pino(p)) => {
                p.drop_tools = split_list(buf);
            }
            (SettingId::Port, ProxySettings::Central(c)) => {
                c.port = parse_port(buf)?;
            }
            _ => debug_assert!(false, "commit_edit: setting/proxy mismatch or non-editable"),
        }
        Ok(())
    }

    /// True when this setting's value is currently `[x]` (bool only).
    fn bool_value(self, s: &ProxySettings) -> bool {
        match (self, s) {
            (SettingId::AutoCache, ProxySettings::Pino(p)) => p.auto_cache,
            (SettingId::StripAnsi, ProxySettings::Pino(p)) => p.strip_ansi,
            (SettingId::Compression, ProxySettings::Headroom(h)) => h.compression,
            _ => false,
        }
    }
}

fn text_to_option(buf: &str) -> Option<String> {
    if buf.is_empty() {
        None
    } else {
        Some(buf.to_string())
    }
}

fn split_list(buf: &str) -> Vec<String> {
    buf.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

fn parse_port(buf: &str) -> anyhow::Result<Option<u16>> {
    if buf.is_empty() {
        return Ok(None);
    }
    buf.parse::<u16>()
        .map(Some)
        .map_err(|_| anyhow::anyhow!("port must be 0–65535"))
}

/// The widget string for a setting's current value: bool `[x]`/`[ ]`, enum
/// `‹ 5m ›`, list joined `,` or `(none)`, text/number value or `(default)`.
pub fn render_value(s: &ProxySettings, id: SettingId) -> String {
    match id.kind() {
        SettingKind::Bool => if id.bool_value(s) { "[x]" } else { "[ ]" }.to_string(),
        SettingKind::Enum => match (id, s) {
            (SettingId::MainTtl, ProxySettings::Pino(p)) => format!("‹ {} ›", p.main_ttl.as_str()),
            (SettingId::SubTtl, ProxySettings::Pino(p)) => format!("‹ {} ›", p.sub_ttl.as_str()),
            _ => String::new(),
        },
        SettingKind::List => match (id, s) {
            (SettingId::DropTools, ProxySettings::Pino(p)) => {
                if p.drop_tools.is_empty() {
                    "(none)".to_string()
                } else {
                    p.drop_tools.join(",")
                }
            }
            _ => "(none)".to_string(),
        },
        SettingKind::Text => match (id, s) {
            (SettingId::ModelOverride, ProxySettings::Pino(p)) => p
                .model_override
                .clone()
                .unwrap_or_else(|| "(default)".to_string()),
            (SettingId::PinnedVersion, ProxySettings::Central(c)) => c
                .pinned_version
                .clone()
                .unwrap_or_else(|| "(default)".to_string()),
            _ => "(default)".to_string(),
        },
        SettingKind::Number => match (id, s) {
            (SettingId::Port, ProxySettings::Central(c)) => c
                .port
                .map(|n| n.to_string())
                .unwrap_or_else(|| "(default)".to_string()),
            _ => "(default)".to_string(),
        },
    }
}

/// A short collapsed-row description that reflects live settings (spec §3.4).
///
/// pino: `cache · <main>/<sub>` (omit `cache` when `auto_cache` off) `(· drop N`
/// when `drop_tools` nonempty); headroom: `compression on`/`compression off`;
/// central: `JetBrains AI · :PORT` or `JetBrains AI`.
pub fn describe(name: ProxyName, s: &ProxySettings) -> String {
    match (name, s) {
        (ProxyName::Pino, ProxySettings::Pino(p)) => {
            let mut parts: Vec<String> = Vec::new();
            if p.auto_cache {
                parts.push("cache".to_string());
            }
            parts.push(format!("{}/{}", p.main_ttl.as_str(), p.sub_ttl.as_str()));
            if !p.drop_tools.is_empty() {
                parts.push(format!("drop {}", p.drop_tools.len()));
            }
            parts.join(" · ")
        }
        (ProxyName::Headroom, ProxySettings::Headroom(h)) => {
            if h.compression {
                "compression on".to_string()
            } else {
                "compression off".to_string()
            }
        }
        (ProxyName::Central, ProxySettings::Central(c)) => match c.port {
            Some(port) => format!("JetBrains AI · :{port}"),
            None => "JetBrains AI".to_string(),
        },
        // name/settings mismatch should never happen (validated upstream).
        _ => String::new(),
    }
}
