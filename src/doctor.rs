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

use std::fmt::Write as _;

/// The five target (os, arch) pairs we build single binaries for.
pub fn target_is_supported(os: &str, arch: &str) -> bool {
    matches!(
        (os, arch),
        ("windows", "x86_64")
            | ("macos", "x86_64")
            | ("macos", "aarch64")
            | ("linux", "x86_64")
            | ("linux", "aarch64")
    )
}

/// Whether a downloadable jbcentral asset exists for this (os, arch).
/// JetBrains ships no windows-arm64 asset; everything else we support has one.
pub fn central_asset_available(os: &str, arch: &str) -> bool {
    if (os, arch) == ("windows", "aarch64") {
        return false;
    }
    target_is_supported(os, arch)
}

/// Toolchain/target diagnostics for the given (os, arch). All findings are
/// `FindingDomain::Toolchain` with `layer: None`.
pub fn analyze_toolchain(os: &str, arch: &str) -> Vec<Finding> {
    let mut findings = Vec::new();
    if !target_is_supported(os, arch) {
        findings.push(Finding {
            domain: FindingDomain::Toolchain,
            layer: None,
            severity: Severity::Error,
            message: format!("unsupported build target {os}/{arch}"),
            found_value: None,
        });
    }
    if !central_asset_available(os, arch) {
        findings.push(Finding {
            domain: FindingDomain::Toolchain,
            layer: None,
            severity: Severity::Warn,
            message: format!(
                "no jbcentral asset for {os}/{arch}; the central proxy cannot be used on this platform"
            ),
            found_value: None,
        });
    }
    findings
}

/// Render findings, errors first then warnings; pure.
pub fn render_findings(findings: &[Finding]) -> String {
    if findings.is_empty() {
        return "doctor: no problems detected\n".to_string();
    }
    let mut out = String::new();
    for f in findings.iter().filter(|f| f.severity == Severity::Error) {
        let _ = writeln!(out, "ERROR: {}", f.message);
    }
    for f in findings.iter().filter(|f| f.severity == Severity::Warn) {
        let _ = writeln!(out, "WARN:  {}", f.message);
    }
    out
}

/// Gather real inputs and print diagnostics. Side-effecting entry point.
///
/// Returns `Ok(false)` when any `Severity::Error` finding exists, so the caller can
/// set a non-zero process exit code.
pub fn run_doctor() -> Result<bool> {
    let mut findings = Vec::new();

    // File-backed settings layers. `doctor` cannot know the ephemeral run port, so
    // any non-empty ANTHROPIC_BASE_URL is a conflict candidate: compare against an
    // impossible sentinel so every set value surfaces.
    for (layer, path) in settings_layer_paths()? {
        let source = read_settings_layer(layer, &path);
        findings.extend(analyze_base_url(&[source], "\u{0}none"));
    }

    findings.extend(analyze_toolchain(
        std::env::consts::OS,
        std::env::consts::ARCH,
    ));

    print!("{}", render_findings(&findings));
    Ok(!findings.iter().any(|f| f.severity == Severity::Error))
}
