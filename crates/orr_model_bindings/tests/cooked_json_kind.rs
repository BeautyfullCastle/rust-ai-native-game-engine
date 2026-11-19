use orr_model_bindings::model_bindings::{cooked_json_kind, ModelKind, COOKED_HEADER_PROBE_BYTES};

#[test]
fn header_members_pick_the_kind_in_any_order() {
    assert_eq!(
        cooked_json_kind(br#"{"format":"orr_static_model","version":1}"#),
        Ok(ModelKind::Static)
    );
    assert_eq!(
        cooked_json_kind(br#"{"version":1,"x":[1,{"y":2}],"format":"orr_animated_model"}"#),
        Ok(ModelKind::Animated)
    );
}

#[test]
fn a_prefix_that_ends_inside_the_body_is_enough() {
    let prefix = br#"{"format":"orr_static_model","version":1,"vertices":[[0.0,1.0"#;
    assert_eq!(cooked_json_kind(prefix), Ok(ModelKind::Static));
}

#[test]
fn unrelated_or_broken_json_is_rejected_with_a_reason() {
    for bytes in [
        &b""[..],
        b"not json",
        b"[1,2]",
        br#"{"hello":"world"}"#,
        br#"{"format":"orr_static_model"}"#,
        br#"{"format":"other","version":1}"#,
        br#"{"format":"orr_static_model","version":2}"#,
        br#"{"format":"orr_static_model_cylinder","version":1}"#,
        br#"{"format":1,"version":1}"#,
    ] {
        assert!(cooked_json_kind(bytes).is_err(), "{bytes:?}");
    }
}

#[test]
fn a_header_after_the_probe_window_is_not_found() {
    let mut text = br#"{"padding":""#.to_vec();
    text.resize(COOKED_HEADER_PROBE_BYTES, b'x');
    assert!(cooked_json_kind(&text).is_err());
}
