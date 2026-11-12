//! Independent test encoder; normalization never calls a production decoder.
#![allow(dead_code, clippy::float_arithmetic)]
use serde_json::{json, Value};
#[path = "nonindexed_fixture.rs"]
mod corners;
#[derive(Clone, Copy, Debug)]
pub enum Encoding {
    U8,
    U16,
    FloatU8,
    FloatU16,
}
pub fn samples(encoding: Encoding) -> [f32; 6] {
    match encoding {
        Encoding::U8 | Encoding::FloatU8 => {
            [0_u16, 255, 1, 127, 254, 128].map(|n| (f64::from(n) / 255.0) as f32)
        }
        Encoding::U16 | Encoding::FloatU16 => {
            [0_u16, 65535, 1, 32767, 65534, 32768].map(|n| (f64::from(n) / 65535.0) as f32)
        }
    }
}
pub fn parts(animated: bool, encoding: Encoding) -> (Value, Vec<u8>) {
    parts_with_count(animated, encoding, None)
}
pub fn parts_with_count(
    animated: bool,
    encoding: Encoding,
    count: Option<usize>,
) -> (Value, Vec<u8>) {
    let (mut doc, mut bin) = corners::expanded_parts(animated, count);
    let primitive_count = doc["meshes"][0]["primitives"].as_array().unwrap().len();
    for p in 0..primitive_count {
        let old = doc["meshes"][0]["primitives"][p]["attributes"]["TEXCOORD_0"]
            .as_u64()
            .unwrap() as usize;
        let count = doc["accessors"][old]["count"].as_u64().unwrap() as usize;
        let (component, width, stride, remainder) = match encoding {
            Encoding::U8 => (5121, 2, 4, 1),
            Encoding::U16 => (5123, 4, 8, 2),
            _ => (5126, 8, 12, 0),
        };
        while bin.len() & 3 != remainder {
            bin.push(0);
        }
        let start = bin.len();
        bin.extend([0; 4]);
        for vertex in 0..count {
            for channel in 0..2 {
                let i = (vertex * 2 + channel) % 6;
                match encoding {
                    Encoding::U8 => bin.push([0, 255, 1, 127, 254, 128][i]),
                    Encoding::U16 => {
                        bin.extend([0_u16, 65535, 1, 32767, 65534, 32768][i].to_le_bytes())
                    }
                    _ => bin.extend(samples(encoding)[i].to_le_bytes()),
                }
            }
            if vertex + 1 < count {
                bin.extend(std::iter::repeat_n(0, stride - width));
            }
        }
        let view = doc["bufferViews"].as_array().unwrap().len();
        doc["bufferViews"].as_array_mut().unwrap().push(json!({"buffer":0,"byteOffset":start,"byteLength":bin.len()-start,"byteStride":stride,"target":34962}));
        let accessor = doc["accessors"].as_array().unwrap().len();
        let mut value = json!({"bufferView":view,"byteOffset":4,"componentType":component,"count":count,"type":"VEC2"});
        if matches!(encoding, Encoding::U8 | Encoding::U16) {
            value["normalized"] = json!(true);
        }
        doc["accessors"].as_array_mut().unwrap().push(value);
        doc["meshes"][0]["primitives"][p]["attributes"]["TEXCOORD_0"] = json!(accessor);
    }
    (doc, bin)
}
pub fn uv_accessor(doc: &Value) -> usize {
    doc["meshes"][0]["primitives"][0]["attributes"]["TEXCOORD_0"]
        .as_u64()
        .unwrap() as usize
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
pub fn glb(animated: bool, encoding: Encoding) -> Vec<u8> {
    let (doc, bin) = parts(animated, encoding);
    pack(&doc, &bin)
}
