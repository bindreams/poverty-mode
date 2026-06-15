use super::*;
use crate::config::{CentralSettings, ProxySettings};
use crate::proxy::pino::{CacheTtl, PinoSettings};
use crate::proxy::ProxyName;

fn pino() -> ProxySettings {
    ProxySettings::Pino(PinoSettings {
        auto_cache: true,
        main_ttl: CacheTtl::OneHour,
        sub_ttl: CacheTtl::FiveMin,
        drop_tools: vec![],
        strip_ansi: true,
        model_override: None,
    })
}

#[test]
fn settings_of_lists_fields_in_fixed_order() {
    assert_eq!(
        settings_of(ProxyName::Pino),
        &[
            SettingId::AutoCache,
            SettingId::MainTtl,
            SettingId::SubTtl,
            SettingId::DropTools,
            SettingId::StripAnsi,
            SettingId::ModelOverride
        ]
    );
    assert_eq!(settings_of(ProxyName::Headroom), &[SettingId::Compression]);
    assert_eq!(
        settings_of(ProxyName::Central),
        &[SettingId::Port, SettingId::PinnedVersion]
    );
}
#[test]
fn kind_classifies_each_setting() {
    assert_eq!(SettingId::AutoCache.kind(), SettingKind::Bool);
    assert_eq!(SettingId::MainTtl.kind(), SettingKind::Enum);
    assert_eq!(SettingId::SubTtl.kind(), SettingKind::Enum);
    assert_eq!(SettingId::DropTools.kind(), SettingKind::List);
    assert_eq!(SettingId::ModelOverride.kind(), SettingKind::Text);
    assert_eq!(SettingId::Port.kind(), SettingKind::Number);
}
#[test]
fn toggle_bool_flips_value() {
    let mut s = pino();
    SettingId::AutoCache.toggle(&mut s);
    assert_eq!(render_value(&s, SettingId::AutoCache), "[ ]");
}
#[test]
fn cycle_enum_wraps() {
    // pino() seeds main=1h, sub=5m.
    // main_ttl flips 1h -> 5m -> 1h, independent of sub_ttl.
    let mut s = pino();
    SettingId::MainTtl.cycle(&mut s, 1);
    assert_eq!(render_value(&s, SettingId::MainTtl), "‹ 5m ›");
    SettingId::MainTtl.cycle(&mut s, 1);
    assert_eq!(render_value(&s, SettingId::MainTtl), "‹ 1h ›");
    // sub_ttl flips 5m -> 1h -> 5m, leaving main_ttl untouched.
    SettingId::SubTtl.cycle(&mut s, 1);
    assert_eq!(render_value(&s, SettingId::SubTtl), "‹ 1h ›");
    SettingId::SubTtl.cycle(&mut s, 1);
    assert_eq!(render_value(&s, SettingId::SubTtl), "‹ 5m ›");
    assert_eq!(render_value(&s, SettingId::MainTtl), "‹ 1h ›");
}
#[test]
fn render_defaults_for_text_and_list() {
    let s = pino();
    assert_eq!(render_value(&s, SettingId::ModelOverride), "(default)");
    assert_eq!(render_value(&s, SettingId::DropTools), "(none)");
}
#[test]
fn edit_buffer_and_commit_text() {
    let mut s = pino();
    assert_eq!(SettingId::ModelOverride.edit_buffer(&s), "");
    SettingId::ModelOverride.commit_edit(&mut s, "claude-opus-4-8").unwrap();
    assert_eq!(render_value(&s, SettingId::ModelOverride), "claude-opus-4-8");
}
#[test]
fn commit_list_splits_csv_and_drops_empties() {
    let mut s = pino();
    SettingId::DropTools.commit_edit(&mut s, "Bash, ,Edit,").unwrap();
    assert_eq!(render_value(&s, SettingId::DropTools), "Bash,Edit");
}
#[test]
fn commit_number_parses_empty_is_none_garbage_errs() {
    let mut s = ProxySettings::Central(CentralSettings {
        port: None,
        pinned_version: None,
    });
    SettingId::Port.commit_edit(&mut s, "9000").unwrap();
    assert_eq!(render_value(&s, SettingId::Port), "9000");
    assert!(SettingId::Port.commit_edit(&mut s, "abc").is_err());
    SettingId::Port.commit_edit(&mut s, "").unwrap();
    assert_eq!(render_value(&s, SettingId::Port), "(default)");
}
#[test]
fn describe_reflects_key_settings() {
    // collapsed-row description must reflect live settings (spec §3.4)
    let s = pino();
    let d = describe(ProxyName::Pino, &s);
    assert!(d.contains("1h/5m")); // main_ttl/sub_ttl
    assert!(d.to_lowercase().contains("cache")); // auto_cache on
    let hr_on = ProxySettings::Headroom(crate::proxy::headroom::HeadroomSettings { compression: true });
    assert!(describe(ProxyName::Headroom, &hr_on)
        .to_lowercase()
        .contains("compression"));
}
