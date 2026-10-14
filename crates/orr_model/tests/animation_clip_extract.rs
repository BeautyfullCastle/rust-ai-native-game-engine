#![cfg(all(feature = "animation", feature = "import"))]
#![allow(clippy::float_arithmetic)]

#[path = "../examples/animation_clip_extract/authoring.rs"]
mod authoring;

use authoring::{extract_glb, ExtractOptions};
use orr_model::{
    animation::{AnimatedModel, ChannelValues, Interpolation, Pose, MAX_ANIMATION_KEYS, MAX_CLIPS},
    animation_import::import_with_resolver,
};
use serde_json::{json, Value};
use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

const FIXTURE: &[u8] = include_bytes!("fixtures/animated_strip.glb");
const DEMO: &[u8] = include_bytes!("../../../assets/animation_demo/animated.glb");

// The fixture writer only serializes independent source data. It never samples,
// slices, or calls the extraction helper to construct an expected result.
fn split_glb(bytes: &[u8]) -> (Value, Vec<u8>) {
    let word = |offset| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
    assert_eq!(&bytes[..4], b"glTF");
    assert_eq!(word(4), 2);
    assert_eq!(word(8) as usize, bytes.len());
    let json_len = word(12) as usize;
    assert_eq!(word(16), 0x4e4f534a);
    let bin_header = 20 + json_len;
    assert_eq!(word(bin_header + 4), 0x004e4942);
    let bin_len = word(bin_header) as usize;
    assert_eq!(bin_header + 8 + bin_len, bytes.len());
    (
        serde_json::from_slice(&bytes[20..bin_header]).unwrap(),
        bytes[bin_header + 8..].to_vec(),
    )
}

fn pack_glb(document: &Value, binary: &[u8]) -> Vec<u8> {
    let mut json = serde_json::to_vec(document).unwrap();
    while json.len() % 4 != 0 {
        json.push(b' ');
    }
    let mut binary = binary.to_vec();
    while binary.len() % 4 != 0 {
        binary.push(0);
    }
    let total = 12 + 8 + json.len() + 8 + binary.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(&0x4e4f534au32.to_le_bytes());
    out.extend(json);
    out.extend_from_slice(&(binary.len() as u32).to_le_bytes());
    out.extend_from_slice(&0x004e4942u32.to_le_bytes());
    out.extend(binary);
    out
}

fn import(bytes: &[u8]) -> AnimatedModel {
    import_with_resolver("test.glb", bytes, |uri| {
        panic!("self-contained fixture attempted to resolve {uri}")
    })
    .unwrap()
}

fn options(start: f32, end: f32) -> ExtractOptions {
    ExtractOptions {
        clip_index: 0,
        start,
        end,
        name: "extracted".into(),
    }
}

struct Fixture {
    document: Value,
    binary: Vec<u8>,
}

impl Fixture {
    fn new() -> Self {
        let (mut document, binary) = split_glb(FIXTURE);
        document["animations"] = json!([{"name":"source","samplers":[],"channels":[]}]);
        Self { document, binary }
    }

    fn accessor(&mut self, kind: &str, width: usize, values: &[f32]) -> usize {
        assert_eq!(values.len() % width, 0);
        while self.binary.len() % 4 != 0 {
            self.binary.push(0);
        }
        let view_index = self.document["bufferViews"].as_array().unwrap().len();
        self.document["bufferViews"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "buffer":0,"byteOffset":self.binary.len(),"byteLength":values.len()*4
            }));
        for value in values {
            self.binary.extend_from_slice(&value.to_le_bytes());
        }
        let index = self.document["accessors"].as_array().unwrap().len();
        let mut accessor = json!({
            "bufferView":view_index,"componentType":5126,
            "count":values.len()/width,"type":kind
        });
        if kind == "SCALAR" {
            accessor["min"] = json!([values[0]]);
            accessor["max"] = json!([values[values.len() - 1]]);
        }
        self.document["accessors"]
            .as_array_mut()
            .unwrap()
            .push(accessor);
        index
    }

    fn track(&mut self, node: u32, path: &str, interpolation: &str, times: &[f32], values: &[f32]) {
        let width = if path == "rotation" { 4 } else { 3 };
        assert_eq!(values.len(), times.len() * width);
        let input = self.accessor("SCALAR", 1, times);
        let output = self.accessor(if width == 4 { "VEC4" } else { "VEC3" }, width, values);
        let clip = &mut self.document["animations"][0];
        let sampler = clip["samplers"].as_array().unwrap().len();
        clip["samplers"].as_array_mut().unwrap().push(json!({
            "input":input,"output":output,"interpolation":interpolation
        }));
        clip["channels"].as_array_mut().unwrap().push(json!({
            "sampler":sampler,"target":{"node":node,"path":path}
        }));
    }

    fn bytes(&self) -> Vec<u8> {
        let mut document = self.document.clone();
        document["buffers"][0]["byteLength"] = json!(self.binary.len());
        pack_glb(&document, &self.binary)
    }
}

fn close(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() <= 0.00005,
        "{actual} differs from expected {expected}"
    );
}

fn close_array<const N: usize>(actual: [f32; N], expected: [f32; N]) {
    for (actual, expected) in actual.into_iter().zip(expected) {
        close(actual, expected);
    }
}

fn close_pose(actual: &Pose, expected: &Pose) {
    assert_eq!(actual.local().len(), expected.local().len());
    for (a, b) in actual.local().iter().zip(expected.local()) {
        close_array(a.translation, b.translation);
        close_array(a.scale, b.scale);
        // q and -q express the same rotation; compare transforms as well.
        let dot: f32 = a.rotation.iter().zip(b.rotation).map(|(a, b)| a * b).sum();
        close(dot.abs(), 1.0);
    }
    for (a, b) in actual.global().iter().zip(expected.global()) {
        for (a, b) in a.iter().zip(b) {
            close_array(*a, *b);
        }
    }
}

fn close_deformation(a: &AnimatedModel, ai: u32, at: f32, b: &AnimatedModel, bi: u32, bt: f32) {
    let ap = a.sample_clip(ai, at).unwrap();
    let bp = b.sample_clip(bi, bt).unwrap();
    close_pose(&ap, &bp);
    let av = a.deform(&ap).unwrap();
    let bv = b.deform(&bp).unwrap();
    assert_eq!(av.len(), bv.len());
    for (a, b) in av.iter().zip(&bv) {
        assert_eq!(a.indices, b.indices);
        assert_eq!(a.material, b.material);
        assert_eq!(a.vertices.len(), b.vertices.len());
        for (a, b) in a.vertices.iter().zip(&b.vertices) {
            close_array(a.position, b.position);
            close_array(a.normal, b.normal);
            assert_eq!(a.uv, b.uv);
        }
    }
}

#[test]
fn actual_demo_appends_one_clip_preserves_originals_and_cooks_reloads() {
    for clip_index in 0..2 {
        let before = DEMO.to_vec();
        let original = import(&before);
        let mut opts = options(0.25, 0.75);
        opts.clip_index = clip_index;
        let result = extract_glb(&before, &opts).unwrap();
        assert_eq!(before, DEMO, "extraction mutated the input");
        assert_eq!(
            extract_glb(&before, &opts).unwrap(),
            result,
            "nondeterministic output"
        );
        let extracted = import(&result);
        let old = original.source();
        let new = extracted.source();
        assert_eq!(new.clips.len(), old.clips.len() + 1);
        assert_eq!(&new.clips[..old.clips.len()], old.clips.as_slice());
        assert_eq!(new.clips[2].name, "extracted");
        close(new.clips[2].duration(), 0.5);
        assert_eq!(new.asset_id, old.asset_id);
        assert_eq!(new.nodes, old.nodes);
        assert_eq!(new.skins, old.skins);
        assert_eq!(new.primitives, old.primitives);
        assert_eq!(new.materials, old.materials);
        assert_eq!(new.images, old.images);
        let cooked = extracted.to_bytes().unwrap();
        let reloaded = AnimatedModel::from_bytes(&cooked).unwrap();
        assert_eq!(reloaded.to_bytes().unwrap(), cooked);
        for time in [0.0, 0.125, 0.25, 0.375, 0.5] {
            close_deformation(&reloaded, 2, time, &original, clip_index, time + opts.start);
        }
        for old_index in 0..2 {
            for time in [-1.0, 0.0, 0.5, 1.0, 2.0, 3.0] {
                close_deformation(&reloaded, old_index, time, &original, old_index, time);
            }
        }
    }
}

#[test]
fn semantic_json_indices_metadata_and_complete_binary_prefix_are_preserved() {
    let (mut doc, binary) = split_glb(FIXTURE);
    doc["extras"] = json!({"project":{"retained":true,"integer":123456789012345u64}});
    doc["buffers"][0]["name"] = json!("retain buffer name");
    doc["buffers"][0]["extras"] = json!([null, "buffer metadata"]);
    doc["nodes"][0]["extras"] = json!({"custom":{"nested":[1,2,3]}});
    doc["animations"][0]["extras"] = json!({"author":"fixture"});
    doc["animations"][0]["samplers"][0]["extras"] = json!({"tag":"retain sampler"});
    doc["accessors"][0]["extras"] = json!({"tag":"retain accessor"});
    let input = pack_glb(&doc, &binary);
    let output = extract_glb(&input, &options(0.25, 1.75)).unwrap();
    let (new, new_binary) = split_glb(&output);
    assert!(
        new_binary.starts_with(&binary),
        "original BIN bytes including padding changed"
    );
    assert!(new_binary.len() > binary.len());
    for (key, value) in doc.as_object().unwrap() {
        match key.as_str() {
            "animations" | "bufferViews" | "accessors" => {
                let old = value.as_array().unwrap();
                let new = new[key].as_array().unwrap();
                assert_eq!(
                    &new[..old.len()],
                    old.as_slice(),
                    "changed original {key} indices"
                );
            }
            "buffers" => {
                assert_eq!(new[key].as_array().unwrap().len(), 1);
                let mut old_buffer = value[0].clone();
                old_buffer["byteLength"] = new[key][0]["byteLength"].clone();
                assert_eq!(new[key][0], old_buffer);
            }
            _ => assert_eq!(&new[key], value, "changed non-animation {key}"),
        }
    }
    assert_eq!(new["animations"].as_array().unwrap().len(), 3);
    import(&output);
}

#[test]
fn linear_translation_scale_and_shortest_arc_rotation_have_analytic_values() {
    let mut f = Fixture::new();
    f.track(
        0,
        "translation",
        "LINEAR",
        &[0.0, 2.0, 4.0],
        &[0.0, 2.0, 4.0, 4.0, -2.0, 8.0, 0.0, 6.0, 12.0],
    );
    f.track(
        1,
        "scale",
        "LINEAR",
        &[0.0, 1.0, 4.0],
        &[1.0, 2.0, 3.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0],
    );
    let theta = std::f32::consts::PI / 3.0;
    // The negative representation forces shortest-arc hemisphere handling.
    f.track(
        2,
        "rotation",
        "LINEAR",
        &[0.0, 2.0],
        &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, -theta.sin(), -theta.cos()],
    );
    let output = extract_glb(&f.bytes(), &options(0.5, 3.0)).unwrap();
    let model = import(&output);
    let clip = &model.source().clips[1];
    assert_eq!(clip.channels.len(), 3);
    assert_eq!(clip.channels[0].times, [0.0, 1.5, 2.5]);
    assert_eq!(clip.channels[1].times, [0.0, 0.5, 2.5]);
    assert_eq!(clip.channels[2].times, [0.0, 1.5, 2.5]);
    for (local_time, translation, scale, half_angle) in [
        (0.0, [1.0, 1.0, 5.0], [2.0, 3.0, 4.0], theta * 0.25),
        (0.5, [2.0, 0.0, 6.0], [3.0, 4.0, 5.0], theta * 0.5),
        (1.5, [4.0, -2.0, 8.0], [4.0, 5.0, 6.0], theta),
        (2.5, [2.0, 2.0, 10.0], [5.0, 6.0, 7.0], theta),
    ] {
        let pose = model.sample_clip(1, local_time).unwrap();
        close_array(pose.local()[0].translation, translation);
        close_array(pose.local()[1].scale, scale);
        close_array(
            pose.local()[2].matrix()[0],
            [(2.0 * half_angle).cos(), (2.0 * half_angle).sin(), 0.0, 0.0],
        );
        assert_eq!(
            pose.local()[3],
            model.source().nodes[3].rest,
            "absent tracks changed rest"
        );
    }
}

#[test]
fn staggered_channels_single_keys_and_absent_tracks_keep_clamping_semantics() {
    let mut f = Fixture::new();
    f.track(
        0,
        "translation",
        "LINEAR",
        &[2.0, 4.0],
        &[2.0, 0.0, 0.0, 4.0, 0.0, 0.0],
    );
    f.track(1, "scale", "LINEAR", &[3.0], &[2.0, 3.0, 4.0]);
    f.track(
        2,
        "translation",
        "LINEAR",
        &[0.0, 6.0],
        &[0.0, 1.5, 0.0, 0.0, 7.5, 0.0],
    );
    let original = import(&f.bytes());
    let output = extract_glb(&f.bytes(), &options(1.0, 5.0)).unwrap();
    let model = import(&output);
    let channels = &model.source().clips[1].channels;
    assert_eq!(channels[0].times, [0.0, 1.0, 3.0, 4.0]);
    assert_eq!(channels[1].times, [0.0, 2.0, 4.0]);
    assert_eq!(channels[2].times, [0.0, 4.0]);
    for time in [0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5, 4.0] {
        let pose = model.sample_clip(1, time).unwrap();
        close_array(
            pose.local()[0].translation,
            [(time + 1.0).clamp(2.0, 4.0), 0.0, 0.0],
        );
        close_array(pose.local()[1].scale, [2.0, 3.0, 4.0]);
        close_array(pose.local()[2].translation, [0.0, time + 2.5, 0.0]);
        assert_eq!(pose.local()[3], model.source().nodes[3].rest);
        close_deformation(&model, 1, time, &original, 0, time + 1.0);
    }
    close_pose(
        &model.sample_clip(1, -10.0).unwrap(),
        &model.sample_clip(1, 0.0).unwrap(),
    );
    close_pose(
        &model.sample_clip(1, 10.0).unwrap(),
        &model.sample_clip(1, 4.0).unwrap(),
    );
}

#[test]
fn step_keys_jump_at_exact_start_interior_and_end_without_linear_ramps() {
    let mut f = Fixture::new();
    f.track(
        0,
        "translation",
        "STEP",
        &[0.0, 1.0, 2.0, 3.0],
        &[
            0.0, 0.0, 0.0, 10.0, 0.0, 0.0, 20.0, 0.0, 0.0, 30.0, 0.0, 0.0,
        ],
    );
    for (start, end, expected_times) in [
        (1.0, 3.0, vec![0.0, 1.0, 2.0]),
        (0.5, 2.0, vec![0.0, 0.5, 1.5]),
        (1.25, 1.75, vec![0.0, 0.5]),
    ] {
        let model = import(&extract_glb(&f.bytes(), &options(start, end)).unwrap());
        let channel = &model.source().clips[1].channels[0];
        assert_eq!(channel.interpolation, Interpolation::Step);
        assert_eq!(channel.times, expected_times);
        for time in [
            0.0, 0.0001, 0.4999, 0.5, 0.9999, 1.0, 1.4999, 1.5, 1.9999, 2.0,
        ] {
            let time = f32::min(time, end - start);
            let source_time = time + start;
            let expected = if source_time >= 3.0 {
                30.0
            } else if source_time >= 2.0 {
                20.0
            } else if source_time >= 1.0 {
                10.0
            } else {
                0.0
            };
            close_array(
                model.sample_clip(1, time).unwrap().local()[0].translation,
                [expected, 0.0, 0.0],
            );
        }
    }
}

#[test]
fn interior_values_and_exact_endpoint_keys_are_retained_once() {
    let mut f = Fixture::new();
    f.track(
        0,
        "translation",
        "LINEAR",
        &[0.0, 1.0, 2.0, 3.0],
        &[0.0, 0.0, 0.0, 7.0, 8.0, 9.0, -5.0, 2.0, 6.0, 4.0, 3.0, 2.0],
    );
    let model = import(&extract_glb(&f.bytes(), &options(1.0, 3.0)).unwrap());
    let channel = &model.source().clips[1].channels[0];
    assert_eq!(channel.times, [0.0, 1.0, 2.0]);
    assert_eq!(
        channel.values,
        ChannelValues::Translation(vec![[7.0, 8.0, 9.0], [-5.0, 2.0, 6.0], [4.0, 3.0, 2.0],])
    );
}

#[test]
fn near_parallel_nonunit_quaternions_use_runtime_normalization_and_interpolation() {
    let mut f = Fixture::new();
    let half_angle = 0.02f32;
    f.track(
        2,
        "rotation",
        "LINEAR",
        &[0.0, 2.0],
        &[
            0.0,
            0.0,
            0.0,
            1.002,
            0.0,
            0.0,
            -half_angle.sin() * 0.999,
            -half_angle.cos() * 0.999,
        ],
    );
    let original = import(&f.bytes());
    let model = import(&extract_glb(&f.bytes(), &options(0.25, 1.75)).unwrap());
    for time in [0.0, 0.375, 0.75, 1.125, 1.5] {
        close_deformation(&model, 1, time, &original, 0, time + 0.25);
        let q = model.sample_clip(1, time).unwrap().local()[2].rotation;
        close(q.iter().map(|v| v * v).sum(), 1.0);
    }
    let pose = model.sample_clip(1, 0.75).unwrap();
    close_array(
        pose.local()[2].rotation,
        [0.0, 0.0, (half_angle * 0.5).sin(), (half_angle * 0.5).cos()],
    );
}

#[test]
fn invalid_names_ranges_and_clip_indices_reject_without_mutation() {
    let bytes = FIXTURE.to_vec();
    let mut invalid = Vec::new();
    for name in [
        "",
        " \t ",
        "bend",
        "pulse",
        "line\nbreak",
        "nul\0name",
        "escape\u{1b}name",
    ] {
        let mut opts = options(0.0, 1.0);
        opts.name = name.into();
        invalid.push(opts);
    }
    for name in ["a".repeat(129), "é".repeat(65)] {
        let mut opts = options(0.0, 1.0);
        opts.name = name;
        invalid.push(opts);
    }
    for (start, end) in [
        (-0.1, 1.0),
        (0.0, 0.0),
        (1.0, 1.0),
        (1.0, 0.5),
        (0.0, 2.001),
        (f32::NAN, 1.0),
        (0.0, f32::NAN),
        (f32::NEG_INFINITY, 1.0),
        (f32::INFINITY, 1.0),
        (0.0, f32::INFINITY),
    ] {
        invalid.push(options(start, end));
    }
    for clip_index in [2, u32::MAX] {
        let mut opts = options(0.0, 1.0);
        opts.clip_index = clip_index;
        invalid.push(opts);
    }
    for opts in invalid {
        assert!(
            extract_glb(&bytes, &opts).is_err(),
            "accepted invalid extraction options"
        );
        assert_eq!(bytes, FIXTURE);
    }
    let mut opts = options(0.0, 2.0);
    opts.name = "é".repeat(64);
    assert!(
        extract_glb(&bytes, &opts).is_ok(),
        "128-byte name is in bounds"
    );
}

#[test]
fn zero_duration_clip_rejects_but_nonzero_single_key_clip_can_be_extracted() {
    for time in [0.0, 2.0] {
        let mut f = Fixture::new();
        f.track(0, "translation", "LINEAR", &[time], &[3.0, 4.0, 5.0]);
        let result = extract_glb(&f.bytes(), &options(0.0, 1.0));
        if time == 0.0 {
            assert!(result.is_err());
        } else {
            let model = import(&result.unwrap());
            assert_eq!(model.source().clips[1].channels[0].times, [0.0, 1.0]);
            for sample in [0.0, 0.5, 1.0] {
                close_array(
                    model.sample_clip(1, sample).unwrap().local()[0].translation,
                    [3.0, 4.0, 5.0],
                );
            }
        }
    }
}

#[test]
fn old_empty_or_duplicate_names_do_not_prevent_a_new_unique_name() {
    let (mut doc, bin) = split_glb(FIXTURE);
    for name in ["", "same old name"] {
        doc["animations"][0]["name"] = json!(name);
        doc["animations"][1]["name"] = json!(name);
        let model = import(&extract_glb(&pack_glb(&doc, &bin), &options(0.0, 1.0)).unwrap());
        assert_eq!(model.source().clips[0].name, name);
        assert_eq!(model.source().clips[1].name, name);
        assert_eq!(model.source().clips[2].name, "extracted");
    }
}

#[test]
fn external_data_uri_unsupported_and_unknown_sources_fail_closed() {
    let (doc, bin) = split_glb(FIXTURE);
    let mut invalid = Vec::new();
    for uri in [
        "outside.bin",
        "../outside.bin",
        "https://example.invalid/file.bin",
        "data:application/octet-stream;base64,AAAA",
    ] {
        let mut d = doc.clone();
        d["buffers"][0]["uri"] = json!(uri);
        invalid.push(d);
    }
    for uri in ["outside.png", "data:image/png;base64,AAAA"] {
        let mut d = doc.clone();
        d["images"][0].as_object_mut().unwrap().remove("bufferView");
        d["images"][0]["uri"] = json!(uri);
        invalid.push(d);
    }
    for pointer in [
        "",
        "/asset",
        "/nodes/0",
        "/animations/0",
        "/animations/0/samplers/0",
        "/animations/0/channels/0/target",
    ] {
        let mut d = doc.clone();
        d.pointer_mut(pointer).unwrap()["unknownSemanticField"] = json!(true);
        invalid.push(d);
    }
    let mut d = doc.clone();
    d["animations"][0]["samplers"][0]["interpolation"] = json!("CUBICSPLINE");
    invalid.push(d);
    let mut d = doc.clone();
    d["animations"][0]["channels"][0]["target"]["path"] = json!("weights");
    invalid.push(d);
    let mut d = doc.clone();
    d["extensionsUsed"] = json!(["KHR_animation_pointer"]);
    invalid.push(d);
    let mut d = doc.clone();
    d["accessors"][7]["sparse"] = json!({});
    invalid.push(d);
    let mut d = doc.clone();
    d["buffers"]
        .as_array_mut()
        .unwrap()
        .push(json!({"byteLength":4}));
    invalid.push(d);
    let mut d = doc.clone();
    d.as_object_mut().unwrap().remove("animations");
    invalid.push(d);
    for d in invalid {
        assert!(extract_glb(&pack_glb(&d, &bin), &options(0.0, 1.0)).is_err());
    }
    assert!(extract_glb(&serde_json::to_vec(&doc).unwrap(), &options(0.0, 1.0)).is_err());
}

#[test]
fn malformed_glb_headers_chunk_order_extra_chunks_and_truncation_reject() {
    let mut invalid = vec![
        vec![],
        b"glTF".to_vec(),
        FIXTURE[..FIXTURE.len() - 1].to_vec(),
    ];
    for (offset, word) in [(4, 1u32), (8, 12), (12, u32::MAX), (16, 0x004e4942)] {
        let mut bytes = FIXTURE.to_vec();
        bytes[offset..offset + 4].copy_from_slice(&word.to_le_bytes());
        invalid.push(bytes);
    }
    for kind in [0x004e4942u32, 0x4e4f534a, 0x12345678] {
        let mut bytes = FIXTURE.to_vec();
        bytes.extend_from_slice(&4u32.to_le_bytes());
        bytes.extend_from_slice(&kind.to_le_bytes());
        bytes.extend_from_slice(&[0; 4]);
        let len = bytes.len() as u32;
        bytes[8..12].copy_from_slice(&len.to_le_bytes());
        invalid.push(bytes);
    }
    let json_len = u32::from_le_bytes(FIXTURE[12..16].try_into().unwrap()) as usize;
    let mut only_json = FIXTURE[..20 + json_len].to_vec();
    let len = only_json.len() as u32;
    only_json[8..12].copy_from_slice(&len.to_le_bytes());
    invalid.push(only_json);
    for bytes in invalid {
        assert!(extract_glb(&bytes, &options(0.0, 1.0)).is_err());
    }
}

#[test]
fn source_validation_rejects_nonfinite_values_duplicate_times_and_invalid_targets() {
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let mut f = Fixture::new();
        f.track(
            0,
            "translation",
            "LINEAR",
            &[0.0, 1.0],
            &[0.0, 0.0, 0.0, bad, 0.0, 0.0],
        );
        assert!(extract_glb(&f.bytes(), &options(0.0, 1.0)).is_err());
    }
    for times in [&[0.0, 0.0][..], &[1.0, 0.0], &[-1.0, 1.0]] {
        let mut f = Fixture::new();
        f.track(0, "translation", "LINEAR", times, &[0.0; 6]);
        assert!(extract_glb(&f.bytes(), &options(0.0, 0.5)).is_err());
    }
    let mut f = Fixture::new();
    f.track(999, "translation", "LINEAR", &[0.0, 1.0], &[0.0; 6]);
    assert!(extract_glb(&f.bytes(), &options(0.0, 1.0)).is_err());
}

#[test]
fn appending_must_respect_clip_accessor_and_buffer_view_budgets() {
    let (doc, bin) = split_glb(FIXTURE);
    let mut full_clips = doc.clone();
    full_clips["animations"] = json!(vec![doc["animations"][0].clone(); MAX_CLIPS]);
    let bytes = pack_glb(&full_clips, &bin);
    assert_eq!(import(&bytes).source().clips.len(), MAX_CLIPS);
    assert!(extract_glb(&bytes, &options(0.0, 1.0)).is_err());
    for field in ["accessors", "bufferViews"] {
        let mut d = doc.clone();
        let values = d[field].as_array_mut().unwrap();
        let unused = values[0].clone();
        values.resize(1024, unused);
        let bytes = pack_glb(&d, &bin);
        import(&bytes);
        assert!(
            extract_glb(&bytes, &options(0.0, 1.0)).is_err(),
            "output exceeded {field} budget"
        );
    }
}

#[test]
fn appending_must_respect_aggregate_animation_key_budget() {
    let mut f = Fixture::new();
    let count = MAX_ANIMATION_KEYS / 4;
    let times: Vec<f32> = (0..count).map(|i| i as f32).collect();
    let values = vec![0.0; count * 3];
    f.track(0, "translation", "LINEAR", &times, &values);
    let clip = f.document["animations"][0].clone();
    f.document["animations"] = json!(vec![clip; 4]);
    let bytes = f.bytes();
    assert_eq!(import(&bytes).source().clips.len(), 4);
    assert!(extract_glb(&bytes, &options(0.0, times[count - 1])).is_err());
}

static TEMP_SERIAL: AtomicUsize = AtomicUsize::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let serial = TEMP_SERIAL.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("orr-clip-extract-{}-{serial}", std::process::id()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn cli_args(input: &Path, output: &Path) -> Vec<OsString> {
    [
        OsString::from("--input"),
        input.as_os_str().to_owned(),
        OsString::from("--output"),
        output.as_os_str().to_owned(),
        OsString::from("--clip"),
        OsString::from("0"),
        OsString::from("--start"),
        OsString::from("0.25"),
        OsString::from("--end"),
        OsString::from("1.75"),
        OsString::from("--name"),
        OsString::from("authored"),
    ]
    .into()
}

#[test]
fn cli_writes_importable_output_once_and_never_changes_source_or_existing_target() {
    let temp = TempDir::new();
    let input = temp.join("source.glb");
    let output = temp.join("clip.glb");
    fs::write(&input, FIXTURE).unwrap();
    authoring::run(cli_args(&input, &output)).unwrap();
    let generated = fs::read(&output).unwrap();
    assert_eq!(import(&generated).source().clips[2].name, "authored");
    assert_eq!(fs::read(&input).unwrap(), FIXTURE);
    assert!(authoring::run(cli_args(&input, &output)).is_err());
    assert_eq!(fs::read(&output).unwrap(), generated);
    assert!(authoring::run(cli_args(&input, &input)).is_err());
    assert_eq!(fs::read(&input).unwrap(), FIXTURE);
    let sentinel = temp.join("existing.glb");
    fs::write(&sentinel, b"keep existing unrelated file").unwrap();
    assert!(authoring::run(cli_args(&input, &sentinel)).is_err());
    assert_eq!(
        fs::read(&sentinel).unwrap(),
        b"keep existing unrelated file"
    );
}

#[test]
fn cli_rejects_bad_arguments_or_inputs_without_leaving_output() {
    let temp = TempDir::new();
    let input = temp.join("source.glb");
    let output = temp.join("clip.glb");
    fs::write(&input, FIXTURE).unwrap();
    let good = cli_args(&input, &output);
    let mut cases = vec![Vec::new(), good[..good.len() - 1].to_vec()];
    for index in (0..good.len()).step_by(2) {
        let mut args = good.clone();
        args.drain(index..index + 2);
        cases.push(args);
    }
    for (index, value) in [
        (0, "--unknown"),
        (5, "-1"),
        (5, "4294967296"),
        (7, "NaN"),
        (9, "inf"),
        (11, ""),
    ] {
        let mut args = good.clone();
        args[index] = value.into();
        cases.push(args);
    }
    let mut duplicate = good.clone();
    duplicate.extend([OsString::from("--name"), OsString::from("duplicate")]);
    cases.push(duplicate);
    for args in cases {
        assert!(authoring::run(args).is_err());
        assert!(!output.exists());
        assert_eq!(fs::read(&input).unwrap(), FIXTURE);
    }
    let mut shuffled: Vec<_> = good
        .chunks(2)
        .rev()
        .flat_map(|pair| pair.iter().cloned())
        .collect();
    shuffled[1] = OsString::from("shuffled");
    authoring::run(shuffled).unwrap();
    assert_eq!(
        import(&fs::read(&output).unwrap()).source().clips[2].name,
        "shuffled"
    );
    fs::remove_file(&output).unwrap();
    for malformed in [b"not GLB".as_slice(), &FIXTURE[..20]] {
        fs::write(&input, malformed).unwrap();
        assert!(authoring::run(cli_args(&input, &output)).is_err());
        assert!(!output.exists());
        assert_eq!(fs::read(&input).unwrap(), malformed);
    }
    assert!(authoring::run(cli_args(&temp.join("missing.glb"), &output)).is_err());
    assert!(!output.exists());
}

#[test]
fn cli_rejects_unsafe_output_names_directories_and_package_store_paths() {
    let temp = TempDir::new();
    let input = temp.join("source.glb");
    fs::write(&input, FIXTURE).unwrap();
    for name in ["clip.gltf", "clip", "clip name.glb", "한글.glb"] {
        let output = temp.join(name);
        assert!(authoring::run(cli_args(&input, &output)).is_err());
        assert!(!output.exists());
    }
    let long = temp.join(&format!("{}.glb", "x".repeat(97)));
    assert!(authoring::run(cli_args(&input, &long)).is_err());
    assert!(!long.exists());
    let directory = temp.join("directory.glb");
    fs::create_dir(&directory).unwrap();
    assert!(authoring::run(cli_args(&input, &directory)).is_err());
    assert!(directory.is_dir());
    let package = temp.join(".orr");
    fs::create_dir(&package).unwrap();
    let output = package.join("clip.glb");
    assert!(authoring::run(cli_args(&input, &output)).is_err());
    assert!(!output.exists());
    assert_eq!(fs::read(&input).unwrap(), FIXTURE);
}

#[test]
fn cli_rejects_oversized_sparse_input_before_reading_or_creating_output() {
    let temp = TempDir::new();
    let input = temp.join("oversized.glb");
    let output = temp.join("clip.glb");
    let file = fs::File::create(&input).unwrap();
    file.set_len(orr_model::MAX_FILE_BYTES as u64 + 1).unwrap();
    drop(file);
    assert!(authoring::run(cli_args(&input, &output)).is_err());
    assert!(!output.exists());
    assert_eq!(
        fs::metadata(&input).unwrap().len(),
        orr_model::MAX_FILE_BYTES as u64 + 1
    );
    assert_eq!(
        fs::read_dir(&temp.0).unwrap().count(),
        1,
        "failed extraction left staging files"
    );
}

#[cfg(unix)]
#[test]
fn cli_rejects_symlink_sources_targets_and_ancestors_including_dangling_targets() {
    use std::os::unix::fs::symlink;
    let temp = TempDir::new();
    let input = temp.join("source.glb");
    let output = temp.join("clip.glb");
    fs::write(&input, FIXTURE).unwrap();
    let linked_input = temp.join("linked-source.glb");
    symlink(&input, &linked_input).unwrap();
    assert!(authoring::run(cli_args(&linked_input, &output)).is_err());
    assert!(!output.exists());
    symlink(&input, &output).unwrap();
    assert!(authoring::run(cli_args(&input, &output)).is_err());
    assert_eq!(fs::read(&input).unwrap(), FIXTURE);
    fs::remove_file(&output).unwrap();
    let missing = temp.join("missing.glb");
    symlink(&missing, &output).unwrap();
    assert!(authoring::run(cli_args(&input, &output)).is_err());
    assert!(!missing.exists());
    assert!(fs::symlink_metadata(&output)
        .unwrap()
        .file_type()
        .is_symlink());
    fs::remove_file(&output).unwrap();
    let directory = temp.join("real");
    let linked_directory = temp.join("linked");
    fs::create_dir(&directory).unwrap();
    symlink(&directory, &linked_directory).unwrap();
    assert!(authoring::run(cli_args(&input, &linked_directory.join("out.glb"))).is_err());
    assert!(!directory.join("out.glb").exists());
    fs::write(directory.join("source.glb"), FIXTURE).unwrap();
    assert!(authoring::run(cli_args(&linked_directory.join("source.glb"), &output)).is_err());
    assert!(!output.exists());
}
