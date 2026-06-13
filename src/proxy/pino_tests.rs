use super::*;

#[test]
fn pino_settings_default_round_trips_yaml() {
    let s = PinoSettings {
        auto_cache: true,
        tail_ttl: TailTtl::FiveMin,
        drop_tools: vec![],
        strip_ansi: true,
        model_override: None,
    };
    let yaml = serde_yaml::to_string(&s).unwrap();
    let back: PinoSettings = serde_yaml::from_str(&yaml).unwrap();
    assert_eq!(s, back);
}

#[test]
fn tail_ttl_serializes_as_short_strings() {
    assert_eq!(serde_yaml::to_string(&TailTtl::FiveMin).unwrap().trim(), "5m");
    assert_eq!(serde_yaml::to_string(&TailTtl::OneHour).unwrap().trim(), "1h");
    let five: TailTtl = serde_yaml::from_str("\"5m\"").unwrap();
    let hour: TailTtl = serde_yaml::from_str("\"1h\"").unwrap();
    assert_eq!(five, TailTtl::FiveMin);
    assert_eq!(hour, TailTtl::OneHour);
}

#[test]
fn tail_ttl_invalid_value_falls_back_to_five_min() {
    // R22/R23k: the custom lenient Deserialize maps any invalid string to
    // FiveMin (Node parseTailTtl parity) instead of erroring. M2 also asserts
    // this from the config layer; M4 relies on it.
    let parsed: TailTtl = serde_yaml::from_str("\"7m\"").unwrap();
    assert_eq!(parsed, TailTtl::FiveMin);
    let parsed: TailTtl = serde_yaml::from_str("\"\"").unwrap();
    assert_eq!(parsed, TailTtl::FiveMin);
    let parsed: TailTtl = serde_yaml::from_str("\"banana\"").unwrap();
    assert_eq!(parsed, TailTtl::FiveMin);
}

#[test]
fn pino_settings_rejects_unknown_fields() {
    let yaml = "auto_cache: true\ntail_ttl: 5m\ndrop_tools: []\nstrip_ansi: true\nmodel_override: null\nbogus: 1\n";
    let err = serde_yaml::from_str::<PinoSettings>(yaml).unwrap_err();
    assert!(
        err.to_string().contains("bogus") || err.to_string().contains("unknown field"),
        "deny_unknown_fields should reject `bogus`, got: {err}"
    );
}

#[test]
fn pino_transform_fails_loud_until_m4() {
    let t = PinoTransform {
        settings: PinoSettings {
            auto_cache: true,
            tail_ttl: TailTtl::FiveMin,
            drop_tools: vec![],
            strip_ansi: true,
            model_override: None,
        },
    };
    let mut body = serde_json::json!({"model": "claude-x", "messages": []});
    let err = crate::proxy::BodyTransform::transform(&t, &mut body).unwrap_err();
    assert_eq!(err.to_string(), "pino transform not implemented");
}

#[test]
fn pino_transform_apply_headers_is_noop_until_m4() {
    let t = PinoTransform {
        settings: PinoSettings {
            auto_cache: true,
            tail_ttl: TailTtl::FiveMin,
            drop_tools: vec![],
            strip_ansi: true,
            model_override: None,
        },
    };
    let mut headers = http::HeaderMap::new();
    crate::proxy::BodyTransform::apply_headers(&t, &mut headers);
    assert!(headers.is_empty());
}
