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

use crate::central::CentralInfo;

#[test]
fn tail_is_central_wire_url_when_central_is_tail() {
    let inputs = TailInputs {
        central: Some(CentralInfo {
            port: 19516,
            secret: "abc123".to_string(),
        }),
        preexisting_base_url: Some("https://user-gateway.example.com".to_string()),
    };
    let up = resolve_tail_upstream(&inputs).unwrap();
    // central wins over a pre-existing base url.
    assert_eq!(
        up.url.as_str(),
        "http://127.0.0.1:19516/wire/abc123/claude-code/anthropic"
    );
}

#[test]
fn tail_is_preexisting_base_url_when_no_central() {
    let inputs = TailInputs {
        central: None,
        preexisting_base_url: Some("https://user-gateway.example.com/".to_string()),
    };
    let up = resolve_tail_upstream(&inputs).unwrap();
    assert_eq!(up.url.as_str(), "https://user-gateway.example.com/");
}

#[test]
fn tail_is_preexisting_base_url_with_path_prefix_preserved() {
    let inputs = TailInputs {
        central: None,
        preexisting_base_url: Some("https://gw.example.com/proxy".to_string()),
    };
    let up = resolve_tail_upstream(&inputs).unwrap();
    assert_eq!(up.url.as_str(), "https://gw.example.com/proxy");
    assert_eq!(up.path_prefix(), "/proxy");
}

#[test]
fn tail_defaults_to_anthropic_when_no_central_and_no_preexisting() {
    let inputs = TailInputs {
        central: None,
        preexisting_base_url: None,
    };
    let up = resolve_tail_upstream(&inputs).unwrap();
    assert_eq!(up.url.as_str(), "https://api.anthropic.com/");
}

#[test]
fn tail_treats_empty_preexisting_as_unset() {
    // An empty/whitespace ANTHROPIC_BASE_URL is the same as not set -> default.
    let inputs = TailInputs {
        central: None,
        preexisting_base_url: Some("   ".to_string()),
    };
    let up = resolve_tail_upstream(&inputs).unwrap();
    assert_eq!(up.url.as_str(), "https://api.anthropic.com/");
}

#[test]
fn tail_errors_on_unparseable_preexisting_base_url() {
    let inputs = TailInputs {
        central: None,
        preexisting_base_url: Some("not a url".to_string()),
    };
    let err = resolve_tail_upstream(&inputs).unwrap_err();
    assert!(
        err.to_string()
            .to_lowercase()
            .contains("anthropic_base_url"),
        "error should name the offending env var: {err}"
    );
}
