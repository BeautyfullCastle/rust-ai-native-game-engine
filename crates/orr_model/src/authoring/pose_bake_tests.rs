use super::*;
use crate::animation::{
    AnimatedNode, AnimatedPrimitive, AnimatedSource, AnimationChannel, AnimationClip,
    ChannelValues, Interpolation, Skin, SkinnedVertex, Trs,
};
use crate::{Image, Material, Vertex, Wrap, IDENTITY};
use serde_json::{json, Value};

const SOURCE: &str = "input.animated.json";
const ID: &str = "variants/public.pose.json";

// Independent cooked fixture from the existing producer contract: two weighted joints,
// nontrivial inverse binds, ancestor/nonuniform transforms and a separately rigid mesh.
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

fn recipe(clip: u32, time: f64) -> Value {
    json!({"format":"orr_animation_pose_bake","version":1,"source":SOURCE,"clip":clip,"time":time})
}
fn apply(input: &[u8], source: &[u8]) -> Result<StaticModel, String> {
    let mut calls = 0;
    let out = bake_pose(input, ID, |path| {
        calls += 1;
        assert_eq!(path, SOURCE);
        assert_eq!(calls, 1, "resolve only the captured direct source once");
        Ok(source.to_vec())
    })?;
    assert_eq!(calls, 1);
    Ok(out)
}
fn near(actual: [f32; 3], expected: [f32; 3]) {
    for (a, e) in actual.into_iter().zip(expected) {
        assert!((a - e).abs() < 0.0001, "{actual:?} != {expected:?}");
    }
}
fn normalized(vector: [f32; 3]) -> [f32; 3] {
    let length = vector.iter().map(|v| v * v).sum::<f32>().sqrt();
    vector.map(|v| v / length)
}

#[test]
fn public_pose_api_matches_weighted_inverse_bind_and_rigid_literal_oracles_and_preserves_tables() {
    let original = fixture();
    let source = original.to_bytes().unwrap();
    for time in [0.0, 0.5, 1.0] {
        let input = serde_json::to_vec(&recipe(0, time)).unwrap();
        let out = apply(&input, &source).unwrap();
        assert_eq!(out.source().format, "orr_static_model");
        assert_eq!(out.source().asset_id, ID);
        assert_eq!(out.source().primitives.len(), 2);
        assert_eq!(out.source().images, original.source().images);
        assert_eq!(
            serde_json::to_vec(&out.source().materials).unwrap(),
            serde_json::to_vec(&original.source().materials).unwrap()
        );
        assert_eq!(
            out.source().dependencies,
            vec![
                Dependency {
                    uri: "$source".into(),
                    sha256: digest(&input)
                },
                Dependency {
                    uri: SOURCE.into(),
                    sha256: digest(&source)
                }
            ]
        );
        for (index, primitive) in out.source().primitives.iter().enumerate() {
            let old = &original.source().primitives[index];
            assert_eq!(
                primitive.transform.map(|column| column.map(f32::to_bits)),
                IDENTITY.map(|column| column.map(f32::to_bits))
            );
            assert_eq!(
                primitive.id,
                format!(
                    "{ID}{}",
                    old.id.strip_prefix(&original.source().asset_id).unwrap()
                )
            );
            assert_eq!(primitive.indices, old.indices);
            assert_eq!(primitive.material, old.material);
            for (vertex, old_vertex) in primitive.vertices.iter().zip(&old.vertices) {
                let q = old_vertex.vertex.position;
                // Literal model-space affine algebra; no sample/deform/matrix helper.
                // The skinned mesh's large independent transform is deliberately absent.
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
                near(
                    vertex.normal,
                    normalized(if index == 0 {
                        [0.6 / 3.5, 0.8 / 3.75, 0.0]
                    } else {
                        [0.6 / 10.0, 0.8 / 6.0, 0.0]
                    }),
                );
                assert_eq!(
                    vertex.uv.map(f32::to_bits),
                    old_vertex.vertex.uv.map(f32::to_bits)
                );
            }
        }
        let bytes = out.to_bytes().unwrap();
        assert_eq!(apply(&input, &source).unwrap().to_bytes().unwrap(), bytes);
        assert_eq!(
            StaticModel::from_bytes(&bytes).unwrap().to_bytes().unwrap(),
            bytes
        );
    }
    assert_eq!(original.to_bytes().unwrap(), source);
}

#[test]
fn public_pose_api_checks_original_f64_time_and_exact_clip_end_without_looping() {
    let original = fixture();
    let source = original.to_bytes().unwrap();
    let above = 1.0 + f64::EPSILON;
    assert_eq!(above as f32, 1.0);
    assert!(
        apply(&serde_json::to_vec(&recipe(0, above)).unwrap(), &source)
            .unwrap_err()
            .contains("duration")
    );
    assert_eq!(f64::MIN_POSITIVE as f32, 0.0);
    assert!(apply(
        &serde_json::to_vec(&recipe(0, f64::MIN_POSITIVE)).unwrap(),
        &source
    )
    .unwrap_err()
    .contains("nonzero f32"));
    assert!(apply(
        &serde_json::to_vec(&recipe(u32::MAX, 0.0)).unwrap(),
        &source
    )
    .unwrap_err()
    .contains("clip index"));
    for (clip, time, skin_x, rigid_x) in [
        (0, 0.0, 8.75, 24.0),
        (0, 1.0, 10.75, 26.0),
        (1, 0.0, 48.75, 64.0),
        (1, 1.0, 58.75, 74.0),
    ] {
        let out = apply(&serde_json::to_vec(&recipe(clip, time)).unwrap(), &source).unwrap();
        near(
            out.source().primitives[0].vertices[0].position,
            [skin_x, 21.5, 34.5],
        );
        near(
            out.source().primitives[1].vertices[0].position,
            [rigid_x, 44.0, 66.0],
        );
    }
    assert_eq!(original.to_bytes().unwrap(), source);
}

#[test]
fn public_pose_api_rejects_strict_recipe_before_resolution_and_invalid_full_source() {
    let valid = recipe(0, 0.5);
    for (field, value) in [
        ("time", json!(-1e-50)),
        ("time", json!("0.5")),
        ("time", json!(null)),
        ("clip", json!(0.0)),
        ("clip", json!(-1)),
        ("version", json!(2)),
        ("source", json!("../input.animated.json")),
    ] {
        let mut invalid = valid.clone();
        invalid[field] = value;
        assert!(
            bake_pose(&serde_json::to_vec(&invalid).unwrap(), ID, |path| panic!(
                "invalid recipe resolved {path}"
            ))
            .is_err()
        );
    }
    for key in ["format", "version", "source", "clip", "time"] {
        let mut missing = valid.clone();
        missing.as_object_mut().unwrap().remove(key);
        assert!(
            bake_pose(&serde_json::to_vec(&missing).unwrap(), ID, |path| panic!(
                "missing field resolved {path}"
            ))
            .is_err()
        );
        let text = serde_json::to_string(&valid).unwrap();
        let duplicate = format!("{{\"{key}\":{},{}", valid[key], &text[1..]);
        assert!(bake_pose(duplicate.as_bytes(), ID, |path| panic!(
            "duplicate field resolved {path}"
        ))
        .is_err());
    }
    let mut extra = valid.clone();
    extra["loop"] = json!(true);
    let mut oversized = serde_json::to_vec(&valid).unwrap();
    oversized.resize(MAX_RECIPE_BYTES + 1, b' ');
    let positional = br#"["orr_animation_pose_bake",1,"input.animated.json",0,0.5]"#.to_vec();
    for input in [serde_json::to_vec(&extra).unwrap(), oversized, positional] {
        assert!(bake_pose(&input, ID, |path| panic!("invalid shape resolved {path}")).is_err());
    }
    let input = serde_json::to_vec(&valid).unwrap();
    for id in ["../bad.pose.json", "bad.model.json", "CON.pose.json"] {
        assert!(bake_pose(&input, id, |path| panic!(
            "invalid identity resolved {path}"
        ))
        .is_err());
    }
    assert!(apply(&input, b"bad source").is_err());
    let mut invalid = fixture().source().clone();
    invalid.images.push(Image {
        width: 1,
        height: 1,
        rgba8: vec![0; 3],
    });
    assert!(apply(&input, &serde_json::to_vec(&invalid).unwrap())
        .unwrap_err()
        .contains("source animated model"));
    let error = bake_pose(&input, ID, |_| Err("original resolver failure".into())).unwrap_err();
    assert_eq!(error, "original resolver failure");
}
