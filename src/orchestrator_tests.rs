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

fn get<'a>(env: &'a [(String, String)], key: &str) -> Option<&'a str> {
    env.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

#[test]
fn agent_env_always_sets_chain_and_tool_search() {
    let chain = vec![pino_rp(), headroom_rp()];
    let env = compute_agent_env(&chain, false);
    assert_eq!(get(&env, "POVERTY_PROXY_CHAIN"), Some("pino,headroom"));
    assert_eq!(get(&env, "ENABLE_TOOL_SEARCH"), Some("true"));
}

#[test]
fn agent_env_omits_auth_token_for_non_central_tail() {
    let chain = vec![pino_rp()];
    let env = compute_agent_env(&chain, false);
    assert_eq!(get(&env, "ANTHROPIC_AUTH_TOKEN"), None);
}

#[test]
fn agent_env_sets_wire_proxy_auth_token_for_central_tail() {
    let chain = vec![pino_rp(), central_rp()];
    let env = compute_agent_env(&chain, true);
    assert_eq!(get(&env, "ANTHROPIC_AUTH_TOKEN"), Some("wire-proxy"));
    assert_eq!(get(&env, "POVERTY_PROXY_CHAIN"), Some("pino,central"));
    assert_eq!(get(&env, "ENABLE_TOOL_SEARCH"), Some("true"));
}

#[test]
fn agent_env_never_includes_base_url_key() {
    // ANTHROPIC_BASE_URL is set by the Agent from its base_url arg, not here.
    let chain = vec![pino_rp(), central_rp()];
    let env = compute_agent_env(&chain, true);
    assert_eq!(get(&env, "ANTHROPIC_BASE_URL"), None);
}

#[test]
fn agent_env_for_empty_chain_has_empty_chain_value() {
    let chain: Vec<ResolvedProxy> = vec![];
    let env = compute_agent_env(&chain, false);
    assert_eq!(get(&env, "POVERTY_PROXY_CHAIN"), Some(""));
    assert_eq!(get(&env, "ENABLE_TOOL_SEARCH"), Some("true"));
    assert_eq!(get(&env, "ANTHROPIC_AUTH_TOKEN"), None);
}
