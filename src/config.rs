use crate::proxy::headroom::HeadroomSettings;
use crate::proxy::pino::{CacheTtl, PinoSettings};
use crate::proxy::ProxyName;

pub mod overrides;

/// The whole config file. `proxies` order is the default chain order.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub version: u32,
    pub proxies: Vec<ProxyEntry>,
    pub defaults: Defaults,
}

/// One proxy's persisted state: identity, enabled flag, and its settings.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProxyEntry {
    pub name: ProxyName,
    pub enabled: bool,
    pub settings: ProxySettings,
}

/// One proxy resolved for an actual run: its identity and the settings to apply,
/// sourced from the config entry for that proxy.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedProxy {
    pub name: ProxyName,
    pub settings: ProxySettings,
}

impl ProxyName {
    /// Parse a lowercase proxy name as used in CSV chains / config.
    fn parse_csv_token(s: &str) -> anyhow::Result<ProxyName> {
        match s {
            "pino" => Ok(ProxyName::Pino),
            "headroom" => Ok(ProxyName::Headroom),
            "central" => Ok(ProxyName::Central),
            other => {
                anyhow::bail!("unknown proxy name {other:?} (expected pino, headroom, or central)")
            }
        }
    }
}

fn parse_chain_csv(csv: &str) -> anyhow::Result<Vec<ProxyName>> {
    let trimmed = csv.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    trimmed
        .split(',')
        .map(|tok| ProxyName::parse_csv_token(tok.trim()))
        .collect()
}

/// Per-proxy settings. `untagged` so the `settings:` mapping is matched
/// structurally; `deny_unknown_fields` on each variant's struct (declared on the
/// settings types in `src/proxy/{pino,headroom}.rs` and on `CentralSettings`
/// below) keeps the match unambiguous. The authoritative discriminator is
/// `ProxyEntry::name`, cross-checked against the parsed variant in `validate`.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(untagged)]
pub enum ProxySettings {
    Pino(PinoSettings),
    Headroom(HeadroomSettings),
    Central(CentralSettings),
}

/// JB Central settings. `port: null` => use the jbcentral default / managed value;
/// `pinned_version: null` => use the poverty-mode default version.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CentralSettings {
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub pinned_version: Option<String>,
}

/// Global defaults not tied to a specific proxy.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Defaults {
    /// The `ENABLE_TOOL_SEARCH` value the orchestrator emits into the agent env
    /// (spec §5.8). Default `true`: a non-first-party base URL otherwise disables
    /// Claude Code MCP tool search, and our proxies forward `tool_reference`
    /// blocks. Setting it `false` disables MCP tool search through the proxy.
    pub enable_tool_search: bool,
}

impl Config {
    /// The safe no-op default written on first run: every known proxy listed in
    /// canonical order, all disabled, with sensible per-proxy settings.
    pub fn default_all_disabled() -> Config {
        Config {
            version: 1,
            proxies: vec![
                ProxyEntry {
                    name: ProxyName::Pino,
                    enabled: false,
                    settings: ProxySettings::Pino(PinoSettings {
                        auto_cache: true,
                        main_ttl: CacheTtl::OneHour,
                        sub_ttl: CacheTtl::FiveMin,
                        drop_tools: Vec::new(),
                        strip_ansi: true,
                        model_override: None,
                    }),
                },
                ProxyEntry {
                    name: ProxyName::Headroom,
                    enabled: false,
                    settings: ProxySettings::Headroom(HeadroomSettings { compression: true }),
                },
                ProxyEntry {
                    name: ProxyName::Central,
                    enabled: false,
                    settings: ProxySettings::Central(CentralSettings {
                        port: None,
                        pinned_version: None,
                    }),
                },
            ],
            defaults: Defaults {
                enable_tool_search: true,
            },
        }
    }

    /// Load the config from `paths::config_path()`. On first run (file absent),
    /// write `default_all_disabled` atomically and return it. On subsequent runs,
    /// parse and validate (settings variant matches `name`; `central` is last).
    pub fn load_or_create() -> anyhow::Result<Config> {
        let path = crate::paths::config_path()?;
        if !path.exists() {
            let cfg = Config::default_all_disabled();
            let yaml = serde_yaml::to_string(&cfg)
                .map_err(|e| anyhow::anyhow!("serializing default config: {e}"))?;
            crate::paths::atomic_write(&path, yaml.as_bytes())?;
            return Ok(cfg);
        }
        let text = std::fs::read_to_string(&path)
            .map_err(|e| anyhow::anyhow!("reading config {}: {e}", path.display()))?;
        let cfg: Config = serde_yaml::from_str(&text)
            .map_err(|e| anyhow::anyhow!("parsing config {}: {e}", path.display()))?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Validate invariants, then atomically write this config to
    /// `paths::config_path()` as YAML (temp file + same-dir rename; 0600 on POSIX).
    pub fn save(&self) -> anyhow::Result<()> {
        self.validate()?;
        let path = crate::paths::config_path()?;
        let yaml =
            serde_yaml::to_string(self).map_err(|e| anyhow::anyhow!("serializing config: {e}"))?;
        crate::paths::atomic_write(&path, yaml.as_bytes())?;
        Ok(())
    }

    /// Resolve the ordered, enabled chain to run.
    ///
    /// Precedence: `cli_proxies` (explicit) > `env_chain` (POVERTY_PROXY_CHAIN
    /// CSV) > config-file order (enabled entries only).
    ///
    /// `central` must be last on every source: the config-file source is checked
    /// via `validate()` (which also enforces name/settings agreement), and the
    /// explicit cli/env order is checked directly (a non-last central requested
    /// explicitly is a hard error). For cli/env sources every requested name must
    /// have a matching entry in this config (settings come from there) and names
    /// must be unique.
    pub fn resolve_chain(
        &self,
        cli_proxies: Option<&[ProxyName]>,
        env_chain: Option<&str>,
    ) -> anyhow::Result<Vec<ResolvedProxy>> {
        // The config's own invariants (name/settings agreement, central-last) must
        // hold for EVERY source, including a directly-constructed in-memory Config
        // resolved via the file branch. Validate up front so the file source can
        // never silently yield a central-not-last chain.
        self.validate()?;

        // 1. Pick the requested order by precedence.
        let requested: Vec<ProxyName> = if let Some(cli) = cli_proxies {
            cli.to_vec()
        } else if let Some(env) = env_chain {
            parse_chain_csv(env)?
        } else {
            // Config-file source: enabled entries in file order (central already
            // validated last by self.validate()).
            return Ok(self
                .proxies
                .iter()
                .filter(|e| e.enabled)
                .map(|e| ResolvedProxy {
                    name: e.name,
                    settings: e.settings.clone(),
                })
                .collect());
        };

        // 2. Reject duplicates.
        let mut seen = std::collections::HashSet::new();
        for name in &requested {
            if !seen.insert(*name) {
                anyhow::bail!("duplicate proxy {:?} in requested chain", name);
            }
        }

        // 3. Central-last rule for explicit requests: a central anywhere but the
        //    final slot is a user error.
        if let Some(pos) = requested.iter().position(|n| n.must_be_last()) {
            if pos != requested.len() - 1 {
                anyhow::bail!(
                    "central must be last in the chain; it was requested at position {} of {}",
                    pos + 1,
                    requested.len()
                );
            }
        }

        // 4. Map each requested name to its config settings (error if absent).
        let mut resolved = Vec::with_capacity(requested.len());
        for name in requested {
            let entry = self
                .proxies
                .iter()
                .find(|e| e.name == name)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "proxy {:?} was requested but has no entry in the config (no settings available)",
                        name
                    )
                })?;
            resolved.push(ResolvedProxy {
                name,
                settings: entry.settings.clone(),
            });
        }
        Ok(resolved)
    }

    /// Clone with each entry's settings merged with the matching override. Enabled
    /// flags and order are untouched.
    pub fn with_overrides(&self, ov: &overrides::Overrides) -> Config {
        let mut next = self.clone();
        for entry in &mut next.proxies {
            match &mut entry.settings {
                ProxySettings::Pino(s) => ov.pino.apply(s),
                ProxySettings::Headroom(s) => ov.headroom.apply(s),
                ProxySettings::Central(s) => ov.central.apply(s),
            }
        }
        next
    }

    /// Persist a fully-specified ordered proxy list (the picker's complete state).
    /// Validates the same invariants as `save` (settings/name agreement, central
    /// last) and writes atomically.
    pub fn save_full_state(&self, proxies: Vec<ProxyEntry>) -> anyhow::Result<()> {
        Config {
            version: self.version,
            proxies,
            defaults: self.defaults.clone(),
        }
        .save()
    }

    /// Full ordered proxy list for a resolved chain: enabled members in chain order
    /// (carrying chain settings), then remaining known proxies disabled in this
    /// config's order keeping their settings; central forced last. The single
    /// ordering authority for the non-interactive `--save` path, producing the same
    /// `Vec<ProxyEntry>` the picker yields for an equivalent state.
    pub fn entries_for_chain(&self, chain: &[ResolvedProxy]) -> Vec<ProxyEntry> {
        let in_chain: std::collections::HashSet<ProxyName> = chain.iter().map(|r| r.name).collect();
        let mut proxies: Vec<ProxyEntry> = Vec::new();
        let mut central: Option<ProxyEntry> = None;
        for r in chain {
            let e = ProxyEntry {
                name: r.name,
                enabled: true,
                settings: r.settings.clone(),
            };
            if r.name.must_be_last() {
                central = Some(e);
            } else {
                proxies.push(e);
            }
        }
        for entry in &self.proxies {
            if in_chain.contains(&entry.name) {
                continue;
            }
            let e = ProxyEntry {
                name: entry.name,
                enabled: false,
                settings: entry.settings.clone(),
            };
            if entry.name.must_be_last() {
                central = Some(e);
            } else {
                proxies.push(e);
            }
        }
        if let Some(c) = central {
            proxies.push(c);
        }
        proxies
    }

    /// Validate the config invariants: each entry's settings variant matches its
    /// declared `name`, and `central` (if present) is the last entry. This is the
    /// single source of truth for the invariants enforced by `load_or_create`,
    /// `save`, and the file-source branch of `resolve_chain`.
    fn validate(&self) -> anyhow::Result<()> {
        for entry in &self.proxies {
            let ok = matches!(
                (entry.name, &entry.settings),
                (ProxyName::Pino, ProxySettings::Pino(_))
                    | (ProxyName::Headroom, ProxySettings::Headroom(_))
                    | (ProxyName::Central, ProxySettings::Central(_))
            );
            if !ok {
                anyhow::bail!(
                    "config error: proxy {:?} has settings that do not match its name (settings mismatch)",
                    entry.name
                );
            }
        }
        if let Some(pos) = self
            .proxies
            .iter()
            .position(|e| e.name == ProxyName::Central)
        {
            if pos != self.proxies.len() - 1 {
                anyhow::bail!("config error: central must be last in the proxies list");
            }
        }
        Ok(())
    }
}

// `config` subcommand support =====

/// Render a [`Config`] as the YAML text shown by `config show`. This is the same
/// serialization `save` writes to disk, so `show` reflects exactly what a later
/// `save` would persist (round-trips through serde).
pub fn render_config(cfg: &Config) -> anyhow::Result<String> {
    serde_yaml::to_string(cfg).map_err(|e| anyhow::anyhow!("serializing config: {e}"))
}

/// Resolve the editor command line for `config edit`, as a non-empty argv.
///
/// Precedence: `$VISUAL`, then `$EDITOR`, then a platform fallback (`notepad` on
/// Windows, `vi` elsewhere). An env var set to whitespace-only is treated as
/// unset. The value is split on ASCII whitespace so `EDITOR="code --wait"` works;
/// the config-file path is appended by the caller as a separate argv element.
pub fn resolve_editor(visual_env: Option<&str>, editor_env: Option<&str>) -> Vec<String> {
    for candidate in [visual_env, editor_env].into_iter().flatten() {
        let parts: Vec<String> = candidate.split_whitespace().map(str::to_string).collect();
        if !parts.is_empty() {
            return parts;
        }
    }
    vec![default_editor().to_string()]
}

/// The platform editor used when neither `$VISUAL` nor `$EDITOR` is set.
fn default_editor() -> &'static str {
    if cfg!(windows) {
        "notepad"
    } else {
        "vi"
    }
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod config_tests;
