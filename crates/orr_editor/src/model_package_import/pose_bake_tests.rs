#![allow(clippy::float_arithmetic)]
use super::*;
fn variant() -> PoseBakeVariant {
    PoseBakeVariant {
        clip: 0,
        time_seconds: 0.5,
        asset_id: "variants/statue.pose.json".into(),
    }
}
fn unopened() -> Request {
    Request {
        source: "missing.animated.json".into(),
        license: "missing-license".into(),
        destination: "missing-parent/output".into(),
        name: "pose-statue".into(),
        version: "1.0.0".into(),
        kind: ModelKind::Animated,
        source_format: SourceFormat::CookedAnimated,
        license_acknowledged: true,
    }
}
#[test]
fn pose_spec_rejects_invalid_identity_time_and_narrowing_without_io() {
    for id in [
        "",
        "../bad.pose.json",
        "CON.pose.json",
        "bad.uv.json",
        "x//bad.pose.json",
    ] {
        let mut v = variant();
        v.asset_id = id.into();
        assert!(validate_pose_bake_variant(&v).is_err());
    }
    for time in [
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        -f64::MIN_POSITIVE,
        -0.1,
        f64::MAX,
        f64::MIN_POSITIVE,
    ] {
        let mut v = variant();
        v.time_seconds = time;
        assert!(validate_pose_bake_variant(&v).is_err());
    }
    for time in [-0.0, 0.0, f64::from(f32::from_bits(1)), f64::from(f32::MAX)] {
        let mut v = variant();
        v.time_seconds = time;
        validate_pose_bake_variant(&v).unwrap();
    }
}
#[test]
fn pose_input_license_mode_format_and_hash_reject_before_paths() {
    let mut r = unopened();
    r.license_acknowledged = false;
    assert!(import_pose_bake_variant(r, variant())
        .err()
        .unwrap()
        .contains("acknowledgement"));
    let mut r = unopened();
    r.kind = ModelKind::Static;
    assert!(import_pose_bake_variant(r, variant())
        .err()
        .unwrap()
        .contains("Animated model mode"));
    for format in [SourceFormat::RawGlb, SourceFormat::CookedStatic] {
        let mut r = unopened();
        r.source_format = format;
        assert!(import_pose_bake_variant(r, variant())
            .err()
            .unwrap()
            .contains("CookedAnimated"));
    }
    for hash in ["".into(), "bad".into(), "z".repeat(64), "a".repeat(65)] {
        assert!(
            import_pose_bake_variant_checked(unopened(), variant(), Some(hash))
                .err()
                .unwrap()
                .contains("64 hexadecimal")
        );
    }
}
#[cfg(not(feature = "room-animated-models"))]
#[test]
fn pose_feature_off_rejects_before_paths_and_cannot_generate_payload() {
    assert!(import_pose_bake_variant(unopened(), variant())
        .err()
        .unwrap()
        .contains("room-animated-models"));
    assert!(
        pose_bake_variant_bytes(b"unopened animated bytes", &variant())
            .unwrap_err()
            .contains("room-animated-models")
    );
}
#[cfg(feature = "room-animated-models")]
mod enabled {
    use super::*;
    use orr_model::{
        animation::*, Dependency, Image, Material, StaticModel, Vertex, Wrap, IDENTITY,
    };
    // Rich independent input also used by the offline producer contract tests.
    fn fixture() -> AnimatedModel {
        let node = |name: &str, children, translation, scale| AnimatedNode {
            name: name.into(),
            children,
            rest: Trs {
                translation,
                scale,
                ..Trs::default()
            },
        };
        let vertices: Vec<_> = [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]]
            .into_iter()
            .enumerate()
            .map(|(i, position)| SkinnedVertex {
                vertex: Vertex {
                    position,
                    normal: [0.6, 0.8, 0.],
                    uv: if i == 0 { [-0., 0.] } else { [i as f32, -0.] },
                },
                joints: [0, 1, 0, 0],
                weights: [0.25, 0.75, 0., 0.],
            })
            .collect();
        let mut rigid = vertices.clone();
        for v in &mut rigid {
            v.joints = [0; 4];
            v.weights = [1., 0., 0., 0.];
        }
        let mut ib0 = IDENTITY;
        ib0[3][0] = -0.5;
        let mut ib1 = IDENTITY;
        ib1[3] = [0., -1., 1., 1.];
        AnimatedModel::new(AnimatedSource {
            format: "orr_animated_model".into(),
            version: 1,
            asset_id: "actors/original.glb".into(),
            dependencies: vec![
                Dependency {
                    uri: "$source".into(),
                    sha256: digest(b"original"),
                },
                Dependency {
                    uri: "historical/missing.png".into(),
                    sha256: digest(b"missing"),
                },
            ],
            images: vec![
                Image {
                    width: 1,
                    height: 1,
                    rgba8: vec![19, 37, 71, 103],
                },
                Image {
                    width: 1,
                    height: 1,
                    rgba8: vec![2, 3, 5, 7],
                },
            ],
            materials: vec![
                Material {
                    base_color: [-0., 0.5, 0.75, 0.25],
                    image: 0,
                    linear_filter: true,
                    wrap_s: Wrap::Mirror,
                    wrap_t: Wrap::Repeat,
                },
                Material {
                    base_color: [1., 0.25, 0., 1.],
                    image: 1,
                    linear_filter: false,
                    wrap_s: Wrap::Clamp,
                    wrap_t: Wrap::Mirror,
                },
            ],
            nodes: vec![
                node("ancestor", vec![1, 2, 3, 4], [10., 20., 30.], [2., 3., 4.]),
                node("joint-a", vec![], [1., 2., 3.], [1., 2., 1.]),
                node("joint-b", vec![], [-1., 1., 0.], [2., 1., 0.5]),
                node("skinned-mesh", vec![], [100., 200., 300.], [3., 5., 7.]),
                node("rigid-mesh", vec![], [7., 8., 9.], [5., 2., 1.]),
            ],
            skins: vec![Skin {
                name: "skin".into(),
                joints: vec![1, 2],
                inverse_bind_matrices: vec![ib0, ib1],
            }],
            primitives: vec![
                AnimatedPrimitive {
                    id: "actors/original.glb#node=3/mesh=0/primitive=0".into(),
                    node: 3,
                    skin: Some(0),
                    vertices,
                    indices: vec![0, 1, 2],
                    material: 0,
                },
                AnimatedPrimitive {
                    id: "actors/original.glb#node=4/mesh=1/primitive=0".into(),
                    node: 4,
                    skin: None,
                    vertices: rigid,
                    indices: vec![2, 1, 0],
                    material: 1,
                },
            ],
            clips: vec![
                AnimationClip {
                    name: "move".into(),
                    channels: vec![AnimationChannel {
                        node: 0,
                        interpolation: Interpolation::Linear,
                        times: vec![0., 1.],
                        values: ChannelValues::Translation(vec![[10., 20., 30.], [12., 20., 30.]]),
                    }],
                },
                AnimationClip {
                    name: "other".into(),
                    channels: vec![AnimationChannel {
                        node: 0,
                        interpolation: Interpolation::Step,
                        times: vec![0., 1.],
                        values: ChannelValues::Translation(vec![[50., 20., 30.], [60., 20., 30.]]),
                    }],
                },
            ],
        })
        .unwrap()
    }

    fn request(root: &Path, source: &[u8]) -> Request {
        let input = root.join("source.animated.json");
        let license = root.join("LICENSE.txt");
        fs::write(&input, source).unwrap();
        fs::write(&license, b"Redistribution license\n").unwrap();
        Request {
            source: input,
            license,
            destination: root.join("new-package"),
            ..unopened()
        }
    }
    fn near(a: [f32; 3], b: [f32; 3]) {
        for (a, b) in a.into_iter().zip(b) {
            assert!((a - b).abs() < 0.0001, "{a} != {b}");
        }
    }
    #[test]
    fn pose_weighted_and_rigid_samples_match_hand_geometry_and_preserve_payload() {
        let original = fixture();
        let source = original.to_bytes().unwrap();
        for time in [0.0, 0.5, 1.0] {
            let mut v = variant();
            v.time_seconds = time;
            let bytes = pose_bake_variant_bytes(&source, &v).unwrap();
            assert_eq!(bytes, pose_bake_variant_bytes(&source, &v).unwrap());
            let out = StaticModel::from_bytes(&bytes).unwrap();
            assert_eq!(out.source().images, original.source().images);
            assert_eq!(
                serde_json::to_vec(&out.source().materials).unwrap(),
                serde_json::to_vec(&original.source().materials).unwrap()
            );
            for (index, p) in out.source().primitives.iter().enumerate() {
                let old = &original.source().primitives[index];
                assert_eq!(p.transform, IDENTITY);
                assert_eq!(p.indices, old.indices);
                assert_eq!(p.material, old.material);
                assert_eq!(p.id, old.id.replacen("actors/original.glb", &v.asset_id, 1));
                for (vertex, old) in p.vertices.iter().zip(&old.vertices) {
                    let q = old.vertex.position;
                    // Literal affine algebra with inverse binds and weighted scales;
                    // no entity placement or extra skinned mesh-node transform.
                    let expected = if index == 0 {
                        [
                            3.5 * q[0] + 8.75 + 2.0 * time as f32,
                            3.75 * q[1] + 21.5,
                            2.5 * q[2] + 34.5,
                        ]
                    } else {
                        [
                            10.0 * q[0] + 24.0 + 2.0 * time as f32,
                            6.0 * q[1] + 44.0,
                            4.0 * q[2] + 66.0,
                        ]
                    };
                    near(vertex.position, expected);
                    let normal = if index == 0 {
                        [0.6f32 / 3.5, 0.8 / 3.75, 0.0]
                    } else {
                        [0.6f32 / 10.0, 0.8 / 6.0, 0.0]
                    };
                    let length = normal.iter().map(|n| n * n).sum::<f32>().sqrt();
                    near(vertex.normal, normal.map(|n| n / length));
                    assert_eq!(vertex.uv.map(f32::to_bits), old.vertex.uv.map(f32::to_bits));
                }
            }
            assert_eq!(out.source().dependencies[1].uri, "input.animated.json");
            assert_eq!(out.source().dependencies[1].sha256, digest(&source));
            if time == 0.5 {
                assert_eq!(out.source().dependencies[0].sha256,digest(br#"{"format":"orr_animation_pose_bake","version":1,"source":"input.animated.json","clip":0,"time":0.5}"#));
            }
        }
        assert_eq!(original.to_bytes().unwrap(), source);
    }
    #[test]
    fn pose_duration_f64_bounds_and_exact_clip_selection_precede_sampling() {
        let bytes = fixture().to_bytes().unwrap();
        let mut v = variant();
        v.time_seconds = 1.0 + f64::EPSILON;
        assert!(pose_bake_variant_bytes(&bytes, &v)
            .unwrap_err()
            .contains("duration"));
        v.time_seconds = 1.0;
        v.clip = u32::MAX;
        assert!(pose_bake_variant_bytes(&bytes, &v)
            .unwrap_err()
            .contains("clip index"));
        v.clip = 1;
        let out = StaticModel::from_bytes(&pose_bake_variant_bytes(&bytes, &v).unwrap()).unwrap();
        // STEP clip at final key must not loop to its first key.
        near(
            out.source().primitives[1].vertices[0].position,
            [74.0, 44.0, 66.0],
        );
    }
    #[test]
    fn pose_singular_weighted_deformation_fails_without_partial_package_or_rest_fallback() {
        let mut source = fixture().source().clone();
        for node in &mut source.nodes {
            node.children.clear();
            node.rest = Trs::default();
        }
        source.nodes[4].children = vec![0, 1, 2, 3];
        source.primitives.truncate(1);
        source.skins[0].joints = vec![0, 1];
        source.skins[0].inverse_bind_matrices = vec![IDENTITY; 2];
        for vertex in &mut source.primitives[0].vertices {
            vertex.joints = [0, 1, 0, 0];
            vertex.weights = [0.5, 0.5, 0.0, 0.0];
        }
        source.clips = vec![AnimationClip {
            name: "singular".into(),
            channels: vec![AnimationChannel {
                node: 1,
                interpolation: Interpolation::Step,
                times: vec![0.0, 1.0],
                values: ChannelValues::Rotation(vec![[0.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 0.0]]),
            }],
        }];
        let bytes = AnimatedModel::new(source).unwrap().to_bytes().unwrap();
        let mut v = variant();
        v.time_seconds = 0.0;
        assert!(pose_bake_variant_bytes(&bytes, &v).is_ok());
        v.time_seconds = 1.0;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let req = request(&root, &bytes);
        let destination = req.destination.clone();
        let error = import_pose_bake_variant(req, v).err().unwrap();
        assert!(error.contains("singular"), "{error}");
        assert!(!destination.exists());
        assert_eq!(fs::read_dir(root).unwrap().count(), 2);
    }
    #[test]
    fn pose_static_vertex_bounds_reject_valid_animated_input_without_publication() {
        let mut source = fixture().source().clone();
        for vertex in &mut source.primitives[0].vertices {
            vertex.vertex.position[0] = 300_000.0;
        }
        let bytes = AnimatedModel::new(source).unwrap().to_bytes().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let req = request(&root, &bytes);
        let destination = req.destination.clone();
        let error = import_pose_bake_variant(req, variant()).err().unwrap();
        assert!(error.contains("baked static model"), "{error}");
        assert!(!destination.exists());
        assert_eq!(fs::read_dir(root).unwrap().count(), 2);
    }
    #[test]
    fn pose_import_converts_only_output_to_static_and_keeps_original_animated_input() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let bytes = fixture().to_bytes().unwrap();
        let req = request(&root, &bytes);
        let source = req.source.clone();
        let license = req.license.clone();
        let report =
            import_pose_bake_variant_checked(req, variant(), Some(digest(&bytes).to_uppercase()))
                .unwrap();
        assert_eq!(report.kind, ModelKind::Static);
        assert_eq!(report.asset, "model.orrmodel");
        let output = fs::read(report.root.join(&report.asset)).unwrap();
        StaticModel::from_bytes(&output).unwrap();
        assert_eq!(output, pose_bake_variant_bytes(&bytes, &variant()).unwrap());
        assert_eq!(report.source_sha256, digest(&output));
        assert_eq!(fs::read(&source).unwrap(), bytes);
        AnimatedModel::from_bytes(&fs::read(source).unwrap()).unwrap();
        assert_eq!(
            fs::read(report.root.join("LICENSE.txt")).unwrap(),
            fs::read(&license).unwrap()
        );
        assert_eq!(report.license_sha256, digest(&fs::read(license).unwrap()));
        let package =
            orr_package::inspect(&report.root, &orr_sample::room_project::compiled_runtime())
                .unwrap();
        assert_eq!(
            package.manifest.capabilities,
            BTreeSet::from(["models".into()])
        );
        assert_eq!(
            package.manifest.files,
            BTreeSet::from(["LICENSE.txt".into(), "model.orrmodel".into()])
        );
        assert!(!report.root.join("model.animated.json").exists());
    }
    #[test]
    fn pose_ordinary_animated_import_keeps_original_kind_filename_and_capabilities() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let bytes = fixture().to_bytes().unwrap();
        let report = import_package(request(&root, &bytes)).unwrap();
        assert_eq!(report.kind, ModelKind::Animated);
        assert_eq!(report.asset, "model.animated.json");
        assert_eq!(fs::read(report.root.join(&report.asset)).unwrap(), bytes);
        let package =
            orr_package::inspect(&report.root, &orr_sample::room_project::compiled_runtime())
                .unwrap();
        assert_eq!(
            package.manifest.capabilities,
            BTreeSet::from(["animation".into(), "models".into()])
        );
    }
    #[test]
    fn pose_stale_hash_precedes_license_payload_and_never_publishes() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let bytes = fixture().to_bytes().unwrap();
        let req = request(&root, &bytes);
        let destination = req.destination.clone();
        fs::write(&req.source, b"changed cooked bytes").unwrap();
        fs::write(&req.license, [0xff]).unwrap();
        let error = import_pose_bake_variant_checked(req, variant(), Some(digest(&bytes)))
            .err()
            .unwrap();
        assert!(
            error.contains("inspected animated source changed"),
            "{error}"
        );
        assert!(!destination.exists());
        assert_eq!(fs::read_dir(root).unwrap().count(), 2);
    }
    #[test]
    fn pose_invalid_input_license_and_existing_destination_never_publish_or_replace() {
        for case in 0..3 {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().canonicalize().unwrap();
            let req = request(&root, &fixture().to_bytes().unwrap());
            let destination = req.destination.clone();
            match case {
                0 => fs::write(&req.source, b"not animated").unwrap(),
                1 => fs::write(&req.license, [0xff]).unwrap(),
                _ => {
                    fs::create_dir(&destination).unwrap();
                    fs::write(destination.join("keep"), b"untouched").unwrap();
                }
            }
            assert!(import_pose_bake_variant(req, variant()).is_err());
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
    fn pose_linked_source_or_license_never_publish() {
        for source_link in [true, false] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().canonicalize().unwrap();
            let mut req = request(&root, &fixture().to_bytes().unwrap());
            let link = root.join(if source_link {
                "link.animated.json"
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
            assert!(import_pose_bake_variant(req, variant()).is_err());
            assert!(!destination.exists());
        }
    }
}
