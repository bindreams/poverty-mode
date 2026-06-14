//! `poverty-mode doctor`: diagnostics for settings.json conflicts and toolchain.

use std::path::PathBuf;

use anyhow::Result;

#[cfg(test)]
#[path = "doctor_tests.rs"]
mod doctor_tests;

/// Which Claude settings layer a finding came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsLayer {
    /// Enterprise Managed policy -- the only layer that overrides our injection.
    Managed,
    /// `~/.claude/settings.json`.
    UserSettings,
    /// project `.claude/settings.json`.
    ProjectSettings,
    /// project `.claude/settings.local.json`.
    ProjectLocalSettings,
}

impl SettingsLayer {
    pub fn label(self) -> &'static str {
        match self {
            SettingsLayer::Managed => "managed policy",
            SettingsLayer::UserSettings => "~/.claude/settings.json",
            SettingsLayer::ProjectSettings => ".claude/settings.json",
            SettingsLayer::ProjectLocalSettings => ".claude/settings.local.json",
        }
    }
}

/// The domain a `Finding` belongs to. Keeps settings-layer findings and toolchain
/// findings distinct so neither is misclassified as the other.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FindingDomain {
    Settings,
    Toolchain,
}

/// A parsed settings layer. `json == None` means the file was absent/unreadable.
#[derive(Clone, Debug)]
pub struct SettingsSource {
    pub layer: SettingsLayer,
    pub json: Option<serde_json::Value>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Warn,
    Error,
}

/// A single diagnostic finding. `layer` is `Some` only for `Settings`-domain
/// findings; `Toolchain` findings carry `layer: None`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    pub domain: FindingDomain,
    pub layer: Option<SettingsLayer>,
    pub severity: Severity,
    pub message: String,
    pub found_value: Option<String>,
}

const BASE_URL_KEY: &str = "ANTHROPIC_BASE_URL";

/// Extract a string `ANTHROPIC_BASE_URL` from a JSON object at a given location,
/// returning `None` if absent or not a string.
fn read_base_url(obj: &serde_json::Value, in_env: bool) -> Option<String> {
    let target = if in_env { obj.get("env")? } else { obj };
    target
        .get(BASE_URL_KEY)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Detect conflicting/Managed `ANTHROPIC_BASE_URL` across settings layers.
///
/// `ours` is the URL `poverty-mode` will inject; a layer carrying exactly that
/// value is not a conflict. Both the top-level key and the `env` block are checked.
/// Managed-layer hits are `Severity::Error` (cannot be overridden); all other
/// layers are `Severity::Warn` (our `--settings` injection wins at CLI precedence).
pub fn analyze_base_url(sources: &[SettingsSource], ours: &str) -> Vec<Finding> {
    let mut findings = Vec::new();
    for source in sources {
        let json = match &source.json {
            Some(j) => j,
            None => continue,
        };
        for in_env in [false, true] {
            if let Some(found) = read_base_url(json, in_env) {
                if found == ours {
                    continue;
                }
                let severity = if source.layer == SettingsLayer::Managed {
                    Severity::Error
                } else {
                    Severity::Warn
                };
                let location = if in_env { "env block" } else { "top level" };
                let message = if source.layer == SettingsLayer::Managed {
                    format!(
                        "{BASE_URL_KEY} is set by managed policy ({location}); \
                         poverty-mode cannot override it and the chain will be bypassed"
                    )
                } else {
                    format!(
                        "{BASE_URL_KEY} is set in {} ({location}); \
                         poverty-mode overrides it via --settings, but verify this is intended",
                        source.layer.label()
                    )
                };
                findings.push(Finding {
                    domain: FindingDomain::Settings,
                    layer: Some(source.layer),
                    severity,
                    message,
                    found_value: Some(found),
                });
            }
        }
    }
    findings
}

/// Read a settings layer from disk into a `SettingsSource` (absent/invalid -> None json).
pub fn read_settings_layer(layer: SettingsLayer, path: &std::path::Path) -> SettingsSource {
    let json = std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok());
    SettingsSource { layer, json }
}

/// Resolve the on-disk paths of the settings layers `doctor` inspects.
pub fn settings_layer_paths() -> Result<Vec<(SettingsLayer, PathBuf)>> {
    let mut out = Vec::new();
    if let Some(base) = directories::BaseDirs::new() {
        out.push((
            SettingsLayer::UserSettings,
            base.home_dir().join(".claude").join("settings.json"),
        ));
    }
    let cwd = std::env::current_dir()?;
    out.push((
        SettingsLayer::ProjectSettings,
        cwd.join(".claude").join("settings.json"),
    ));
    out.push((
        SettingsLayer::ProjectLocalSettings,
        cwd.join(".claude").join("settings.local.json"),
    ));
    Ok(out)
}
