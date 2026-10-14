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
use sha2::{Digest, Sha256};
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
        speed: 1.0,
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

const SPEEDS: [f32; 5] = [0.25, 0.5, 1.0, 2.0, 4.0];

fn speed_options(start: f32, end: f32, speed: f32) -> ExtractOptions {
    let mut opts = options(start, end);
    opts.speed = speed;
    opts
}

fn generated_accessor(document: &Value, binary: &[u8], index: usize) -> Vec<f32> {
    let accessor = &document["accessors"][index];
    assert_eq!(accessor["componentType"], 5126);
    let width = match accessor["type"].as_str().unwrap() {
        "SCALAR" => 1,
        "VEC3" => 3,
        "VEC4" => 4,
        kind => panic!("unexpected generated accessor type {kind}"),
    };
    let view = &document["bufferViews"][accessor["bufferView"].as_u64().unwrap() as usize];
    assert_eq!(view["buffer"], 0);
    let start = view["byteOffset"].as_u64().unwrap() as usize;
    let length = accessor["count"].as_u64().unwrap() as usize * width * 4;
    assert_eq!(view["byteLength"].as_u64().unwrap() as usize, length);
    binary[start..start + length]
        .chunks_exact(4)
        .map(|word| f32::from_le_bytes(word.try_into().unwrap()))
        .collect()
}

fn assert_generated_payload(output: &[u8], model: &AnimatedModel, clip_index: usize) {
    let (document, binary) = split_glb(output);
    let clip = &model.source().clips[clip_index];
    let generated = &document["animations"][clip_index];
    assert_eq!(
        generated["samplers"].as_array().unwrap().len(),
        clip.channels.len()
    );
    for (index, channel) in clip.channels.iter().enumerate() {
        assert_eq!(generated["channels"][index]["sampler"], json!(index));
        assert_eq!(generated["channels"][index]["target"]["node"], channel.node);
        let sampler = &generated["samplers"][index];
        let input = sampler["input"].as_u64().unwrap() as usize;
        let output = sampler["output"].as_u64().unwrap() as usize;
        let bits = |values: &[f32]| {
            values
                .iter()
                .map(|value| value.to_bits())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            bits(&generated_accessor(&document, &binary, input)),
            bits(&channel.times)
        );
        for (bound, expected) in [
            ("min", channel.times[0]),
            ("max", *channel.times.last().unwrap()),
        ] {
            let values = document["accessors"][input][bound].as_array().unwrap();
            assert_eq!(values.len(), 1, "scalar accessor requires one bound");
            let value = values[0].as_f64().unwrap();
            assert!(value.is_finite());
            // JSON numbers are parsed as f64, while componentType 5126 and
            // the BIN payload use f32. Decimal round trips may differ in f64
            // while retaining the exact same f32 value; require its exact bits.
            let typed = value as f32;
            assert!(typed.is_finite());
            assert_eq!(typed.to_bits(), expected.to_bits());
        }
        let (path, values): (&str, Vec<f32>) = match &channel.values {
            ChannelValues::Translation(values) => {
                ("translation", values.iter().flatten().copied().collect())
            }
            ChannelValues::Scale(values) => ("scale", values.iter().flatten().copied().collect()),
            ChannelValues::Rotation(values) => {
                ("rotation", values.iter().flatten().copied().collect())
            }
        };
        assert_eq!(generated["channels"][index]["target"]["path"], path);
        assert_eq!(
            bits(&generated_accessor(&document, &binary, output)),
            bits(&values)
        );
        assert_eq!(
            sampler["interpolation"],
            match channel.interpolation {
                Interpolation::Linear => "LINEAR",
                Interpolation::Step => "STEP",
            }
        );
    }
}

fn timing_fixture(times: &[f32]) -> Vec<u8> {
    let mut fixture = Fixture::new();
    let values: Vec<f32> = (0..times.len())
        .flat_map(|i| [i as f32, 0.0, 0.0])
        .collect();
    fixture.track(0, "translation", "STEP", times, &values);
    fixture.bytes()
}

#[test]
fn retimed_linear_tracks_have_analytic_values_and_exact_generated_keys_for_every_speed() {
    let mut fixture = Fixture::new();
    fixture.track(
        0,
        "translation",
        "LINEAR",
        &[0.0, 2.0, 4.0],
        &[0.0, 2.0, 4.0, 4.0, -2.0, 8.0, 0.0, 6.0, 12.0],
    );
    fixture.track(
        1,
        "scale",
        "LINEAR",
        &[0.0, 1.0, 4.0],
        &[1.0, 2.0, 3.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0],
    );
    let theta = std::f32::consts::PI / 3.0;
    let interior_rotation = [0.0, 0.0, -theta.sin(), -theta.cos()];
    fixture.track(
        2,
        "rotation",
        "LINEAR",
        &[0.0, 2.0],
        &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, -theta.sin(), -theta.cos()],
    );
    let bytes = fixture.bytes();
    let original = import(&bytes);
    for speed in SPEEDS {
        let opts = speed_options(0.5, 3.0, speed);
        let output = extract_glb(&bytes, &opts).unwrap();
        assert_eq!(extract_glb(&bytes, &opts).unwrap(), output);
        assert_eq!(bytes, fixture.bytes());
        let model = import(&output);
        let clip = &model.source().clips[1];
        assert_eq!(clip.duration(), 2.5 / speed);
        assert_eq!(clip.channels[0].times, [0.0, 1.5 / speed, 2.5 / speed]);
        assert_eq!(clip.channels[1].times, [0.0, 0.5 / speed, 2.5 / speed]);
        assert_eq!(clip.channels[2].times, [0.0, 1.5 / speed, 2.5 / speed]);
        assert_eq!(
            clip.channels[0].values,
            ChannelValues::Translation(vec![[1.0, 1.0, 5.0], [4.0, -2.0, 8.0], [2.0, 2.0, 10.0],])
        );
        assert_eq!(
            clip.channels[1].values,
            ChannelValues::Scale(vec![[2.0, 3.0, 4.0], [3.0, 4.0, 5.0], [5.0, 6.0, 7.0],])
        );
        let ChannelValues::Rotation(rotations) = &clip.channels[2].values else {
            panic!("rotation channel changed type");
        };
        assert_eq!(
            rotations[1].map(f32::to_bits),
            interior_rotation.map(f32::to_bits)
        );
        close_array(
            rotations[0],
            [0.0, 0.0, (theta * 0.25).sin(), (theta * 0.25).cos()],
        );
        close_array(rotations[2], interior_rotation);
        assert_generated_payload(&output, &model, 1);
        for (elapsed, translation, scale, half_angle) in [
            (0.0, [1.0, 1.0, 5.0], [2.0, 3.0, 4.0], theta * 0.25),
            (0.5, [2.0, 0.0, 6.0], [3.0, 4.0, 5.0], theta * 0.5),
            (1.0, [3.0, -1.0, 7.0], [3.5, 4.5, 5.5], theta * 0.75),
            (1.5, [4.0, -2.0, 8.0], [4.0, 5.0, 6.0], theta),
            (2.0, [3.0, 0.0, 9.0], [4.5, 5.5, 6.5], theta),
            (2.5, [2.0, 2.0, 10.0], [5.0, 6.0, 7.0], theta),
        ] {
            let time = elapsed / speed;
            let pose = model.sample_clip(1, time).unwrap();
            close_array(pose.local()[0].translation, translation);
            close_array(pose.local()[1].scale, scale);
            close_array(
                pose.local()[2].matrix()[0],
                [(2.0 * half_angle).cos(), (2.0 * half_angle).sin(), 0.0, 0.0],
            );
            assert_eq!(pose.local()[3], model.source().nodes[3].rest);
            close_deformation(&model, 1, time, &original, 0, elapsed + 0.5);
        }
    }
}

#[test]
fn retimed_step_tracks_preserve_jumps_clamping_single_keys_and_rest_for_every_speed() {
    let mut fixture = Fixture::new();
    fixture.track(
        0,
        "translation",
        "STEP",
        &[1.0, 2.0, 4.0],
        &[10.0, 0.0, 0.0, 20.0, 0.0, 0.0, 40.0, 0.0, 0.0],
    );
    fixture.track(1, "scale", "STEP", &[3.0], &[2.0, 3.0, 4.0]);
    fixture.track(
        2,
        "rotation",
        "STEP",
        &[0.0, 2.0, 4.0, 6.0],
        &[
            0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0, 1.0,
        ],
    );
    let bytes = fixture.bytes();
    let original = import(&bytes);
    for speed in SPEEDS {
        let output = extract_glb(&bytes, &speed_options(0.25, 5.0, speed)).unwrap();
        let model = import(&output);
        let clip = &model.source().clips[1];
        assert_eq!(clip.duration(), 4.75 / speed);
        assert_eq!(
            clip.channels[0].times,
            [0.0, 0.75 / speed, 1.75 / speed, 3.75 / speed, 4.75 / speed]
        );
        assert_eq!(clip.channels[1].times, [0.0, 2.75 / speed, 4.75 / speed]);
        assert_eq!(
            clip.channels[2].times,
            [0.0, 1.75 / speed, 3.75 / speed, 4.75 / speed]
        );
        assert_eq!(
            clip.channels[0].values,
            ChannelValues::Translation(vec![
                [10.0, 0.0, 0.0],
                [10.0, 0.0, 0.0],
                [20.0, 0.0, 0.0],
                [40.0, 0.0, 0.0],
                [40.0, 0.0, 0.0],
            ])
        );
        assert_eq!(
            clip.channels[1].values,
            ChannelValues::Scale(vec![[2.0, 3.0, 4.0]; 3])
        );
        assert_eq!(
            clip.channels[2].values,
            ChannelValues::Rotation(vec![
                [0.0, 0.0, 0.0, 1.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, -1.0],
                [0.0, 0.0, 0.0, -1.0],
            ])
        );
        assert!(clip
            .channels
            .iter()
            .all(|channel| channel.interpolation == Interpolation::Step));
        assert_generated_payload(&output, &model, 1);
        // The binary fractions straddle each jump without decimal-rounding ambiguity.
        let edge = 1.0 / 256.0;
        for source_time in [
            0.25,
            0.5,
            1.0 - edge,
            1.0,
            1.5,
            2.0 - edge,
            2.0,
            2.0 + edge,
            3.0,
            4.0 - edge,
            4.0,
            4.0 + edge,
            5.0,
        ] {
            let time = (source_time - 0.25) / speed;
            let pose = model.sample_clip(1, time).unwrap();
            let x = if source_time < 2.0 {
                10.0
            } else if source_time < 4.0 {
                20.0
            } else {
                40.0
            };
            assert_eq!(pose.local()[0].translation, [x, 0.0, 0.0]);
            assert_eq!(pose.local()[1].scale, [2.0, 3.0, 4.0]);
            assert_eq!(pose.local()[3], model.source().nodes[3].rest);
            close_deformation(&model, 1, time, &original, 0, source_time);
        }
        close_pose(
            &model.sample_clip(1, -1.0).unwrap(),
            &model.sample_clip(1, 0.0).unwrap(),
        );
        close_pose(
            &model.sample_clip(1, clip.duration() + 1.0).unwrap(),
            &model.sample_clip(1, clip.duration()).unwrap(),
        );
        // Exact source keys at both boundaries occur once, including the end jump.
        let exact_output = extract_glb(&bytes, &speed_options(1.0, 4.0, speed)).unwrap();
        let exact = import(&exact_output);
        let channel = &exact.source().clips[1].channels[0];
        assert_eq!(channel.times, [0.0, 1.0 / speed, 3.0 / speed]);
        assert_eq!(
            channel.values,
            ChannelValues::Translation(vec![[10.0, 0.0, 0.0], [20.0, 0.0, 0.0], [40.0, 0.0, 0.0],])
        );
        assert_eq!(
            exact.sample_clip(1, 3.0 / speed).unwrap().local()[0].translation,
            [40.0, 0.0, 0.0]
        );
        assert_generated_payload(&exact_output, &exact, 1);
        assert_eq!(bytes, fixture.bytes());
    }
}

#[test]
fn retimed_demo_preserves_original_content_and_cooks_reloads_for_every_speed() {
    let original = import(DEMO);
    for speed in SPEEDS {
        for clip_index in 0..2 {
            let mut opts = speed_options(0.25, 0.75, speed);
            opts.clip_index = clip_index;
            let bytes = DEMO.to_vec();
            let output = extract_glb(&bytes, &opts).unwrap();
            assert_eq!(bytes, DEMO);
            assert_eq!(extract_glb(&bytes, &opts).unwrap(), output);
            let model = import(&output);
            let old = original.source();
            let new = model.source();
            assert_eq!(new.clips.len(), old.clips.len() + 1);
            assert_eq!(&new.clips[..old.clips.len()], old.clips.as_slice());
            assert_eq!(new.clips[2].name, "extracted");
            assert_eq!(new.clips[2].duration(), 0.5 / speed);
            assert_eq!(new.asset_id, old.asset_id);
            assert_eq!(new.nodes, old.nodes);
            assert_eq!(new.skins, old.skins);
            assert_eq!(new.primitives, old.primitives);
            assert_eq!(new.materials, old.materials);
            assert_eq!(new.images, old.images);
            assert_generated_payload(&output, &model, 2);
            let cooked = model.to_bytes().unwrap();
            let reloaded = AnimatedModel::from_bytes(&cooked).unwrap();
            assert_eq!(reloaded.to_bytes().unwrap(), cooked);
            assert_eq!(reloaded.source(), model.source());
            for elapsed in [0.0, 0.0625, 0.125, 0.25, 0.375, 0.5] {
                close_deformation(
                    &reloaded,
                    2,
                    elapsed / speed,
                    &original,
                    clip_index,
                    elapsed + 0.25,
                );
            }
            for old_index in 0..2 {
                for time in [-1.0, 0.0, 0.5, 1.0, 2.0, 3.0] {
                    close_deformation(&reloaded, old_index, time, &original, old_index, time);
                }
            }
        }
    }
}

#[test]
fn retimed_glb_preserves_json_indices_metadata_and_complete_binary_prefix_for_every_speed() {
    let (mut document, binary) = split_glb(FIXTURE);
    document["extras"] = json!({"retimed":false,"id":123456789012345u64});
    document["buffers"][0]["name"] = json!("retained buffer");
    document["buffers"][0]["extras"] = json!([null, "original metadata"]);
    document["nodes"][0]["extras"] = json!({"retained":[1,2,3]});
    document["animations"][0]["extras"] = json!({"author":"original"});
    document["animations"][0]["samplers"][0]["extras"] = json!({"retained":true});
    document["accessors"][0]["extras"] = json!({"retained":true});
    let source = pack_glb(&document, &binary);
    for speed in SPEEDS {
        let output = extract_glb(&source, &speed_options(0.25, 1.75, speed)).unwrap();
        let (generated, generated_binary) = split_glb(&output);
        assert!(generated_binary.starts_with(&binary));
        assert!(generated_binary.len() > binary.len());
        for (key, value) in document.as_object().unwrap() {
            match key.as_str() {
                "animations" | "bufferViews" | "accessors" => {
                    let old = value.as_array().unwrap();
                    let new = generated[key].as_array().unwrap();
                    assert_eq!(
                        &new[..old.len()],
                        old.as_slice(),
                        "changed original {key} indices"
                    );
                }
                "buffers" => {
                    assert_eq!(generated[key].as_array().unwrap().len(), 1);
                    let mut expected = value[0].clone();
                    expected["byteLength"] = generated[key][0]["byteLength"].clone();
                    assert_eq!(generated[key][0], expected);
                }
                _ => assert_eq!(&generated[key], value, "changed original {key}"),
            }
        }
        assert_eq!(generated["animations"].as_array().unwrap().len(), 3);
        assert_eq!(source, pack_glb(&document, &binary));
        let model = import(&output);
        assert_eq!(model.source().clips[2].duration(), 1.5 / speed);
        assert_generated_payload(&output, &model, 2);
    }
}

#[test]
fn retiming_rejects_invalid_rates_without_mutating_source() {
    let bytes = FIXTURE.to_vec();
    for speed in [
        f32::NAN,
        f32::INFINITY,
        f32::NEG_INFINITY,
        -4.0,
        -1.0,
        -0.0,
        0.0,
        0.125,
        0.75,
        1.5,
        3.0,
        8.0,
        f32::from_bits(1),
        f32::MAX,
        f32::from_bits(1.0f32.to_bits() + 1),
    ] {
        assert!(
            extract_glb(&bytes, &speed_options(0.25, 1.75, speed)).is_err(),
            "accepted unsupported speed {speed:?}"
        );
        assert_eq!(bytes, FIXTURE);
    }
}

#[test]
fn retiming_rejects_overflow_and_duration_underflow_without_mutating_source() {
    for (end, speed) in [
        (f32::MAX, 0.25),
        (f32::MAX, 0.5),
        (f32::from_bits(1), 2.0),
        (f32::from_bits(1), 4.0),
        (f32::from_bits(2), 4.0),
    ] {
        let bytes = timing_fixture(&[0.0, end]);
        let before = bytes.clone();
        assert_eq!(import(&bytes).source().clips[0].duration(), end);
        assert!(
            extract_glb(&bytes, &speed_options(0.0, end, speed)).is_err(),
            "accepted duration overflow/underflow: {end:?} / {speed}"
        );
        assert_eq!(bytes, before);
    }
}

#[test]
fn retiming_rejects_precision_loss_and_collapsed_interior_keys() {
    // These durations round to a positive subnormal, so a positivity check is insufficient.
    for (end_bits, speed) in [(3, 2.0), (5, 4.0), (7, 4.0)] {
        let end = f32::from_bits(end_bits);
        assert!(end / speed > 0.0);
        let bytes = timing_fixture(&[0.0, end]);
        let before = bytes.clone();
        import(&bytes);
        assert!(extract_glb(&bytes, &speed_options(0.0, end, speed)).is_err());
        assert_eq!(bytes, before);
    }
    // Duration is exact in every case; only an interior key loses information or collapses.
    for (bits, speed) in [
        (vec![0, 1, 8], 2.0),
        (vec![0, 3, 8], 2.0),
        (vec![0, 4, 5, 8], 4.0),
        (vec![0, 7, 8], 4.0),
    ] {
        let times: Vec<f32> = bits.into_iter().map(f32::from_bits).collect();
        let end = *times.last().unwrap();
        assert_eq!(f64::from(end / speed), f64::from(end) / f64::from(speed));
        let bytes = timing_fixture(&times);
        let before = bytes.clone();
        import(&bytes);
        assert!(
            extract_glb(&bytes, &speed_options(0.0, end, speed)).is_err(),
            "accepted inexact or collapsed interior key at speed {speed}"
        );
        assert_eq!(bytes, before);
    }
}

#[test]
fn retiming_preserves_exact_subnormal_times_and_adjacent_interior_keys() {
    // Exactly representable subnormals are valid input and remain valid at every speed.
    let times = [0, 4, 8, 12, 16].map(f32::from_bits);
    let bytes = timing_fixture(&times);
    let original = import(&bytes);
    for speed in SPEEDS {
        let output = extract_glb(&bytes, &speed_options(times[1], times[4], speed)).unwrap();
        let model = import(&output);
        let channel = &model.source().clips[1].channels[0];
        let expected = [0, 4, 8, 12].map(|bits| f32::from_bits(bits) / speed);
        assert_eq!(
            channel
                .times
                .iter()
                .map(|time| time.to_bits())
                .collect::<Vec<_>>(),
            expected.map(f32::to_bits)
        );
        assert_eq!(
            channel.values,
            ChannelValues::Translation(vec![
                [1.0, 0.0, 0.0],
                [2.0, 0.0, 0.0],
                [3.0, 0.0, 0.0],
                [4.0, 0.0, 0.0],
            ])
        );
        assert_generated_payload(&output, &model, 1);
        let cooked = model.to_bytes().unwrap();
        let reloaded = AnimatedModel::from_bytes(&cooked).unwrap();
        assert_eq!(reloaded.to_bytes().unwrap(), cooked);
        for (time, source_time) in expected.into_iter().zip(&times[1..]) {
            close_deformation(&reloaded, 1, time, &original, 0, *source_time);
        }
    }
    let adjacent = [
        1.0,
        f32::from_bits(1.0f32.to_bits() + 1),
        f32::from_bits(1.0f32.to_bits() + 2),
    ];
    let bytes = timing_fixture(&adjacent);
    for speed in SPEEDS {
        let output = extract_glb(&bytes, &speed_options(adjacent[0], adjacent[2], speed)).unwrap();
        let model = import(&output);
        assert_eq!(
            model.source().clips[1].channels[0].times,
            [
                0.0,
                (adjacent[1] - 1.0) / speed,
                (adjacent[2] - 1.0) / speed
            ]
        );
        assert_eq!(
            model.source().clips[1].channels[0].values,
            ChannelValues::Translation(vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]])
        );
        assert_generated_payload(&output, &model, 1);
    }
    // A huge finite source is not rejected merely for its magnitude when the result is exact.
    let bytes = timing_fixture(&[0.0, f32::MAX]);
    let output = extract_glb(&bytes, &speed_options(0.0, f32::MAX, 4.0)).unwrap();
    let model = import(&output);
    assert_eq!(model.source().clips[1].duration(), f32::MAX / 4.0);
    assert_generated_payload(&output, &model, 1);
}

#[test]
fn cli_accepts_each_speed_in_every_option_position_and_preserves_source() {
    let temp = TempDir::new();
    let input = temp.join("source.glb");
    fs::write(&input, FIXTURE).unwrap();
    for (rate_index, (token, speed)) in ["0.25", "0.5", "1", "2", "4"]
        .into_iter()
        .zip(SPEEDS)
        .enumerate()
    {
        for pair_index in 0..=6 {
            let output = temp.join(&format!("speed-{rate_index}-position-{pair_index}.glb"));
            let mut args = cli_args(&input, &output);
            args.splice(
                pair_index * 2..pair_index * 2,
                [OsString::from("--speed"), OsString::from(token)],
            );
            authoring::run(args).unwrap();
            let generated = fs::read(&output).unwrap();
            let mut opts = speed_options(0.25, 1.75, speed);
            opts.name = "authored".into();
            assert_eq!(generated, extract_glb(FIXTURE, &opts).unwrap());
            let model = import(&generated);
            assert_eq!(model.source().clips[2].duration(), 1.5 / speed);
            assert_generated_payload(&generated, &model, 2);
            assert_eq!(fs::read(&input).unwrap(), FIXTURE);
        }
    }
    assert_eq!(
        fs::read_dir(&temp.0).unwrap().count(),
        36,
        "unexpected staging files"
    );
}

#[test]
fn cli_speed_one_matches_omitted_speed_and_prior_output_bytes() {
    let temp = TempDir::new();
    let input = temp.join("source.glb");
    let omitted = temp.join("omitted.glb");
    let explicit = temp.join("explicit.glb");
    fs::write(&input, DEMO).unwrap();
    let mut omitted_args = cli_args(&input, &omitted);
    omitted_args[9] = OsString::from("0.75");
    omitted_args[11] = OsString::from("short-action");
    authoring::run(omitted_args.clone()).unwrap();
    let mut explicit_args = omitted_args;
    explicit_args[3] = explicit.as_os_str().to_owned();
    explicit_args.extend([OsString::from("--speed"), OsString::from("1")]);
    authoring::run(explicit_args).unwrap();
    let generated = fs::read(&omitted).unwrap();
    assert_eq!(fs::read(&explicit).unwrap(), generated);
    // Independently retained pre-retiming CLI output: clip 0, 0.25..0.75, short-action.
    // This immutable digest predates the speed implementation; never regenerate it from the helper.
    let digest: String = Sha256::digest(&generated)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    assert_eq!(generated.len(), 4292);
    assert_eq!(
        digest,
        "fb9f3e53c02e69bd46798b7af2d92e712137fb9a091f92eecfefa83a01a9ac57"
    );
    assert_eq!(fs::read(&input).unwrap(), DEMO);
    assert_eq!(import(&generated).source().clips[2].duration(), 0.5);
    assert_eq!(fs::read_dir(&temp.0).unwrap().count(), 3);
}

#[test]
fn cli_rejects_invalid_speed_tokens_duplicate_odd_extra_and_missing_options() {
    let temp = TempDir::new();
    let input = temp.join("source.glb");
    let output = temp.join("clip.glb");
    fs::write(&input, FIXTURE).unwrap();
    let good = cli_args(&input, &output);
    let mut cases = Vec::new();
    for token in [
        "0", "-0", "-1", "0.125", "0.75", "1.5", "3", "8", "NaN", "nan", "inf", "-inf", "Infinity",
        "+1", "1.0", "01", "1e0", "2.0", "4.0", "0.50", ".5", "0.250", " 1", "1 ", "", "--name",
    ] {
        let mut args = good.clone();
        args.extend([OsString::from("--speed"), OsString::from(token)]);
        cases.push(args);
    }
    let mut with_speed = good.clone();
    with_speed.extend([OsString::from("--speed"), OsString::from("2")]);
    for index in (0..good.len()).step_by(2) {
        let mut missing = with_speed.clone();
        missing.drain(index..index + 2);
        cases.push(missing);
        let mut duplicate = with_speed.clone();
        duplicate.extend_from_slice(&good[index..index + 2]);
        cases.push(duplicate);
        let mut replaced = with_speed.clone();
        replaced[index] = OsString::from("--speed");
        replaced[index + 1] = OsString::from("2");
        cases.push(replaced);
    }
    for extra in [
        vec!["--speed"],
        vec!["--speed", "4"],
        vec!["--unknown", "value"],
        vec!["positional"],
        vec!["extra", "value"],
    ] {
        let mut args = with_speed.clone();
        args.extend(extra.into_iter().map(OsString::from));
        cases.push(args);
    }
    let mut dangling = good.clone();
    dangling.push(OsString::from("--speed"));
    cases.push(dangling);
    let mut absent_value = with_speed.clone();
    absent_value.remove(13);
    cases.push(absent_value);
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        let mut args = with_speed.clone();
        args[13] = OsString::from_vec(vec![0xff]);
        cases.push(args);
    }
    for args in cases {
        assert!(authoring::run(args).is_err());
        assert!(!output.exists());
        assert_eq!(fs::read(&input).unwrap(), FIXTURE);
        assert_eq!(
            fs::read_dir(&temp.0).unwrap().count(),
            1,
            "failed CLI left staging files"
        );
    }
}

#[test]
fn cli_retiming_failures_preserve_existing_targets_sources_and_leave_no_staging_files() {
    for (times, speed) in [
        (vec![0.0, f32::MAX], "0.25"),
        (vec![0.0, f32::from_bits(1)], "2"),
        (vec![0.0, f32::from_bits(3)], "2"),
        (vec![0.0, f32::from_bits(3), f32::from_bits(8)], "2"),
        (
            vec![0.0, f32::from_bits(4), f32::from_bits(5), f32::from_bits(8)],
            "4",
        ),
    ] {
        let temp = TempDir::new();
        let input = temp.join("source.glb");
        let output = temp.join("clip.glb");
        let existing = temp.join("existing.glb");
        let bytes = timing_fixture(&times);
        import(&bytes);
        fs::write(&input, &bytes).unwrap();
        fs::write(&existing, b"keep existing unrelated file").unwrap();
        let mut args = cli_args(&input, &output);
        args[7] = OsString::from("0");
        args[9] = OsString::from(format!("{:e}", times.last().unwrap()));
        args.extend([OsString::from("--speed"), OsString::from(speed)]);
        assert!(authoring::run(args.clone()).is_err());
        assert!(!output.exists());
        args[3] = existing.as_os_str().to_owned();
        assert!(authoring::run(args.clone()).is_err());
        args[3] = input.as_os_str().to_owned();
        assert!(authoring::run(args).is_err());
        assert_eq!(fs::read(&input).unwrap(), bytes);
        assert_eq!(
            fs::read(&existing).unwrap(),
            b"keep existing unrelated file"
        );
        assert_eq!(
            fs::read_dir(&temp.0).unwrap().count(),
            2,
            "failed retiming left staging files"
        );
    }
    // Successful retiming still never replaces a source, prior result, or unrelated target.
    for speed in ["0.25", "0.5", "1", "2", "4"] {
        let temp = TempDir::new();
        let input = temp.join("source.glb");
        let output = temp.join("clip.glb");
        let existing = temp.join("existing.glb");
        fs::write(&input, FIXTURE).unwrap();
        fs::write(&existing, b"keep existing unrelated file").unwrap();
        let mut args = cli_args(&input, &output);
        args.extend([OsString::from("--speed"), OsString::from(speed)]);
        authoring::run(args.clone()).unwrap();
        let generated = fs::read(&output).unwrap();
        assert!(authoring::run(args.clone()).is_err());
        args[3] = input.as_os_str().to_owned();
        assert!(authoring::run(args.clone()).is_err());
        args[3] = existing.as_os_str().to_owned();
        assert!(authoring::run(args).is_err());
        assert_eq!(fs::read(&input).unwrap(), FIXTURE);
        assert_eq!(fs::read(&output).unwrap(), generated);
        assert_eq!(
            fs::read(&existing).unwrap(),
            b"keep existing unrelated file"
        );
        assert_eq!(
            fs::read_dir(&temp.0).unwrap().count(),
            3,
            "publication left staging files"
        );
    }
}
