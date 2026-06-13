use crate::proxy::headroom::HeadroomSettings;
use crate::proxy::pino::{PinoSettings, TailTtl};
use crate::proxy::ProxyName;

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
                        tail_ttl: TailTtl::FiveMin,
                        drop_tools: Vec::new(),
                        strip_ansi: true,
                        model_override: None,
                    }),
                },
                ProxyEntry {
                    name: ProxyName::Headroom,
                    enabled: false,
                    settings: ProxySettings::Headroom(HeadroomSettings {
                        compression: false,
                    }),
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
        if let Some(pos) = self.proxies.iter().position(|e| e.name == ProxyName::Central) {
            if pos != self.proxies.len() - 1 {
                anyhow::bail!("config error: central must be last in the proxies list");
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod config_tests;
