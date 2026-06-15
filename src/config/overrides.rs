//! Partial, all-`Option` per-proxy setting overrides, parsed from `run` flags and
//! merged onto a `Config` via `Config::with_overrides`. `Some(field)` replaces the
//! base; `None` keeps it. Lists replace wholesale.

use crate::config::CentralSettings;
use crate::proxy::headroom::HeadroomSettings;
use crate::proxy::pino::{CacheTtl, PinoSettings};

#[cfg(test)]
#[path = "overrides_tests.rs"]
mod overrides_tests;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Overrides {
    pub pino: PinoOverride,
    pub headroom: HeadroomOverride,
    pub central: CentralOverride,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PinoOverride {
    pub auto_cache: Option<bool>,
    pub main_ttl: Option<CacheTtl>,
    pub sub_ttl: Option<CacheTtl>,
    pub drop_tools: Option<Vec<String>>,
    pub strip_ansi: Option<bool>,
    pub model_override: Option<String>,
}
impl PinoOverride {
    pub fn apply(&self, base: &mut PinoSettings) {
        if let Some(v) = self.auto_cache {
            base.auto_cache = v;
        }
        if let Some(v) = self.main_ttl {
            base.main_ttl = v;
        }
        if let Some(v) = self.sub_ttl {
            base.sub_ttl = v;
        }
        if let Some(v) = &self.drop_tools {
            base.drop_tools = v.clone();
        }
        if let Some(v) = self.strip_ansi {
            base.strip_ansi = v;
        }
        if let Some(v) = &self.model_override {
            base.model_override = Some(v.clone());
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct HeadroomOverride {
    pub compression: Option<bool>,
}
impl HeadroomOverride {
    pub fn apply(&self, base: &mut HeadroomSettings) {
        if let Some(v) = self.compression {
            base.compression = v;
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CentralOverride {
    pub port: Option<u16>,
    pub pinned_version: Option<String>,
}
impl CentralOverride {
    pub fn apply(&self, base: &mut CentralSettings) {
        if let Some(v) = self.port {
            base.port = Some(v);
        }
        if let Some(v) = &self.pinned_version {
            base.pinned_version = Some(v.clone());
        }
    }
}
