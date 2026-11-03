//! Independent literal matrix/TRS GLB twins. No production transform encoder.
#![allow(dead_code)]
use serde_json::{json, Value};

pub const PARENT_NODE: usize = 4;
pub const MATRIX: [f32; 16] = [
    -1.25, 0.0, 0.0, 0.0, 0.0, -0.75, 0.0, 0.0, 0.0, 0.0, 1.5, 0.0, 0.4, 3.0, 0.2, 1.0,
];

/// Decode the checked-in GLB without a base64 dependency (also used by Editor).
pub fn parts() -> (Value, Vec<u8>) {
    let bytes = include_bytes!("../fixtures/animated_strip.glb");
    assert_eq!(&bytes[..4], b"glTF");
    let word = |i| u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap()) as usize;
    assert_eq!(word(4), 2);
    assert_eq!(word(8), bytes.len());
    assert_eq!(word(16), 0x4e4f534a);
    let json_end = 20 + word(12);
    let doc = serde_json::from_slice(&bytes[20..json_end]).unwrap();
    assert_eq!(word(json_end + 4), 0x004e4942);
    assert_eq!(json_end + 8 + word(json_end), bytes.len());
    (doc, bytes[json_end + 8..].to_vec())
}

pub fn matrix_parts() -> (Value, Vec<u8>) {
    let (mut doc, bin) = parts();
    assert_eq!(doc["nodes"].as_array().unwrap().len(), PARENT_NODE);
    doc["nodes"].as_array_mut().unwrap().push(json!({
        "name":"untargeted matrix ancestor", "children":[0,3], "matrix":MATRIX
    }));
    doc["scenes"][0]["nodes"] = json!([PARENT_NODE]);
    (doc, bin)
}

pub fn trs_parts() -> (Value, Vec<u8>) {
    let (mut doc, bin) = matrix_parts();
    let node = &mut doc["nodes"][PARENT_NODE];
    node.as_object_mut().unwrap().remove("matrix");
    node["translation"] = json!([0.4, 3.0, 0.2]);
    node["rotation"] = json!([0.0, 0.0, 1.0, 0.0]);
    node["scale"] = json!([1.25, 0.75, 1.5]);
    (doc, bin)
}

pub fn pack(doc: &Value, bin: &[u8]) -> Vec<u8> {
    let mut json = serde_json::to_vec(doc).unwrap();
    while json.len() & 3 != 0 {
        json.push(b' ');
    }
    let mut bin = bin.to_vec();
    while bin.len() & 3 != 0 {
        bin.push(0);
    }
    let mut bytes = Vec::new();
    for word in [
        0x46546c67_u32,
        2,
        (28 + json.len() + bin.len()) as u32,
        json.len() as u32,
        0x4e4f534a,
    ] {
        bytes.extend(word.to_le_bytes());
    }
    bytes.extend(json);
    bytes.extend((bin.len() as u32).to_le_bytes());
    bytes.extend(0x004e4942_u32.to_le_bytes());
    bytes.extend(bin);
    bytes
}

/// (Matrix GLB, explicit-TRS GLB), with identical independent literal transforms.
pub fn twins() -> (Vec<u8>, Vec<u8>) {
    let (matrix, bin) = matrix_parts();
    let (trs, _) = trs_parts();
    (pack(&matrix, &bin), pack(&trs, &bin))
}

/// Valid embedded assets with one deliberately unsupported matrix-node edit.
pub fn rejected_cases() -> Vec<(&'static str, Vec<u8>)> {
    let mut cases = Vec::new();
    for label in ["targeted", "mirrored", "shear", "singular", "projective"] {
        let (mut doc, bin) = matrix_parts();
        match label {
            "targeted" => {
                doc["animations"][0]["channels"][0]["target"]["node"] = json!(PARENT_NODE)
            }
            "mirrored" => doc["nodes"][PARENT_NODE]["matrix"][0] = json!(1.25),
            "shear" => doc["nodes"][PARENT_NODE]["matrix"][4] = json!(0.01),
            "singular" => doc["nodes"][PARENT_NODE]["matrix"][0] = json!(0.0),
            "projective" => doc["nodes"][PARENT_NODE]["matrix"][3] = json!(0.01),
            _ => unreachable!(),
        }
        cases.push((label, pack(&doc, &bin)));
    }
    cases
}
