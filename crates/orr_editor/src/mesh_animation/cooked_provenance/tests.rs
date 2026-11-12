use super::*;
const SOURCE: &[u8] =
    include_bytes!("../../../../../assets/room_character_demo/courier.orrmodel.json");
fn png() -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut e = png::Encoder::new(&mut out, 1, 1);
        e.set_color(png::ColorType::Rgba);
        e.set_depth(png::BitDepth::Eight);
        e.write_header()
            .unwrap()
            .write_image_data(&[11, 33, 77, 255])
            .unwrap();
    }
    out
}
#[test]
fn all_four_real_producers_keep_cooked_payload_material_rig_and_clips() {
    let recipes = [
        serde_json::json!({"format":"orr_animated_material_color","version":1,"source":"inputs/hero.animated.json","material":0,"linear_rgb":[0.2,0.7,0.4]}),
        serde_json::json!({"format":"orr_animated_material_texture","version":1,"source":"inputs/hero.animated.json","texture":"textures/skin.png","material":0}),
        serde_json::json!({"format":"orr_animated_material_sampler","version":1,"source":"inputs/hero.animated.json","material":0,"linear_filter":true,"wrap_s":"repeat","wrap_t":"mirror"}),
        serde_json::json!({"format":"orr_animated_material_uv","version":1,"source":"inputs/hero.animated.json","material":0,"offset":[0.25,-0.5],"scale":[2.0,0.5],"quarter_turns":1}),
    ];
    for (i, recipe) in recipes.into_iter().enumerate() {
        let recipe = serde_json::to_vec(&recipe).unwrap();
        let resolver = |path: &str| -> Result<Vec<u8>, String> {
            match path {
                "inputs/hero.animated.json" => Ok(SOURCE.to_vec()),
                "textures/skin.png" => Ok(png()),
                _ => panic!("historical metadata resolved"),
            }
        };
        let model = match i {
            0 => orr_model::animated_recolor::recolor_model(
                &recipe,
                "variants/hero.recolor.json",
                resolver,
            ),
            1 => orr_model::animated_retexture::retexture_model(
                &recipe,
                "variants/hero.retexture.json",
                resolver,
            ),
            2 => orr_model::animated_sampler::author_sampler(
                &recipe,
                "variants/hero.sampler.json",
                resolver,
            ),
            _ => orr_model::animated_uv::transform_model_uv(
                &recipe,
                "variants/hero.uv.json",
                resolver,
            ),
        }
        .unwrap();
        let bytes = model.to_bytes().unwrap();
        let loaded = decode("model.ANIMATED.JSON", &bytes).unwrap();
        assert_eq!(loaded.to_bytes().unwrap(), bytes);
        assert_eq!(loaded.source().clips.len(), 3);
        for clip in 0..3 {
            let pose = loaded.sample_clip(clip, 0.5).unwrap();
            loaded.bounds(&pose).unwrap();
        }
    }
}
#[test]
fn original_cooked_and_raw_glb_remain_accepted_without_external_lookup() {
    assert_eq!(
        decode("courier.orrmodel.json", SOURCE)
            .unwrap()
            .source()
            .clips
            .len(),
        3
    );
    let raw = include_bytes!("../../../../../assets/animation_demo/animated.glb");
    assert!(decode("animated.glb", raw).is_ok());
    assert!(decode("bad.animated.json", raw).is_err());
    assert!(decode("bad.glb", SOURCE).is_err());
}
#[test]
fn unsafe_or_nonproducer_metadata_is_rejected() {
    for uri in [
        "https://example/a.animated.json",
        "/a.animated.json",
        "../a.animated.json",
        "a/../b.animated.json",
        "C:/a.animated.json",
        "a\\b.animated.json",
        "CON.animated.json",
        "wrong.glb",
    ] {
        let mut source = AnimatedModel::from_bytes(SOURCE).unwrap().source().clone();
        source.dependencies.push(orr_model::Dependency {
            uri: uri.into(),
            sha256: "a".repeat(64),
        });
        let bytes = serde_json::to_vec(&source).unwrap();
        assert!(decode("model.animated.json", &bytes).is_err(), "{uri}");
    }
    let mut source = AnimatedModel::from_bytes(SOURCE).unwrap().source().clone();
    source.dependencies[0].sha256 = "A".repeat(64);
    assert!(decode("model.animated.json", &serde_json::to_vec(&source).unwrap()).is_err());
    source.dependencies[0].sha256 = "a".repeat(64);
    source.dependencies.push(source.dependencies[0].clone());
    assert!(decode("model.animated.json", &serde_json::to_vec(&source).unwrap()).is_err());
}
#[test]
fn malformed_or_external_image_payload_never_passes_cooked_admission() {
    let mut value: serde_json::Value = serde_json::from_slice(SOURCE).unwrap();
    value["images"][0] = serde_json::json!({"uri":"missing.png"});
    assert!(decode("model.animated.json", &serde_json::to_vec(&value).unwrap()).is_err());
    assert!(decode("model.animated.json", b"{}").is_err());
    assert!(decode("model.animated.json", b"[]").is_err());
    // Retain the actual BIN chunk while changing the declared buffer to an external URI.
    let raw = include_bytes!("../../../../../assets/animation_demo/animated.glb");
    let json_len = u32::from_le_bytes(raw[12..16].try_into().unwrap()) as usize;
    let mut json: serde_json::Value = serde_json::from_slice(&raw[20..20 + json_len]).unwrap();
    json["buffers"][0]["uri"] = serde_json::json!("missing.bin");
    let mut payload = serde_json::to_vec(&json).unwrap();
    while payload.len() % 4 != 0 {
        payload.push(b' ');
    }
    let rest = &raw[20 + json_len..];
    let length = 12 + 8 + payload.len() + rest.len();
    let mut changed = raw[..12].to_vec();
    changed[8..12].copy_from_slice(&(length as u32).to_le_bytes());
    changed.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    changed.extend_from_slice(&raw[16..20]);
    changed.extend_from_slice(&payload);
    changed.extend_from_slice(rest);
    assert!(decode("animated.glb", &changed).is_err());
}
