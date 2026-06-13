use super::*;

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
    let kinds = [
        TransformKind::None,
        TransformKind::Pino,
        TransformKind::Headroom,
    ];
    assert_eq!(kinds.len(), 3);
}

// ---- BodyTransform default apply_headers is a no-op (R6) ----

struct IdentityTransform;
impl BodyTransform for IdentityTransform {
    fn transform(&self, _body: &mut serde_json::Value) -> anyhow::Result<()> {
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

#[test]
fn engine_config_holds_run_id_and_transform_kind() {
    let cfg = EngineConfig {
        name: ProxyName::Pino,
        listen: "127.0.0.1:0".parse().unwrap(),
        upstream: Upstream::parse("https://api.anthropic.com").unwrap(),
        run_id: "01ARZ".to_string(),
        log_file: None,
        transform: TransformKind::Pino,
    };
    assert_eq!(cfg.name, ProxyName::Pino);
    assert_eq!(cfg.run_id, "01ARZ");
    assert_eq!(cfg.transform, TransformKind::Pino);
    assert_eq!(cfg.upstream.host_header(), "api.anthropic.com");
}
