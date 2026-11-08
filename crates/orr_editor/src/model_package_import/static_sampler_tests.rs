use super::*;
use orr_model::{Dependency, Image, StaticModel, Wrap};

fn variant() -> StaticSamplerVariant {
    StaticSamplerVariant {
        material: 1,
        linear_filter: false,
        wrap_s: Wrap::Clamp,
        wrap_t: Wrap::Mirror,
        asset_id: "variants/prop.sampler.json".into(),
    }
}

fn unopened() -> Request {
    Request {
        source: "missing.model.json".into(),
        license: "missing-license".into(),
        destination: "missing-parent/output".into(),
        name: "sampler-prop".into(),
        version: "1.0.0".into(),
        kind: ModelKind::Static,
        source_format: SourceFormat::CookedStatic,
        license_acknowledged: true,
    }
}

fn fixture() -> StaticModel {
    let model = orr_model::import::import_with_resolver(
        "props/original.glb",
        include_bytes!("../../../orr_model/tests/fixtures/model.glb"),
        |_| panic!("embedded fixture"),
    )
    .unwrap();
    let mut source = model.source().clone();
    assert_eq!(source.materials.len(), 2);
    assert!(source.primitives.iter().any(|p| p.material == 1));
    source.materials[0].base_color = [-0.0, 0.25, 0.75, 1.0];
    source.materials[1].base_color = [0.125, 0.5, 0.75, 0.25];
    source.materials[1].image = source.materials[0].image;
    source.materials[0].linear_filter = true;
    source.materials[0].wrap_s = Wrap::Mirror;
    source.materials[0].wrap_t = Wrap::Clamp;
    source.materials.push(source.materials[0].clone());
    source.images.push(Image {
        width: 1,
        height: 1,
        rgba8: vec![19, 37, 71, 103],
    });
    source.primitives[0].transform[3][2] = -0.0;
    for p in &mut source.primitives {
        p.vertices[0].position[2] = -0.0;
        p.vertices[0].normal[0] = -0.0;
        p.vertices[0].uv = [-0.0, f32::from_bits(1)];
    }
    source.dependencies.push(Dependency {
        uri: "history/missing.png".into(),
        sha256: digest(b"not resolved"),
    });
    StaticModel::new(source).unwrap()
}

fn request(root: &Path, source: &[u8]) -> Request {
    let source_path = root.join("source.model.json");
    let license = root.join("LICENSE.txt");
    fs::write(&source_path, source).unwrap();
    fs::write(&license, b"Redistribution license\n").unwrap();
    Request {
        source: source_path,
        license,
        destination: root.join("new-package"),
        ..unopened()
    }
}

fn name(wrap: Wrap) -> &'static str {
    match wrap {
        Wrap::Clamp => "clamp",
        Wrap::Repeat => "repeat",
        Wrap::Mirror => "mirror",
    }
}

#[test]
fn static_sampler_all_eighteen_settings_preserve_exact_other_fields_and_provenance() {
    let original = fixture();
    let source = original.to_bytes().unwrap();
    for filter in [false, true] {
        for s in [Wrap::Clamp, Wrap::Repeat, Wrap::Mirror] {
            for t in [Wrap::Clamp, Wrap::Repeat, Wrap::Mirror] {
                let mut v = variant();
                v.linear_filter = filter;
                v.wrap_s = s;
                v.wrap_t = t;
                let bytes = static_sampler_variant_bytes(&source, &v).unwrap();
                assert_eq!(bytes, static_sampler_variant_bytes(&source, &v).unwrap());
                let out = StaticModel::from_bytes(&bytes).unwrap();
                let mut expected = original.source().clone();
                expected.asset_id = v.asset_id.clone();
                for p in &mut expected.primitives {
                    p.id = format!(
                        "{}{}",
                        v.asset_id,
                        p.id.strip_prefix(&original.source().asset_id).unwrap()
                    );
                }
                let selected = &mut expected.materials[1];
                selected.linear_filter = filter;
                selected.wrap_s = s;
                selected.wrap_t = t;
                // Hand-built canonical recipe oracle independently fixes field
                // spelling, types, order, selected slot and lowercase wrap names.
                let recipe = format!(
                    r#"{{"format":"orr_static_material_sampler","version":1,"source":"input.model.json","material":1,"linear_filter":{filter},"wrap_s":"{}","wrap_t":"{}"}}"#,
                    name(s),
                    name(t)
                );
                expected.dependencies = vec![
                    Dependency {
                        uri: "$source".into(),
                        sha256: digest(recipe.as_bytes()),
                    },
                    Dependency {
                        uri: "input.model.json".into(),
                        sha256: digest(&source),
                    },
                ];
                // Whole payload bytes also preserve signed zeros/subnormals,
                // unused slots, shared image bytes, transforms and topology.
                assert_eq!(serde_json::to_vec(&expected).unwrap(), bytes);
                assert_eq!(
                    out.source().materials[0].image,
                    out.source().materials[1].image
                );
                assert_eq!(out.source().materials[0], original.source().materials[0]);
                assert_eq!(out.source().materials[2], original.source().materials[2]);
            }
        }
    }
    assert_eq!(original.to_bytes().unwrap(), source);
}

#[test]
fn static_sampler_identity_settings_keep_source_float_bits_and_inventory() {
    let original = fixture();
    let mut v = variant();
    let m = &original.source().materials[1];
    v.linear_filter = m.linear_filter;
    v.wrap_s = m.wrap_s;
    v.wrap_t = m.wrap_t;
    let out = StaticModel::from_bytes(
        &static_sampler_variant_bytes(&original.to_bytes().unwrap(), &v).unwrap(),
    )
    .unwrap();
    for (old, new) in original
        .source()
        .primitives
        .iter()
        .zip(&out.source().primitives)
    {
        assert_eq!(
            old.transform.map(|c| c.map(f32::to_bits)),
            new.transform.map(|c| c.map(f32::to_bits))
        );
        for (old, new) in old.vertices.iter().zip(&new.vertices) {
            assert_eq!(
                old.position.map(f32::to_bits),
                new.position.map(f32::to_bits)
            );
            assert_eq!(old.normal.map(f32::to_bits), new.normal.map(f32::to_bits));
            assert_eq!(old.uv.map(f32::to_bits), new.uv.map(f32::to_bits));
        }
    }
    assert_eq!(out.source().materials, original.source().materials);
    assert_eq!(out.source().images, original.source().images);
}

#[test]
fn static_sampler_invalid_identity_index_mode_license_and_hash_reject_before_paths() {
    for id in [
        "",
        "../bad.sampler.json",
        "CON.sampler.json",
        "bad.uv.json",
        "x//bad.sampler.json",
    ] {
        let mut v = variant();
        v.asset_id = id.into();
        assert!(import_static_sampler_variant(unopened(), v)
            .err()
            .unwrap()
            .contains("variant identity"));
    }
    let mut v = variant();
    v.material = u32::MAX;
    assert!(import_static_sampler_variant(unopened(), v)
        .err()
        .unwrap()
        .contains("material limit"));
    let mut r = unopened();
    r.license_acknowledged = false;
    assert!(import_static_sampler_variant(r, variant())
        .err()
        .unwrap()
        .contains("acknowledgement"));
    let mut r = unopened();
    r.kind = ModelKind::Animated;
    assert!(import_static_sampler_variant(r, variant())
        .err()
        .unwrap()
        .contains("Static model mode"));
    for format in [SourceFormat::RawGlb, SourceFormat::CookedAnimated] {
        let mut r = unopened();
        r.source_format = format;
        assert!(import_static_sampler_variant(r, variant())
            .err()
            .unwrap()
            .contains("CookedStatic"));
    }
    for hash in ["".into(), "bad".into(), "z".repeat(64), "a".repeat(65)] {
        assert!(
            import_static_sampler_variant_checked(unopened(), variant(), Some(hash))
                .err()
                .unwrap()
                .contains("64 hexadecimal")
        );
    }
}

#[test]
fn static_sampler_unused_or_absent_material_rejects_without_publication() {
    for index in [2, 3] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let req = request(&root, &fixture().to_bytes().unwrap());
        let destination = req.destination.clone();
        let mut v = variant();
        v.material = index;
        let error = import_static_sampler_variant(req, v).err().unwrap();
        assert!(error.contains("must exist and be used"), "{error}");
        assert!(!destination.exists());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 2);
    }
}

#[test]
fn static_sampler_publication_preserves_source_license_and_publishes_cooked_payload() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let bytes = fixture().to_bytes().unwrap();
    let req = request(&root, &bytes);
    let source = req.source.clone();
    let license = req.license.clone();
    let report =
        import_static_sampler_variant_checked(req, variant(), Some(digest(&bytes).to_uppercase()))
            .unwrap();
    assert_eq!(report.asset, "model.orrmodel");
    assert_eq!(report.kind, ModelKind::Static);
    let output = fs::read(report.root.join(&report.asset)).unwrap();
    assert_eq!(
        output,
        static_sampler_variant_bytes(&bytes, &variant()).unwrap()
    );
    assert_eq!(report.source_sha256, digest(&output));
    assert_eq!(fs::read(source).unwrap(), bytes);
    assert_eq!(
        fs::read(report.root.join("LICENSE.txt")).unwrap(),
        fs::read(&license).unwrap()
    );
    assert_eq!(report.license_sha256, digest(&fs::read(license).unwrap()));
    let package =
        orr_package::inspect(&report.root, &orr_sample::room_project::compiled_runtime()).unwrap();
    assert_eq!(
        package.manifest.files,
        BTreeSet::from(["LICENSE.txt".into(), "model.orrmodel".into()])
    );
    assert_eq!(
        package.manifest.capabilities,
        BTreeSet::from(["models".into()])
    );
}

#[test]
fn static_sampler_stale_source_hash_precedes_license_payload_and_producer() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let bytes = fixture().to_bytes().unwrap();
    let req = request(&root, &bytes);
    let destination = req.destination.clone();
    fs::write(&req.source, b"changed cooked bytes").unwrap();
    fs::write(&req.license, [0xff]).unwrap();
    let error = import_static_sampler_variant_checked(req, variant(), Some(digest(&bytes)))
        .err()
        .unwrap();
    assert!(error.contains("inspected static source changed"), "{error}");
    assert!(!destination.exists());
    assert_eq!(fs::read_dir(&root).unwrap().count(), 2);
}

#[test]
fn static_sampler_invalid_source_license_or_existing_destination_never_publishes_or_overwrites() {
    for case in 0..3 {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let req = request(&root, &fixture().to_bytes().unwrap());
        let destination = req.destination.clone();
        match case {
            0 => fs::write(&req.source, b"not a cooked model").unwrap(),
            1 => fs::write(&req.license, [0xff]).unwrap(),
            _ => {
                fs::create_dir(&destination).unwrap();
                fs::write(destination.join("keep"), b"untouched").unwrap();
            }
        }
        assert!(import_static_sampler_variant(req, variant()).is_err());
        if case == 2 {
            assert_eq!(fs::read(destination.join("keep")).unwrap(), b"untouched");
        } else {
            assert!(!destination.exists());
            assert_eq!(fs::read_dir(root).unwrap().count(), 2);
        }
    }
}

#[cfg(unix)]
#[test]
fn static_sampler_source_and_license_links_reject_without_publication() {
    for source_link in [true, false] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let mut req = request(&root, &fixture().to_bytes().unwrap());
        let link = root.join(if source_link {
            "link.model.json"
        } else {
            "link.txt"
        });
        std::os::unix::fs::symlink(
            if source_link {
                &req.source
            } else {
                &req.license
            },
            &link,
        )
        .unwrap();
        if source_link {
            req.source = link;
        } else {
            req.license = link;
        }
        let destination = req.destination.clone();
        assert!(import_static_sampler_variant(req, variant()).is_err());
        assert!(!destination.exists());
    }
}
