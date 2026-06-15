use super::*;
use crate::proxy::ProxyName;
use crate::tui::reducer::settings::SettingId;

#[test]
fn buffer_accumulates_and_backspaces() {
    let mut e = EditState::new(ProxyName::Pino, SettingId::ModelOverride, "ab");
    e.push('c');
    e.push('d');
    assert_eq!(e.buffer(), "abcd");
    e.backspace();
    assert_eq!(e.buffer(), "abc");
}
