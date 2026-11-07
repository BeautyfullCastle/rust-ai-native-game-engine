use super::*;
fn document() -> Document {
    Document {
        environment: Some(Environment::default()),
        ..Document::enabled_default()
    }
}
#[test]
fn strict_codec_roundtrip_and_actual_consumer() {
    let source = document();
    let preset = Preset::from_document(&source).unwrap();
    let bytes = preset.encode().unwrap();
    let decoded = Preset::decode(&bytes).unwrap();
    assert_eq!(preset, decoded);
    assert_eq!(bytes, decoded.encode().unwrap());
    let merged = decoded.merge_into(&Document::default()).unwrap();
    let mut lighting = orr_render::Lighting::default();
    merged.apply_environment(&mut lighting).unwrap();
    assert_eq!(lighting.direction, source.environment.unwrap().direction);
    assert_eq!(
        Document::parse(&merged.to_bytes().unwrap()).unwrap(),
        merged
    );
}
#[test]
fn merging_preserves_scene_bound_light_policy_and_inputs() {
    let mut target = document();
    target.follow_player = Some([1.0, -0.0, 2.0]);
    target.activation = Some(orr_sample::room_lighting::Activation::KeyCollected);
    let original = target.to_bytes().unwrap();
    let mut other = document();
    other.environment.as_mut().unwrap().ambient = 7.0;
    let merged = Preset::from_document(&other)
        .unwrap()
        .merge_into(&target)
        .unwrap();
    assert_eq!(merged.point_light, target.point_light);
    assert_eq!(merged.follow_player, target.follow_player);
    assert_eq!(merged.activation, target.activation);
    assert_eq!(merged.environment, other.environment);
    assert_eq!(target.to_bytes().unwrap(), original);
}
#[test]
fn strict_wire_rejects_unknown_duplicate_null_arrays_trailing_and_bounds() {
    let bytes = Preset::from_document(&document())
        .unwrap()
        .encode()
        .unwrap();
    let base = String::from_utf8(bytes).unwrap();
    for invalid in [
        "[]".to_owned(),
        "null".to_owned(),
        base.replace("\"schema\": 1", "\"schema\": 2"),
        base.replace("\"schema\": 1", "\"schema\": 1, \"schema\": 1"),
        base.replace("\"schema\": 1", "\"schema\": 1, \"extra\": 0"),
        base.replace(IDENTITY, "OTHER"),
        format!("{base} null"),
        "{\"schema\":1,\"identity\":\"ORR_ROOM_ENVIRONMENT_PRESET_V1\",\"environment\":null}"
            .to_owned(),
    ] {
        assert!(Preset::decode(invalid.as_bytes()).is_err(), "{invalid}");
    }
    for invalid in [
        serde_json::json!({"direction":[0,0,0],"color":[1,1,1],"intensity":1,"sky":[1,1,1],"ground":[0,0,0],"ambient":1}),
        serde_json::json!({"direction":[0,1,0],"color":[2,1,1],"intensity":1,"sky":[1,1,1],"ground":[0,0,0],"ambient":1}),
    ] {
        let wire = serde_json::json!({"schema":1,"identity":IDENTITY,"environment":invalid});
        assert!(Preset::decode(&serde_json::to_vec(&wire).unwrap()).is_err());
    }
    assert!(Preset::decode(&vec![b' '; MAX_BYTES + 1]).is_err());
}
#[test]
fn disabled_capture_and_invalid_public_values_fail_without_mutation() {
    assert!(Preset::from_document(&Document::default()).is_err());
    let mut source = document();
    source.environment.as_mut().unwrap().intensity = f32::NAN;
    assert!(Preset::from_document(&source).is_err());
    let bad = Preset {
        environment: source.environment.unwrap(),
    };
    assert!(bad.encode().is_err());
    let target = document();
    let before = target.to_bytes().unwrap();
    assert!(bad.merge_into(&target).is_err());
    assert_eq!(target.to_bytes().unwrap(), before);
}
#[cfg(target_os = "linux")]
#[test]
fn store_roundtrip_never_replaces_foreign_output_or_accepts_links() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("look.room-environment-preset.json");
    let bytes = Preset::from_document(&document())
        .unwrap()
        .encode()
        .unwrap();
    save_new(&path, &bytes).unwrap();
    assert_eq!(load(&path).unwrap().encode().unwrap(), bytes);
    assert!(save_new(&path, &bytes).is_err());
    assert_eq!(fs::read(&path).unwrap(), bytes);
    let link = dir.path().join("link.room-environment-preset.json");
    std::os::unix::fs::symlink(&path, &link).unwrap();
    assert!(load(&link).is_err());
    assert!(save_new(&link, &bytes).is_err());
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
}
#[cfg(target_os = "linux")]
#[test]
fn observed_source_replacement_and_publication_collision_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("look.room-environment-preset.json");
    let bytes = Preset::from_document(&document())
        .unwrap()
        .encode()
        .unwrap();
    save_new(&path, &bytes).unwrap();
    let checked = checked_path(&path).unwrap();
    assert!(read_file(
        &checked,
        || {
            fs::rename(&path, dir.path().join("old")).unwrap();
            fs::write(&path, &bytes).unwrap();
        },
        || {}
    )
    .is_err());
    let output = dir.path().join("new.room-environment-preset.json");
    assert!(save_new_observed(&output, &bytes, || {
        fs::write(&output, b"foreign").unwrap();
    })
    .is_err());
    assert_eq!(fs::read(output).unwrap(), b"foreign");
}
