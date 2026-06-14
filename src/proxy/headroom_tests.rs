use super::*;
use crate::proxy::BodyTransform;
use serde_json::json;

#[test]
fn headroom_settings_default_round_trips_yaml() {
    let s = HeadroomSettings { compression: false };
    let yaml = serde_yaml::to_string(&s).unwrap();
    let back: HeadroomSettings = serde_yaml::from_str(&yaml).unwrap();
    assert_eq!(s, back);
}

#[test]
fn headroom_settings_rejects_unknown_fields() {
    let yaml = "compression: true\nbogus: 1\n";
    let err = serde_yaml::from_str::<HeadroomSettings>(yaml).unwrap_err();
    assert!(
        err.to_string().contains("bogus") || err.to_string().contains("unknown field"),
        "deny_unknown_fields should reject `bogus`, got: {err}"
    );
}

#[test]
fn headroom_transform_fails_loud_until_m5() {
    let t = HeadroomTransform {
        settings: HeadroomSettings { compression: true },
    };
    let mut body = serde_json::json!({"messages": []});
    let err = crate::proxy::BodyTransform::transform(&t, &mut body).unwrap_err();
    assert_eq!(
        err.to_string(),
        "headroom compression enabled but transform not implemented"
    );
}

/// A representative Anthropic request body with a large, highly compressible
/// JSON-array tool_result. With compression DISABLED, the transform must not
/// touch a single byte — the serialized Value must be identical before/after.
fn compressible_body() -> serde_json::Value {
    let array: Vec<serde_json::Value> = (0..200)
        .map(|i| json!({ "id": i, "status": "ok", "value": format!("repeat-pattern-{}", i % 3) }))
        .collect();
    let payload = serde_json::to_string(&array).unwrap();
    json!({
        "model": "claude-sonnet-4-6",
        "max_tokens": 64,
        "system": "you are a helpful assistant",
        "messages": [{
            "role": "user",
            "content": [{
                "type": "tool_result",
                "tool_use_id": "toolu_pm_test",
                "content": payload,
            }],
        }],
    })
}

#[test]
fn disabled_compression_is_byte_equal_noop() {
    let t = HeadroomTransform {
        settings: HeadroomSettings { compression: false },
    };
    let mut body = compressible_body();
    let before = serde_json::to_vec(&body).unwrap();
    t.transform(&mut body)
        .expect("disabled transform must be Ok");
    let after = serde_json::to_vec(&body).unwrap();
    assert_eq!(
        before, after,
        "disabled compression must be a byte-equal no-op"
    );
}
