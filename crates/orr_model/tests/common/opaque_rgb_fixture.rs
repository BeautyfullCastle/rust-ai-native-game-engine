//! Independently encoded opaque twins; no production decoder supplies the oracle.
#![allow(dead_code)]
#[path = "nonindexed_fixture.rs"]
mod original;
use serde_json::{json, Value};

pub const RGBA: [u8; 16] = [
    255, 0, 17, 255, 0, 255, 33, 255, 5, 19, 255, 255, 129, 67, 3, 255,
];
pub fn png(rgb: bool) -> Vec<u8> {
    let data: Vec<u8> = if rgb {
        vec![255, 0, 17, 0, 255, 33, 5, 19, 255, 129, 67, 3]
    } else {
        RGBA.to_vec()
    };
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, 2, 2);
        encoder.set_color(if rgb {
            png::ColorType::Rgb
        } else {
            png::ColorType::Rgba
        });
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&data).unwrap();
        writer.finish().unwrap();
    }
    out
}
pub fn replace_images(doc: &mut Value, bin: &mut Vec<u8>, data: &[u8]) {
    while bin.len() & 3 != 0 {
        bin.push(0);
    }
    let view = doc["bufferViews"].as_array().unwrap().len();
    doc["bufferViews"]
        .as_array_mut()
        .unwrap()
        .push(json!({"buffer":0,"byteOffset":bin.len(),"byteLength":data.len()}));
    bin.extend_from_slice(data);
    for image in doc["images"].as_array_mut().unwrap() {
        *image = json!({"bufferView":view,"mimeType":"image/png"});
    }
}
pub fn parts(animated: bool, rgb: bool) -> (Value, Vec<u8>) {
    let (mut doc, mut bin) = original::original_parts(animated);
    replace_images(&mut doc, &mut bin, &png(rgb));
    (doc, bin)
}
pub fn pack(doc: &Value, bin: &[u8]) -> Vec<u8> {
    let mut doc = doc.clone();
    doc["buffers"][0].as_object_mut().unwrap().remove("uri");
    doc["buffers"][0]["byteLength"] = json!(bin.len());
    let mut json = serde_json::to_vec(&doc).unwrap();
    while json.len() & 3 != 0 {
        json.push(b' ');
    }
    let mut bin = bin.to_vec();
    while bin.len() & 3 != 0 {
        bin.push(0);
    }
    let mut out = Vec::new();
    for word in [
        0x46546c67_u32,
        2,
        (28 + json.len() + bin.len()) as u32,
        json.len() as u32,
        0x4e4f534a,
    ] {
        out.extend(word.to_le_bytes());
    }
    out.extend(json);
    out.extend((bin.len() as u32).to_le_bytes());
    out.extend(0x004e4942_u32.to_le_bytes());
    out.extend(bin);
    out
}
pub fn glb(animated: bool, rgb: bool) -> Vec<u8> {
    let (doc, bin) = parts(animated, rgb);
    pack(&doc, &bin)
}
