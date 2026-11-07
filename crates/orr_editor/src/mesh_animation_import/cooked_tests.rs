use super::*;
#[cfg(target_os = "linux")]
const SOURCE: &[u8] =
    include_bytes!("../../../../assets/room_character_demo/courier.orrmodel.json");
#[cfg(target_os = "linux")]
pub(crate) fn png_bytes() -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, 1, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&[7, 31, 127, 63])
            .unwrap();
    }
    bytes
}
#[cfg(target_os = "linux")]
pub(crate) fn variants() -> Vec<(&'static str, Vec<u8>)> {
    let recipe = |format: &str, extra: serde_json::Value| {
        let mut r = serde_json::json!({"format":format,"version":1,"source":"input.animated.json","material":0});
        r.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        serde_json::to_vec(&r).unwrap()
    };
    let resolve = |p: &str| {
        assert_eq!(p, "input.animated.json");
        Ok(SOURCE.to_vec())
    };
    let color = orr_model::animated_recolor::recolor_model(
        &recipe(
            "orr_animated_material_color",
            serde_json::json!({"linear_rgb":[0.2,0.7,0.4]}),
        ),
        "variants/blue.recolor.json",
        resolve,
    )
    .unwrap();
    let texture = orr_model::animated_retexture::retexture_model(
        &recipe(
            "orr_animated_material_texture",
            serde_json::json!({"texture":"input.png"}),
        ),
        "variants/skin.retexture.json",
        |p| match p {
            "input.animated.json" => Ok(SOURCE.to_vec()),
            "input.png" => Ok(png_bytes()),
            _ => panic!("historical provenance was resolved"),
        },
    )
    .unwrap();
    let sampler = orr_model::animated_sampler::author_sampler(
        &recipe(
            "orr_animated_material_sampler",
            serde_json::json!({"linear_filter":true,"wrap_s":"mirror","wrap_t":"repeat"}),
        ),
        "variants/tile.sampler.json",
        resolve,
    )
    .unwrap();
    let uv = orr_model::animated_uv::transform_model_uv(
        &recipe(
            "orr_animated_material_uv",
            serde_json::json!({"offset":[0.25,-0.5],"scale":[2,0.5],"quarter_turns":1}),
        ),
        "variants/tile.uv.json",
        resolve,
    )
    .unwrap();
    vec![
        ("color", color.to_bytes().unwrap()),
        ("texture", texture.to_bytes().unwrap()),
        ("sampler", sampler.to_bytes().unwrap()),
        ("uv", uv.to_bytes().unwrap()),
    ]
}
#[test]
fn explicit_filename_dispatch_preserves_raw_and_selects_only_recognized_cooked_suffixes() {
    for name in ["model.glb", "MODEL.GLB"] {
        assert_eq!(source_asset(Path::new(name)).unwrap(), ASSET);
    }
    for name in [
        "hero.animated.json",
        "hero.ANIMATED.JSON",
        "courier.orrmodel.json",
        "courier.ORRMODEL.JSON",
    ] {
        assert_eq!(source_asset(Path::new(name)).unwrap(), COOKED_ASSET);
    }
    for name in [
        "hero.json",
        "hero.glb.backup",
        "hero.gltf",
        "hero.orrmodel",
        "hero.model.json",
        "",
    ] {
        assert!(source_asset(Path::new(name)).is_err(), "{name}");
    }
}
#[cfg(target_os = "linux")]
mod filesystem {
    use super::*;
    fn request(root: &Path, bytes: &[u8]) -> Request {
        let source = root.join("hero.animated.json");
        let license = root.join("license.txt");
        fs::write(&source, bytes).unwrap();
        fs::write(&license, b"Explicit redistributable fixture license").unwrap();
        Request {
            source,
            license,
            destination: root.join("package"),
            name: "cooked-mesh-animation".into(),
            version: "1.0.0".into(),
            license_acknowledged: true,
            graph_revision: [0x5a; 32],
        }
    }
    fn no_stage(root: &Path) {
        assert!(!fs::read_dir(root).unwrap().any(|p| p
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".orr-mesh-animation-import-")));
    }
    #[test]
    fn four_real_producer_outputs_publish_verbatim_and_reopen_without_original_inputs() {
        for (name, bytes) in variants() {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().canonicalize().unwrap();
            let req = request(&root, &bytes);
            let source = req.source.clone();
            let license = req.license.clone();
            let report = import_package(req).unwrap();
            assert!(report.publication_warning.is_none(), "{name}");
            assert_eq!(report.asset, COOKED_ASSET);
            assert_eq!(fs::read(report.root.join(COOKED_ASSET)).unwrap(), bytes);
            assert_eq!(fs::read(&source).unwrap(), bytes);
            let encoded = report.snapshot.encode_embedded("mesh.scene.yaml").unwrap();
            fs::remove_file(source).unwrap();
            fs::remove_file(license).unwrap();
            fs::rename(&report.root, root.join("hidden-package")).unwrap();
            let reopened =
                Snapshot::decode_embedded(&encoded, "mesh.scene.yaml", [0x5a; 32]).unwrap();
            assert_eq!(
                reopened.encode_embedded("mesh.scene.yaml").unwrap(),
                encoded
            );
            no_stage(&root);
        }
    }
    #[test]
    fn malformed_renamed_raw_links_aliases_and_existing_outputs_never_publish() {
        let valid = variants().remove(0).1;
        for bad in [
            b"not JSON".to_vec(),
            include_bytes!("../../../../assets/animation_demo/animated.glb").to_vec(),
            vec![b' '; MAX_SOURCE_BYTES as usize + 1],
        ] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().canonicalize().unwrap();
            let req = request(&root, &bad);
            let output = req.destination.clone();
            assert!(import_package(req).is_err());
            assert!(!output.exists());
            no_stage(&root);
        }
        for fault in 0..4 {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().canonicalize().unwrap();
            let mut req = request(&root, &valid);
            let output = req.destination.clone();
            match fault {
                0 => {
                    let alias = root.join("alias.animated.json");
                    std::os::unix::fs::symlink(&req.source, &alias).unwrap();
                    req.source = alias;
                }
                1 => {
                    fs::remove_file(&req.license).unwrap();
                    fs::hard_link(&req.source, &req.license).unwrap();
                }
                2 => {
                    fs::create_dir(&output).unwrap();
                    fs::write(output.join("keep"), b"foreign").unwrap();
                }
                _ => req.license_acknowledged = false,
            };
            assert!(import_package(req).is_err());
            if fault == 2 {
                assert_eq!(fs::read(output.join("keep")).unwrap(), b"foreign");
            } else {
                assert!(!output.exists());
            }
            no_stage(&root);
        }
    }
    #[test]
    fn changed_cooked_input_and_staging_bytes_keep_original_no_replace_contract() {
        for fault in 0..3 {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().canonicalize().unwrap();
            let bytes = variants().remove(0).1;
            let req = request(&root, &bytes);
            let source = req.source.clone();
            let output = req.destination.clone();
            assert!(import_with_hook(req, |phase, stage, _| {
                if phase == Phase::BeforePublish {
                    match fault {
                        0 => fs::write(&source, b"changed").unwrap(),
                        1 => fs::write(stage.join(COOKED_ASSET), b"changed").unwrap(),
                        _ => {
                            fs::remove_file(stage.join(COOKED_ASSET)).unwrap();
                            fs::write(stage.join(COOKED_ASSET), &bytes).unwrap();
                        }
                    }
                }
                Ok(())
            })
            .is_err());
            assert!(!output.exists());
            if fault == 0 {
                no_stage(&root);
            } else {
                let retained = fs::read_dir(&root)
                    .unwrap()
                    .map(|e| e.unwrap().path())
                    .find(|p| {
                        p.file_name()
                            .unwrap()
                            .to_string_lossy()
                            .starts_with(".orr-mesh-animation-import-")
                    })
                    .expect("foreign changed staging file must not be deleted");
                assert_eq!(
                    fs::read(retained.join(COOKED_ASSET)).unwrap(),
                    if fault == 1 {
                        b"changed".to_vec()
                    } else {
                        bytes.clone()
                    }
                );
            }
        }
    }
}
