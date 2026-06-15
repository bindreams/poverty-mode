use super::*;
use http::HeaderMap;

// ---- Request classification helpers (M3.1) ----

#[test]
fn messages_path_matches_exact_and_query_and_count_tokens() {
    assert!(is_messages_path("/v1/messages"));
    assert!(is_messages_path("/v1/messages?beta=true"));
    assert!(is_messages_path("/v1/messages/count_tokens"));
    assert!(is_messages_path("/v1/messages/count_tokens?x=1"));
}

#[test]
fn messages_path_rejects_other_paths() {
    assert!(!is_messages_path("/v1/complete"));
    assert!(!is_messages_path("/__pm/health"));
    assert!(!is_messages_path("/v1/messagesX"));
    assert!(!is_messages_path("/v1/messages/count_tokensX"));
    assert!(!is_messages_path("/"));
    assert!(!is_messages_path(""));
}

#[test]
fn json_content_type_detection() {
    let mut h = HeaderMap::new();
    h.insert("content-type", "application/json".parse().unwrap());
    assert!(is_json_content_type(&h));

    let mut h2 = HeaderMap::new();
    h2.insert(
        "content-type",
        "application/json; charset=utf-8".parse().unwrap(),
    );
    assert!(is_json_content_type(&h2));

    let mut h3 = HeaderMap::new();
    h3.insert("content-type", "APPLICATION/JSON".parse().unwrap());
    assert!(is_json_content_type(&h3), "case-insensitive");

    let mut h4 = HeaderMap::new();
    h4.insert("content-type", "text/event-stream".parse().unwrap());
    assert!(!is_json_content_type(&h4));

    let empty = HeaderMap::new();
    assert!(!is_json_content_type(&empty));
}

// ---- ProxyName ----

#[test]
fn proxy_name_as_str_kind_and_last() {
    assert_eq!(ProxyName::Pino.as_str(), "pino");
    assert_eq!(ProxyName::Headroom.as_str(), "headroom");
    assert_eq!(ProxyName::Central.as_str(), "central");

    assert_eq!(ProxyName::Pino.kind(), ProxyKind::FirstParty);
    assert_eq!(ProxyName::Headroom.kind(), ProxyKind::FirstParty);
    assert_eq!(ProxyName::Central.kind(), ProxyKind::External);

    assert!(!ProxyName::Pino.must_be_last());
    assert!(!ProxyName::Headroom.must_be_last());
    assert!(ProxyName::Central.must_be_last());
}

#[test]
fn proxy_name_health_path_first_party_vs_central() {
    assert_eq!(ProxyName::Pino.health_path(), "/__pm/health");
    assert_eq!(ProxyName::Headroom.health_path(), "/__pm/health");
    assert_eq!(ProxyName::Central.health_path(), "/health");
}

#[test]
fn proxy_name_from_str_roundtrip_and_reject() {
    assert_eq!("pino".parse::<ProxyName>().unwrap(), ProxyName::Pino);
    assert_eq!(
        "headroom".parse::<ProxyName>().unwrap(),
        ProxyName::Headroom
    );
    assert_eq!("central".parse::<ProxyName>().unwrap(), ProxyName::Central);
    assert!("nope".parse::<ProxyName>().is_err());
}

// ---- Upstream::host_header ----

#[test]
fn host_header_elides_default_https_port() {
    let u = Upstream::parse("https://api.anthropic.com/v1").unwrap();
    assert_eq!(u.host_header(), "api.anthropic.com");
}

#[test]
fn host_header_elides_default_http_port() {
    let u = Upstream::parse("http://example.com/").unwrap();
    assert_eq!(u.host_header(), "example.com");
}

#[test]
fn host_header_preserves_explicit_non_default_port() {
    let u = Upstream::parse("http://127.0.0.1:8787/").unwrap();
    assert_eq!(u.host_header(), "127.0.0.1:8787");
}

#[test]
fn host_header_preserves_explicit_port_that_is_other_schemes_default() {
    // 443 on an http:// URL is non-default for http, so it must be preserved.
    let u = Upstream::parse("http://localhost:443/").unwrap();
    assert_eq!(u.host_header(), "localhost:443");
    // 80 on an https:// URL is non-default for https, so it must be preserved.
    let u = Upstream::parse("https://localhost:80/").unwrap();
    assert_eq!(u.host_header(), "localhost:80");
}

// ---- Upstream::path_prefix ----

#[test]
fn path_prefix_strips_single_trailing_slash() {
    let u = Upstream::parse("https://api.anthropic.com/").unwrap();
    assert_eq!(u.path_prefix(), "");

    let u = Upstream::parse("http://127.0.0.1:9000/wire/SECRET/claude-code/anthropic").unwrap();
    assert_eq!(u.path_prefix(), "/wire/SECRET/claude-code/anthropic");

    let u = Upstream::parse("http://127.0.0.1:9000/wire/SECRET/anthropic/").unwrap();
    assert_eq!(u.path_prefix(), "/wire/SECRET/anthropic");
}

#[test]
fn path_prefix_root_is_empty() {
    let u = Upstream::parse("http://localhost:1234").unwrap();
    assert_eq!(u.path_prefix(), "");
}

// ---- TransformKind ----

#[test]
fn transform_kind_variants_exist() {
    use crate::proxy::headroom::HeadroomSettings;
    use crate::proxy::pino::{CacheTtl, PinoSettings};
    let kinds = [
        TransformKind::None,
        TransformKind::Pino(PinoSettings {
            auto_cache: false,
            main_ttl: CacheTtl::OneHour,
            sub_ttl: CacheTtl::FiveMin,
            drop_tools: vec![],
            strip_ansi: false,
            model_override: None,
        }),
        TransformKind::Headroom(HeadroomSettings { compression: false }),
    ];
    assert_eq!(kinds.len(), 3);
}

// ---- BodyTransform default apply_headers is a no-op (R6) ----

struct IdentityTransform;
impl BodyTransform for IdentityTransform {
    fn transform(
        &self,
        _body: &mut serde_json::Value,
        _ctx: &RequestContext,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}

#[test]
fn body_transform_default_apply_headers_is_noop() {
    let t = IdentityTransform;
    let mut headers = http::HeaderMap::new();
    headers.insert("x-keep", http::HeaderValue::from_static("1"));
    t.apply_headers(&mut headers);
    assert_eq!(headers.len(), 1);
    assert_eq!(headers.get("x-keep").unwrap(), "1");
}

// ---- Wire types serialize with the documented field names (R10) ----

#[test]
fn ready_line_serializes_with_run_id() {
    let r = ReadyLine {
        ready: true,
        port: 5050,
        proxy: "pino".to_string(),
        run_id: "01ARZ".to_string(),
    };
    let json = serde_json::to_value(&r).unwrap();
    assert_eq!(json["ready"], serde_json::json!(true));
    assert_eq!(json["port"], serde_json::json!(5050));
    assert_eq!(json["proxy"], serde_json::json!("pino"));
    assert_eq!(json["run_id"], serde_json::json!("01ARZ"));
}

#[test]
fn health_body_serializes_with_run_id_and_upstream() {
    let h = HealthBody {
        proxy: "headroom".to_string(),
        port: 6060,
        upstream: "api.anthropic.com".to_string(),
        run_id: "01ARZ".to_string(),
    };
    let json = serde_json::to_value(&h).unwrap();
    assert_eq!(json["proxy"], serde_json::json!("headroom"));
    assert_eq!(json["port"], serde_json::json!(6060));
    assert_eq!(json["upstream"], serde_json::json!("api.anthropic.com"));
    assert_eq!(json["run_id"], serde_json::json!("01ARZ"));
}

// CHARACTERIZATION GUARD (not TDD): pins the EXACT compact wire string the M3
// engine prints as its READY line (a single serde_json::to_string with the
// documented field order). The neighboring `ready_line_serializes_with_run_id`
// only checks fields via to_value; this guards the byte-exact line shape the
// orchestrator parses (R10/§9). Added after the M1 type already exists.
#[test]
fn ready_line_serializes_compactly_with_run_id() {
    let rl = ReadyLine {
        ready: true,
        port: 54321,
        proxy: "pino".to_string(),
        run_id: "01J0RUNID".to_string(),
    };
    let s = serde_json::to_string(&rl).unwrap();
    assert_eq!(
        s,
        r#"{"ready":true,"port":54321,"proxy":"pino","run_id":"01J0RUNID"}"#
    );
    let back: ReadyLine = serde_json::from_str(&s).unwrap();
    assert_eq!(back.port, 54321);
    assert_eq!(back.proxy, "pino");
    assert_eq!(back.run_id, "01J0RUNID");
    assert!(back.ready);
}

// CHARACTERIZATION GUARD (not TDD): full string round-trip of the health body
// the M3 engine serves at `/__pm/health` (R10/§5.4). The neighboring
// `health_body_serializes_with_run_id_and_upstream` only checks to_value; this
// guards that to_string -> from_str preserves every field. Added after the M1
// type already exists.
#[test]
fn health_body_round_trips_with_run_id() {
    let hb = HealthBody {
        proxy: "headroom".to_string(),
        port: 8080,
        upstream: "api.anthropic.com".to_string(),
        run_id: "01J0RUNID".to_string(),
    };
    let s = serde_json::to_string(&hb).unwrap();
    let back: HealthBody = serde_json::from_str(&s).unwrap();
    assert_eq!(back.proxy, "headroom");
    assert_eq!(back.port, 8080);
    assert_eq!(back.upstream, "api.anthropic.com");
    assert_eq!(back.run_id, "01J0RUNID");
}

#[test]
fn engine_config_holds_run_id_and_transform_kind() {
    use crate::proxy::pino::{CacheTtl, PinoSettings};
    let settings = PinoSettings {
        auto_cache: false,
        main_ttl: CacheTtl::OneHour,
        sub_ttl: CacheTtl::FiveMin,
        drop_tools: vec![],
        strip_ansi: false,
        model_override: None,
    };
    let cfg = EngineConfig {
        name: ProxyName::Pino,
        listen: "127.0.0.1:0".parse().unwrap(),
        upstream: Upstream::parse("https://api.anthropic.com").unwrap(),
        run_id: "01ARZ".to_string(),
        log_file: None,
        transform: TransformKind::Pino(settings.clone()),
    };
    assert_eq!(cfg.name, ProxyName::Pino);
    assert_eq!(cfg.run_id, "01ARZ");
    assert_eq!(cfg.transform, TransformKind::Pino(settings));
    assert_eq!(cfg.upstream.host_header(), "api.anthropic.com");
}

// ---- upstream_target_uri (M3.2) ----

fn up(s: &str) -> Upstream {
    Upstream {
        url: url::Url::parse(s).unwrap(),
    }
}

#[test]
fn target_uri_prepends_path_prefix_for_secret_wire_path() {
    let u = up("http://127.0.0.1:9999/wire/SECRET/claude-code/anthropic");
    let got = upstream_target_uri(&u, "/v1/messages").unwrap();
    assert_eq!(
        got.to_string(),
        "http://127.0.0.1:9999/wire/SECRET/claude-code/anthropic/v1/messages"
    );
}

#[test]
fn target_uri_bare_upstream_has_no_prefix() {
    let u = up("https://api.anthropic.com");
    let got = upstream_target_uri(&u, "/v1/messages").unwrap();
    assert_eq!(got.to_string(), "https://api.anthropic.com/v1/messages");
}

#[test]
fn target_uri_strips_trailing_slash_and_elides_default_port() {
    // http default port :80 is elided by host_header() (JS URL.host parity), and
    // the prefix's trailing slash is stripped by path_prefix().
    let u = up("http://127.0.0.1:80/prefix/");
    let got = upstream_target_uri(&u, "/v1/messages/count_tokens").unwrap();
    assert_eq!(
        got.to_string(),
        "http://127.0.0.1/prefix/v1/messages/count_tokens"
    );
}

#[test]
fn target_uri_preserves_inbound_query_string() {
    let u = up("https://api.anthropic.com");
    let got = upstream_target_uri(&u, "/v1/messages?beta=true").unwrap();
    assert_eq!(
        got.to_string(),
        "https://api.anthropic.com/v1/messages?beta=true"
    );
}

#[test]
fn target_uri_rejects_upstream_with_userinfo() {
    let u = up("http://user:pass@127.0.0.1:9999/prefix");
    let err = upstream_target_uri(&u, "/v1/messages").unwrap_err();
    assert!(
        err.to_string().contains("userinfo"),
        "upstream with userinfo must be rejected, got: {err}"
    );
}

#[test]
fn target_uri_rejects_upstream_with_query() {
    let u = up("http://127.0.0.1:9999/prefix?leftover=1");
    let err = upstream_target_uri(&u, "/v1/messages").unwrap_err();
    assert!(
        err.to_string().contains("query"),
        "upstream with a query string must be rejected, got: {err}"
    );
}

// ---- Upstream host_header/path_prefix characterization guard (M3.2b) ----

// CHARACTERIZATION GUARD (not TDD): re-pins the M1 Upstream contract that the
// M3 engine depends on. Grounded in reference/pino/src/config.js:24-30 —
// `hostHeader = url.host` (JS URL.host elides default ports, keeps explicit
// non-default ports) and `pathPrefix = url.pathname.replace(/\/+$/, "")` with
// "/" normalized to "". Expected to pass on first run because M1 already
// implements and tests this; here it guards M3's reliance on it.
#[test]
fn guard_host_header_elides_default_ports_keeps_explicit() {
    assert_eq!(
        up("http://api.anthropic.com").host_header(),
        "api.anthropic.com"
    );
    assert_eq!(
        up("https://api.anthropic.com").host_header(),
        "api.anthropic.com"
    );
    assert_eq!(
        up("http://api.anthropic.com:80").host_header(),
        "api.anthropic.com"
    );
    assert_eq!(
        up("https://api.anthropic.com:443").host_header(),
        "api.anthropic.com"
    );
    assert_eq!(up("http://127.0.0.1:9999").host_header(), "127.0.0.1:9999");
    assert_eq!(
        up("https://example.com:8443").host_header(),
        "example.com:8443"
    );
}

#[test]
fn guard_path_prefix_strips_trailing_slash_and_normalizes_root() {
    assert_eq!(up("https://api.anthropic.com").path_prefix(), "");
    assert_eq!(up("https://api.anthropic.com/").path_prefix(), "");
    assert_eq!(up("http://127.0.0.1:9999/prefix/").path_prefix(), "/prefix");
    assert_eq!(
        up("http://127.0.0.1:9999/wire/SECRET/claude-code/anthropic").path_prefix(),
        "/wire/SECRET/claude-code/anthropic"
    );
    assert_eq!(
        up("http://127.0.0.1:9999/wire/SECRET/claude-code/anthropic/").path_prefix(),
        "/wire/SECRET/claude-code/anthropic"
    );
}

// ---- build_upstream_client (M3.4) ----

#[test]
fn upstream_client_builds_and_clone_is_cheap() {
    // We do not perform a TLS handshake here; only assert that constructing the
    // native-roots-backed, no-redirect client succeeds. A real http forward is
    // covered by the integration tests in tests/proxy_engine.rs.
    let client = build_upstream_client().expect("client builds");
    // Cloning shares the connection pool; assert it is cheap/valid.
    let _clone = client.clone();
}

// ---- TransformKind::as_body_transform (M3.5) ----

#[test]
fn transform_kind_none_yields_no_transform() {
    let k = TransformKind::None;
    assert!(k.as_body_transform().is_none());
}

#[test]
fn transform_kind_pino_yields_a_boxed_transform() {
    use crate::proxy::pino::{CacheTtl, PinoSettings};
    // M1's PinoTransform stub fails loud until M4; the materializer must still
    // hand back an Arc-shared transform (the engine decides what to do with an Err).
    let settings = PinoSettings {
        auto_cache: false,
        main_ttl: CacheTtl::OneHour,
        sub_ttl: CacheTtl::FiveMin,
        drop_tools: vec![],
        strip_ansi: false,
        model_override: None,
    };
    let k = TransformKind::Pino(settings);
    assert!(
        k.as_body_transform().is_some(),
        "pino kind yields a transform"
    );
}

#[test]
fn transform_kind_pino_transform_succeeds_after_m4() {
    use crate::proxy::pino::{CacheTtl, PinoSettings};
    // R9's M1 stub returned an explicit Err; M4.2 replaced it with the real
    // dispatch skeleton, so the materialized pino transform now returns Ok (and,
    // with every feature off, is a byte-faithful no-op).
    let settings = PinoSettings {
        auto_cache: false,
        main_ttl: CacheTtl::OneHour,
        sub_ttl: CacheTtl::FiveMin,
        drop_tools: vec![],
        strip_ansi: false,
        model_override: None,
    };
    let t = TransformKind::Pino(settings).as_body_transform().unwrap();
    let original = serde_json::json!({"model": "claude-x", "messages": []});
    let mut body = original.clone();
    t.transform(&mut body, &crate::proxy::RequestContext::default())
        .expect("M4 pino transform must succeed");
    assert_eq!(body, original, "all-features-off pino transform is a no-op");
}

#[test]
fn transform_kind_headroom_yields_a_boxed_transform() {
    use crate::proxy::headroom::HeadroomSettings;
    let k = TransformKind::Headroom(HeadroomSettings { compression: false });
    assert!(
        k.as_body_transform().is_some(),
        "headroom kind yields a transform"
    );
}

// ---- RequestContext subagent detection ----

#[test]
fn request_context_detects_subagent_from_nonempty_header() {
    let mut h = http::HeaderMap::new();
    h.insert(
        SUBAGENT_HEADER,
        http::HeaderValue::from_static("agent_general-purpose_01J"),
    );
    assert!(RequestContext::from_headers(&h).is_subagent);
}

#[test]
fn request_context_main_when_header_absent() {
    let h = http::HeaderMap::new();
    assert!(!RequestContext::from_headers(&h).is_subagent);
    assert_eq!(RequestContext::from_headers(&h), RequestContext::default());
}

#[test]
fn request_context_main_when_header_present_but_empty() {
    // Matches Claude Code's `Boolean(header)` truthiness: "" is not a subagent.
    let mut h = http::HeaderMap::new();
    h.insert(SUBAGENT_HEADER, http::HeaderValue::from_static(""));
    assert!(!RequestContext::from_headers(&h).is_subagent);
}

#[test]
fn request_context_main_when_header_value_not_ascii() {
    // Real agent IDs are ASCII; a non-ASCII value (`to_str` fails) is treated as main.
    let mut h = http::HeaderMap::new();
    h.insert(
        SUBAGENT_HEADER,
        http::HeaderValue::from_bytes(b"\xff\xfe").unwrap(),
    );
    assert!(!RequestContext::from_headers(&h).is_subagent);
}

#[test]
fn request_context_uses_first_value_for_duplicate_header() {
    // `HeaderMap::get` reads only the first value: an empty first value wins.
    let mut h = http::HeaderMap::new();
    h.append(SUBAGENT_HEADER, http::HeaderValue::from_static(""));
    h.append(
        SUBAGENT_HEADER,
        http::HeaderValue::from_static("agent_general-purpose_01J"),
    );
    assert!(!RequestContext::from_headers(&h).is_subagent);
}
