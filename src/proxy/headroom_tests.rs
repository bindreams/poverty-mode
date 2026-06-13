use super::*;

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
    assert_eq!(err.to_string(), "headroom transform not implemented");
}
