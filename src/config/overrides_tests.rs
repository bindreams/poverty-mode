use super::*;
use crate::config::CentralSettings;
use crate::proxy::headroom::HeadroomSettings;
use crate::proxy::pino::{CacheTtl, PinoSettings};

fn base_pino() -> PinoSettings {
    PinoSettings {
        auto_cache: true,
        main_ttl: CacheTtl::OneHour,
        sub_ttl: CacheTtl::FiveMin,
        drop_tools: vec![],
        strip_ansi: true,
        model_override: None,
    }
}

#[test]
fn empty_override_is_identity() {
    let mut s = base_pino();
    PinoOverride::default().apply(&mut s);
    assert_eq!(s, base_pino());
}

#[test]
fn pino_override_sets_only_present_fields() {
    let mut s = base_pino();
    PinoOverride {
        sub_ttl: Some(CacheTtl::OneHour),
        auto_cache: Some(false),
        ..Default::default()
    }
    .apply(&mut s);
    assert_eq!(s.sub_ttl, CacheTtl::OneHour);
    // main_ttl was not overridden, so it keeps the base value.
    assert_eq!(s.main_ttl, CacheTtl::OneHour);
    assert!(!s.auto_cache);
    assert!(s.strip_ansi);
}

#[test]
fn pino_override_applies_both_ttls_independently() {
    let mut s = base_pino(); // main=1h, sub=5m
    PinoOverride {
        main_ttl: Some(CacheTtl::FiveMin),
        sub_ttl: Some(CacheTtl::OneHour),
        ..Default::default()
    }
    .apply(&mut s);
    assert_eq!(s.main_ttl, CacheTtl::FiveMin);
    assert_eq!(s.sub_ttl, CacheTtl::OneHour);
}

#[test]
fn pino_override_replaces_drop_tools_list() {
    let mut s = base_pino();
    s.drop_tools = vec!["x".into()];
    PinoOverride {
        drop_tools: Some(vec!["a".into(), "b".into()]),
        ..Default::default()
    }
    .apply(&mut s);
    assert_eq!(s.drop_tools, vec!["a".to_string(), "b".to_string()]);
}

#[test]
fn headroom_override_sets_compression() {
    let mut s = HeadroomSettings { compression: true };
    HeadroomOverride {
        compression: Some(false),
    }
    .apply(&mut s);
    assert!(!s.compression);
}

#[test]
fn empty_headroom_override_keeps_base() {
    let mut s = HeadroomSettings { compression: true };
    HeadroomOverride::default().apply(&mut s);
    assert!(s.compression);
}

#[test]
fn empty_central_override_keeps_base() {
    let mut s = CentralSettings {
        port: Some(7000),
        pinned_version: Some("9.9.9".into()),
        executable: None,
    };
    CentralOverride::default().apply(&mut s);
    assert_eq!(s.port, Some(7000));
    assert_eq!(s.pinned_version.as_deref(), Some("9.9.9"));
}

#[test]
fn central_override_sets_port_and_version() {
    let mut s = CentralSettings {
        port: None,
        pinned_version: None,
        executable: None,
    };
    CentralOverride {
        port: Some(9000),
        pinned_version: Some("1.2.3".into()),
        executable: None,
    }
    .apply(&mut s);
    assert_eq!(s.port, Some(9000));
    assert_eq!(s.pinned_version.as_deref(), Some("1.2.3"));
}

#[test]
fn central_override_sets_and_clears_executable() {
    let mut s = CentralSettings {
        port: None,
        pinned_version: None,
        executable: Some("jbcentral".into()),
    };
    CentralOverride {
        port: None,
        pinned_version: None,
        executable: Some("/opt/jb".into()),
    }
    .apply(&mut s);
    assert_eq!(s.executable.as_deref(), Some("/opt/jb"));
    // Empty string clears to Download mode (None).
    CentralOverride {
        port: None,
        pinned_version: None,
        executable: Some(String::new()),
    }
    .apply(&mut s);
    assert_eq!(s.executable, None);
    // A whitespace-only value clears too (trimmed before the empty check).
    s.executable = Some("jbcentral".into());
    CentralOverride {
        port: None,
        pinned_version: None,
        executable: Some("   ".into()),
    }
    .apply(&mut s);
    assert_eq!(s.executable, None);
}
