use super::*;
use crate::config::{CentralSettings, ProxySettings, ResolvedProxy};
use crate::proxy::headroom::HeadroomSettings;
use crate::proxy::pino::{PinoSettings, TailTtl};
use crate::proxy::ProxyName;

pub(crate) fn pino_rp() -> ResolvedProxy {
    ResolvedProxy {
        name: ProxyName::Pino,
        settings: ProxySettings::Pino(PinoSettings {
            auto_cache: true,
            tail_ttl: TailTtl::FiveMin,
            drop_tools: vec![],
            strip_ansi: true,
            model_override: None,
        }),
    }
}

pub(crate) fn headroom_rp() -> ResolvedProxy {
    ResolvedProxy {
        name: ProxyName::Headroom,
        settings: ProxySettings::Headroom(HeadroomSettings { compression: false }),
    }
}

pub(crate) fn central_rp() -> ResolvedProxy {
    ResolvedProxy {
        name: ProxyName::Central,
        settings: ProxySettings::Central(CentralSettings {
            port: None,
            pinned_version: None,
        }),
    }
}

#[test]
fn serialize_chain_renders_lowercase_csv_in_order() {
    let chain = vec![pino_rp(), headroom_rp(), central_rp()];
    assert_eq!(serialize_chain(&chain), "pino,headroom,central");
}

#[test]
fn serialize_chain_single_proxy() {
    assert_eq!(serialize_chain(&[pino_rp()]), "pino");
}

#[test]
fn serialize_chain_empty_is_empty_string() {
    let empty: Vec<ResolvedProxy> = vec![];
    assert_eq!(serialize_chain(&empty), "");
}

#[test]
fn parse_chain_reads_names_in_order() {
    assert_eq!(
        parse_chain("pino,headroom,central"),
        vec!["pino", "headroom", "central"]
    );
}

#[test]
fn parse_chain_trims_whitespace_and_drops_empties() {
    assert_eq!(parse_chain("  pino , headroom "), vec!["pino", "headroom"]);
    let empty: Vec<String> = vec![];
    assert_eq!(parse_chain(""), empty);
    assert_eq!(parse_chain("   "), empty);
}

#[test]
fn serialize_then_parse_round_trips() {
    let chain = vec![pino_rp(), headroom_rp(), central_rp()];
    let s = serialize_chain(&chain);
    let names = parse_chain(&s);
    assert_eq!(names, vec!["pino", "headroom", "central"]);
}
