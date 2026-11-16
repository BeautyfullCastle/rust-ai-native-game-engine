//! Morph targets of animated models: import, weight sampling, deformation and cooking.
#![cfg(all(feature = "animation", feature = "import"))]
#![allow(clippy::float_arithmetic)]
use base64::Engine;
use orr_model::{
    animation::{AnimatedModel, Interpolation, MAX_MORPH_TARGETS},
    animation_import::import_with_resolver,
    Error,
};
use serde_json::{json, Value};

fn source() -> Value {
    serde_json::from_slice(include_bytes!("fixtures/animated_strip.gltf")).unwrap()
}
fn import(value: &Value) -> Result<AnimatedModel, Error> {
    import_with_resolver("animated_strip.gltf", &serde_json::to_vec(value).unwrap(), |_| panic!("self-contained fixture"))
}
fn buffer(value: &Value) -> Vec<u8> {
    let uri = value["buffers"][0]["uri"].as_str().unwrap();
    base64::engine::general_purpose::STANDARD.decode(uri.split_once(',').unwrap().1).unwrap()
}

/// Appends `floats` to the buffer as a new accessor described by `fields`; returns its index.
fn add_accessor(value: &mut Value, floats: &[f32], fields: Value) -> usize {
    let mut bytes = buffer(value);
    bytes.resize(bytes.len().next_multiple_of(4), 0);
    let view = value["bufferViews"].as_array().unwrap().len();
    let data: Vec<u8> = floats.iter().flat_map(|f| f.to_le_bytes()).collect();
    value["bufferViews"].as_array_mut().unwrap().push(json!({"buffer": 0, "byteOffset": bytes.len(), "byteLength": data.len()}));
    bytes.extend(&data);
    value["buffers"][0]["uri"] = json!(format!("data:application/octet-stream;base64,{}", base64::engine::general_purpose::STANDARD.encode(&bytes)));
    value["buffers"][0]["byteLength"] = json!(bytes.len());
    let mut accessor = json!({"bufferView": view, "componentType": 5126});
    accessor.as_object_mut().unwrap().extend(fields.as_object().unwrap().clone());
    let accessors = value["accessors"].as_array_mut().unwrap();
    accessors.push(accessor);
    accessors.len() - 1
}

/// The strip with two targets (every vertex 1 m along x; every vertex 1 m along y)
/// at default weights 0.5 and 0, and a clip "face" moving both from 0 to 1 over a second.
fn morphed() -> Value {
    let mut v = source();
    let along = |axis: usize| (0..8).flat_map(|_| (0..3).map(move |c| if c == axis { 1.0 } else { 0.0 })).collect::<Vec<f32>>();
    let x = add_accessor(&mut v, &along(0), json!({"count": 8, "type": "VEC3"}));
    let y = add_accessor(&mut v, &along(1), json!({"count": 8, "type": "VEC3"}));
    let zero = add_accessor(&mut v, &[0.0; 24], json!({"count": 8, "type": "VEC3"}));
    v["meshes"][0]["primitives"][0]["targets"] = json!([{"POSITION": x}, {"POSITION": y, "NORMAL": zero}]);
    v["meshes"][0]["weights"] = json!([0.5, 0.0]);
    let times = add_accessor(&mut v, &[0.0, 1.0], json!({"count": 2, "type": "SCALAR", "min": [0], "max": [1]}));
    let weights = add_accessor(&mut v, &[0.0, 0.0, 1.0, 1.0], json!({"count": 4, "type": "SCALAR"}));
    v["animations"].as_array_mut().unwrap().push(json!({
        "name": "face",
        "samplers": [{"input": times, "output": weights, "interpolation": "LINEAR"}],
        "channels": [{"sampler": 0, "target": {"node": 3, "path": "weights"}}]
    }));
    v
}

/// One way to break the morphed fixture.
type Change = fn(&mut Value);

fn positions(model: &AnimatedModel, weights: &[Vec<f32>]) -> Vec<[f32; 3]> {
    let pose = model.rest_pose().unwrap();
    model.deform_morphed(&pose, weights).unwrap()[0].vertices.iter().map(|v| v.position).collect()
}

#[test]
fn import_reads_targets_default_weights_and_a_weights_track() {
    let model = import(&morphed()).unwrap();
    let morph = &model.source().morph;
    assert_eq!(morph.primitives.len(), 1);
    assert_eq!(morph.primitives[0].targets.len(), 2);
    assert_eq!(morph.primitives[0].weights, vec![0.5, 0.0]);
    assert!(morph.primitives[0].targets[0].normals.is_empty() && morph.primitives[0].targets[1].normals.len() == 8);
    let face = model.source().clips.iter().position(|c| c.name == "face").unwrap() as u32;
    assert!(model.source().clips[face as usize].channels.is_empty(), "a weights-only clip has no joint channels");
    assert_eq!(model.clip_duration(face).unwrap(), 1.0);
    assert_eq!(model.rest_weights(), vec![vec![0.5, 0.0]]);
    assert_eq!(model.sample_weights(face, 0.25).unwrap(), vec![vec![0.25, 0.25]]);
    assert_eq!(model.sample_weights(face, 9.0).unwrap(), vec![vec![1.0, 1.0]], "time clamps to the track");
    // Another clip leaves the defaults.
    assert_eq!(model.sample_weights(0, 0.5).unwrap(), model.rest_weights());
}

#[test]
fn weights_move_the_vertices_by_their_targets_before_skinning() {
    let model = import(&morphed()).unwrap();
    let base = positions(&model, &[vec![0.0, 0.0]]);
    for (weights, offset) in [([1.0, 0.0], [1.0, 0.0, 0.0]), ([0.5, 2.0], [0.5, 2.0, 0.0])] {
        for (moved, base) in positions(&model, &[weights.to_vec()]).iter().zip(&base) {
            let d: Vec<f32> = (0..3).map(|c| moved[c] - base[c]).collect();
            assert!(d.iter().zip(offset).all(|(a, b)| (a - b).abs() < 1e-4), "weights {weights:?} move a vertex by {d:?}");
        }
    }
    // The plain deformation uses the default weights.
    assert_eq!(model.deform(&model.rest_pose().unwrap()).unwrap()[0].vertices.iter().map(|v| v.position).collect::<Vec<_>>(), positions(&model, &[vec![0.5, 0.0]]));
    let pose = model.rest_pose().unwrap();
    assert!(model.deform_morphed(&pose, &[vec![1.0]]).is_err(), "one weight per target");
    assert!(model.deform_morphed(&pose, &[vec![f32::NAN, 0.0]]).is_err());
}

#[test]
fn cooking_keeps_the_targets_and_leaves_models_without_them_unchanged() {
    let model = import(&morphed()).unwrap();
    let cooked = AnimatedModel::from_bytes(&model.to_bytes().unwrap()).unwrap();
    assert_eq!(cooked.source(), model.source());
    let plain = import(&source()).unwrap().to_bytes().unwrap();
    assert!(!String::from_utf8(plain).unwrap().contains("\"morph\""), "an empty morph section is not written");
}

#[test]
fn malformed_targets_and_weight_tracks_reject() {
    let cases: [(&str, Change); 7] = [
        ("empty targets", |v| v["meshes"][0]["primitives"][0]["targets"] = json!([])),
        ("UV offsets", |v| v["meshes"][0]["primitives"][0]["targets"][0]["TEXCOORD_0"] = json!(2)),
        ("mesh weights count", |v| v["meshes"][0]["weights"] = json!([1.0])),
        ("node weights count", |v| v["nodes"][3]["weights"] = json!([1.0, 0.0, 0.0])),
        ("cubic weights", |v| {
            let last = v["animations"].as_array().unwrap().len() - 1;
            v["animations"][last]["samplers"][0]["interpolation"] = json!("CUBICSPLINE");
        }),
        ("weights on a node without targets", |v| {
            let last = v["animations"].as_array().unwrap().len() - 1;
            v["animations"][last]["channels"][0]["target"]["node"] = json!(2);
        }),
        ("too many targets", |v| {
            let target = v["meshes"][0]["primitives"][0]["targets"][0].clone();
            v["meshes"][0]["primitives"][0]["targets"] = json!(vec![target; MAX_MORPH_TARGETS + 1]);
            v["meshes"][0]["weights"] = json!(vec![0.0; MAX_MORPH_TARGETS + 1]);
        }),
    ];
    for (name, change) in cases {
        let mut v = morphed();
        change(&mut v);
        assert!(import(&v).is_err(), "accepted {name}");
    }
    // A weights track with a value count that is not one per target per key.
    let mut v = morphed();
    let short = add_accessor(&mut v, &[0.0, 0.0, 1.0], json!({"count": 3, "type": "SCALAR"}));
    let last = v["animations"].as_array().unwrap().len() - 1;
    v["animations"][last]["samplers"][0]["output"] = json!(short);
    assert!(import(&v).is_err(), "accepted a short weights track");
}

#[test]
fn cooked_morph_data_is_checked_on_load() {
    let model = import(&morphed()).unwrap();
    let mut s = model.source().clone();
    s.morph.primitives[0].targets[0].positions.pop();
    assert!(AnimatedModel::new(s).is_err(), "an offset per vertex");
    let mut s = model.source().clone();
    s.morph.channels[0].interpolation = Interpolation::CubicSpline;
    assert!(AnimatedModel::new(s).is_err(), "step and linear only");
    let mut s = model.source().clone();
    s.morph.primitives[0].weights = vec![100.0, 0.0];
    assert!(AnimatedModel::new(s).is_err(), "weights within the limit");
    let mut s = model.source().clone();
    s.morph.channels[0].clip = 99;
    assert!(AnimatedModel::new(s).is_err(), "the clip exists");
}

fn joint_keys(s: &orr_model::animation::AnimatedSource) -> usize {
    use orr_model::animation::ChannelValues::{Rotation, Scale, Translation};
    s.clips
        .iter()
        .flat_map(|c| &c.channels)
        .map(|c| match &c.values {
            Translation(v) | Scale(v) => v.len(),
            Rotation(v) => v.len(),
        })
        .sum()
}

#[test]
fn cooked_morph_channels_must_be_unique_per_clip_and_node() {
    let model = import(&morphed()).unwrap();
    let mut s = model.source().clone();
    let mut other = s.morph.channels[0].clone();
    other.weights = vec![1.0, 1.0, 0.0, 0.0];
    s.morph.channels.push(other);
    assert!(AnimatedModel::new(s.clone()).is_err(), "a second track for the same clip and node");
    s.morph.channels.swap(0, 1);
    assert!(AnimatedModel::new(s).is_err(), "the order does not matter");
}

#[test]
fn morph_keys_count_against_the_animation_key_budget() {
    let model = import(&morphed()).unwrap();
    let channel = &model.source().morph.channels[0];
    let k = channel.weights.len() / channel.times.len();
    let room = orr_model::animation::MAX_ANIMATION_KEYS - joint_keys(model.source());
    let with_keys = |keys: usize| {
        let mut s = model.source().clone();
        let n = keys / k;
        s.morph.channels[0].times = (0..n).map(|i| i as f32).collect();
        s.morph.channels[0].weights = vec![0.0; n * k];
        AnimatedModel::new(s)
    };
    assert!(with_keys(room).is_ok(), "keys up to the budget");
    assert!(with_keys(room + k).is_err(), "one more key than the budget");
}

#[test]
fn morph_channels_count_against_the_clip_channel_limit() {
    use orr_model::animation::{AnimationChannel, ChannelValues, MAX_CHANNELS};
    let model = import(&morphed()).unwrap();
    let mut s = model.source().clone();
    let clip = s.morph.channels[0].clip as usize;
    let mut template = s.nodes[0].clone();
    template.children.clear();
    s.nodes.resize(100, template);
    let track = |node: u32, values| AnimationChannel { node, interpolation: Interpolation::Step, times: vec![0.0], values };
    s.clips[clip].channels = (0..100u32)
        .flat_map(|n| {
            [
                track(n, ChannelValues::Translation(vec![[0.0; 3]])),
                track(n, ChannelValues::Scale(vec![[1.0; 3]])),
                track(n, ChannelValues::Rotation(vec![[0.0, 0.0, 0.0, 1.0]])),
            ]
        })
        .take(MAX_CHANNELS - 1)
        .collect();
    assert!(AnimatedModel::new(s.clone()).is_ok(), "joint tracks and one weights track fill the limit");
    s.clips[clip].channels.push(track(99, ChannelValues::Translation(vec![[0.0; 3]])));
    assert!(AnimatedModel::new(s).is_err(), "one track past the limit");
}
