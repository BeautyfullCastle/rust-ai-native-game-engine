use super::*;
#[allow(dead_code)]
#[path = "../../../../orr_model/examples/static_model_cylinder/authoring.rs"]
mod cylinder;
#[allow(dead_code)]
#[path = "../../../../orr_model/examples/static_model_pivot/authoring.rs"]
mod pivot;
#[allow(dead_code)]
#[path = "../../../../orr_model/examples/static_model_recolor/authoring.rs"]
mod recolor;
#[allow(dead_code)]
#[path = "../../../../orr_model/examples/static_model_sampler/authoring.rs"]
mod sampler;
fn base() -> Vec<u8> {
    cylinder::author_cylinder(br#"{"format":"orr_static_model_cylinder","version":1,"radius":0.5,"height":2,"segments":8}"#,"base.cylinder.json").unwrap().to_bytes().unwrap()
}
fn png() -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut e = png::Encoder::new(&mut out, 1, 1);
        e.set_color(png::ColorType::Rgba);
        e.set_depth(png::BitDepth::Eight);
        e.write_header()
            .unwrap()
            .write_image_data(&[17, 39, 81, 255])
            .unwrap();
    }
    out
}
pub(crate) fn variants() -> Vec<(&'static str, Vec<u8>)> {
    let source = base();
    let resolve = |path: &str| -> Result<Vec<u8>, String> {
        match path {
            "inputs/base.model.json" => Ok(source.clone()),
            "textures/skin.png" => Ok(png()),
            _ => panic!("historicalsource resolved"),
        }
    };
    let color=recolor::recolor_model(br#"{"format":"orr_static_material_color","version":1,"source":"inputs/base.model.json","material":0,"linear_rgb":[0.2,0.7,0.4]}"#,"variants/color.recolor.json",resolve).unwrap();
    let texture=orr_model::static_retexture::retexture_model(br#"{"format":"orr_static_material_texture","version":1,"source":"inputs/base.model.json","material":0,"texture":"textures/skin.png"}"#,"variants/skin.retexture.json",resolve).unwrap();
    let uv=orr_model::static_uv::transform_model_uv(br#"{"format":"orr_static_material_uv","version":1,"source":"inputs/base.model.json","material":0,"offset":[0.25,-0.5],"scale":[2,0.5],"quarter_turns":1}"#,"variants/uv.uv.json",resolve).unwrap();
    let sampler=sampler::author_sampler(br#"{"format":"orr_static_material_sampler","version":1,"source":"inputs/base.model.json","material":0,"linear_filter":true,"wrap_s":"repeat","wrap_t":"mirror"}"#,"variants/tile.sampler.json",resolve).unwrap();
    let pivot=pivot::rebase_model_pivot(br#"{"format":"orr_static_model_pivot","version":1,"source":"inputs/base.model.json","pivot":[1,0,0]}"#,"variants/pivot.pivot.json",resolve).unwrap();
    let recipe = serde_json::json!({"format":"orr_static_model_assemble","version":1,"parts":[{"key":"a","source":"inputs/base.model.json","placement":orr_model::IDENTITY}]});
    let assembly = orr_model::static_assemble::assemble_model(
        &serde_json::to_vec(&recipe).unwrap(),
        "variants/group.assemble.json",
        resolve,
    )
    .unwrap();
    vec![
        ("cylinder", source),
        ("assembly", assembly.to_bytes().unwrap()),
        ("color", color.to_bytes().unwrap()),
        ("texture", texture.to_bytes().unwrap()),
        ("uv", uv.to_bytes().unwrap()),
        ("sampler", sampler.to_bytes().unwrap()),
        ("pivot", pivot.to_bytes().unwrap()),
    ]
}
#[test]
fn all_real_static_producers_preserve_exact_cooked_payload() {
    for (name, bytes) in variants() {
        for suffix in ["model.model.json", "model.ORRMODEL.JSON", "model.orrmodel"] {
            let loaded = decode(suffix, &bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(loaded.to_bytes().unwrap(), bytes);
        }
    }
}
#[test]
fn assembly_with_32_distinct_direct_inputs_is_not_limited_to_three_records() {
    let source = base();
    let parts:Vec<_>=(0..32).map(|i|serde_json::json!({"key":format!("p{i}"),"source":format!("inputs/part{i}.model.json"),"placement":orr_model::IDENTITY})).collect();
    let recipe = serde_json::to_vec(
        &serde_json::json!({"format":"orr_static_model_assemble","version":1,"parts":parts}),
    )
    .unwrap();
    let model = orr_model::static_assemble::assemble_model(&recipe, "group.assemble.json", |_| {
        Ok(source.clone())
    })
    .unwrap();
    assert_eq!(model.source().dependencies.len(), 33);
    let bytes = model.to_bytes().unwrap();
    assert_eq!(
        decode("group.model.json", &bytes)
            .unwrap()
            .to_bytes()
            .unwrap(),
        bytes
    );
}
#[test]
fn unsafe_duplicate_or_bad_hash_metadata_rejects_without_history_io() {
    let original = StaticModel::from_bytes(&base()).unwrap();
    for uri in [
        "https://x/a.model.json",
        "/a.model.json",
        "../a.model.json",
        "a/../b.model.json",
        "C:/a.model.json",
        "CON.model.json",
        "a\\b.model.json",
    ] {
        let mut s = original.source().clone();
        s.dependencies.push(orr_model::Dependency {
            uri: uri.into(),
            sha256: "a".repeat(64),
        });
        assert!(
            decode("model.model.json", &serde_json::to_vec(&s).unwrap()).is_err(),
            "{uri}"
        );
    }
    let mut s = original.source().clone();
    s.dependencies.push(s.dependencies[0].clone());
    assert!(decode("model.model.json", &serde_json::to_vec(&s).unwrap()).is_err());
    s = original.source().clone();
    s.dependencies[0].sha256 = "A".repeat(64);
    assert!(decode("model.model.json", &serde_json::to_vec(&s).unwrap()).is_err());
}
#[test]
fn raw_and_cooked_formats_are_explicit_and_external_resources_rejected() {
    let raw = include_bytes!("../../../../../assets/imported_scene_demo/foreground.glb");
    assert!(decode("model.glb", raw).is_ok());
    assert!(decode("model.model.json", raw).is_err());
    assert!(decode("model.glb", &base()).is_err());
    assert!(decode("model.json", &base()).is_err());
    assert!(decode(
        "animated.model.json",
        include_bytes!("../../../../../assets/room_character_demo/courier.orrmodel.json")
    )
    .is_err());
    let len = u32::from_le_bytes(raw[12..16].try_into().unwrap()) as usize;
    let mut json: serde_json::Value = serde_json::from_slice(&raw[20..20 + len]).unwrap();
    json["buffers"][0]["uri"] = serde_json::json!("missing.bin");
    let mut data = serde_json::to_vec(&json).unwrap();
    while data.len() % 4 != 0 {
        data.push(b' ');
    }
    let rest = &raw[20 + len..];
    let total = 20 + data.len() + rest.len();
    let mut changed = raw[..12].to_vec();
    changed[8..12].copy_from_slice(&(total as u32).to_le_bytes());
    changed.extend_from_slice(&(data.len() as u32).to_le_bytes());
    changed.extend_from_slice(&raw[16..20]);
    changed.extend(data);
    changed.extend(rest);
    assert!(decode("model.glb", &changed).is_err());
}
