use super::*;
use crate::proxy::ProxyName;

#[test]
fn visible_collapsed_is_headers_then_buttons() {
    let rows = [(ProxyName::Pino, false), (ProxyName::Headroom, false)];
    assert_eq!(
        visible_focus(&rows),
        vec![
            Focus::Proxy(ProxyName::Pino),
            Focus::Proxy(ProxyName::Headroom),
            Focus::Start,
            Focus::Cancel,
        ]
    );
}
#[test]
fn visible_expands_settings_under_proxy() {
    let rows = [(ProxyName::Headroom, true), (ProxyName::Pino, false)];
    assert_eq!(
        visible_focus(&rows),
        vec![
            Focus::Proxy(ProxyName::Headroom),
            Focus::Setting(ProxyName::Headroom, SettingId::Compression),
            Focus::Proxy(ProxyName::Pino),
            Focus::Start,
            Focus::Cancel,
        ]
    );
}
#[test]
fn next_prev_step_and_clamp() {
    let rows = [(ProxyName::Pino, false)];
    let vis = visible_focus(&rows); // [Proxy(Pino), Start, Cancel]
    assert_eq!(next_focus(&vis, Focus::Proxy(ProxyName::Pino)), Focus::Start);
    assert_eq!(next_focus(&vis, Focus::Cancel), Focus::Cancel);
    assert_eq!(
        prev_focus(&vis, Focus::Proxy(ProxyName::Pino)),
        Focus::Proxy(ProxyName::Pino)
    );
    assert_eq!(prev_focus(&vis, Focus::Start), Focus::Proxy(ProxyName::Pino));
}
