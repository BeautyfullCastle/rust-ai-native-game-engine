//! Original test-only variants of the workspace's original licensed fixtures.
#![allow(dead_code)]
use serde_json::{json, Value};

pub const WHITE_PNG: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0,
    0, 0, 31, 21, 196, 137, 0, 0, 0, 11, 73, 68, 65, 84, 120, 156, 99, 248, 15, 4, 0, 9, 251, 3,
    253, 251, 94, 107, 43, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];
pub fn original(animated: bool) -> (Value, Vec<u8>) {
    let bytes: &[u8] = if animated {
        include_bytes!("../fixtures/animated_strip.glb")
    } else {
        include_bytes!("../fixtures/model.glb")
    };
    let json_len = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    let source: Value = serde_json::from_slice(&bytes[20..20 + json_len]).unwrap();
    let offset = 20 + json_len;
    let bin_len = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
    let declared = source["buffers"][0]["byteLength"].as_u64().unwrap() as usize;
    assert!((declared..=declared + 3).contains(&bin_len));
    assert!(bytes[offset + 8 + declared..offset + 8 + bin_len]
        .iter()
        .all(|b| *b == 0));
    (source, bytes[offset + 8..offset + 8 + declared].to_vec())
}
pub fn factor(animated: bool, with_uv: bool) -> (Value, Vec<u8>) {
    let (mut source, buffer) = original(animated);
    source.as_object_mut().unwrap().remove("images");
    source.as_object_mut().unwrap().remove("textures");
    source.as_object_mut().unwrap().remove("samplers");
    for (index, material) in source["materials"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .enumerate()
    {
        let pbr = material["pbrMetallicRoughness"].as_object_mut().unwrap();
        pbr.remove("baseColorTexture");
        pbr.insert(
            "baseColorFactor".into(),
            if index == 0 {
                json!([0.25, 0.5, 0.75, 1.0])
            } else {
                json!([1.0, 0.25, 0.5, 1.0])
            },
        );
        pbr.insert("metallicFactor".into(), json!(0.0));
        pbr.remove("roughnessFactor"); // Exact glTF default1 remains admitted.
    }
    if !with_uv {
        for mesh in source["meshes"].as_array_mut().unwrap() {
            for primitive in mesh["primitives"].as_array_mut().unwrap() {
                primitive["attributes"]
                    .as_object_mut()
                    .unwrap()
                    .remove("TEXCOORD_0");
            }
        }
    }
    (source, buffer)
}
pub fn white_textured(source: &mut Value) {
    source["images"] = json!([{"uri":"white.png"}]);
    source["textures"] = json!([{"source":0}]);
    for material in source["materials"].as_array_mut().unwrap() {
        material["pbrMetallicRoughness"]["baseColorTexture"] = json!({"index":0});
    }
}
pub fn gltf(source: &Value) -> Vec<u8> {
    let mut source = source.clone();
    source["buffers"][0]["uri"] = json!("buffer.bin");
    serde_json::to_vec(&source).unwrap()
}
pub fn glb(source: &Value, buffer: &[u8]) -> Vec<u8> {
    let mut source = source.clone();
    source["buffers"][0].as_object_mut().unwrap().remove("uri");
    let mut json = serde_json::to_vec(&source).unwrap();
    while json.len() % 4 != 0 {
        json.push(b' ');
    }
    let mut bin = buffer.to_vec();
    while bin.len() % 4 != 0 {
        bin.push(0);
    }
    let mut out = Vec::new();
    for word in [
        0x46546c67_u32,
        2,
        (12 + 8 + json.len() + 8 + bin.len()) as u32,
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
pub fn resolve(uri: &str, buffer: &[u8]) -> Result<Vec<u8>, orr_model::Error> {
    match uri {
        "buffer.bin" => Ok(buffer.to_vec()),
        "white.png" => Ok(WHITE_PNG.to_vec()),
        _ => Err(orr_model::Error::message(format!(
            "unexpected fixture URI: {uri}"
        ))),
    }
}
pub fn uv_accessor(source: &Value) -> usize {
    source["meshes"][0]["primitives"][0]["attributes"]["TEXCOORD_0"]
        .as_u64()
        .unwrap() as usize
}
pub fn set_first_uv(source: &Value, buffer: &mut [u8], value: f32) {
    let a = &source["accessors"][uv_accessor(source)];
    let view = &source["bufferViews"][a["bufferView"].as_u64().unwrap() as usize];
    let offset = view["byteOffset"].as_u64().unwrap_or(0) as usize
        + a["byteOffset"].as_u64().unwrap_or(0) as usize;
    buffer[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
/// Isolated, owned test directory; never removes an existing path.
pub struct Scratch(pub std::path::PathBuf);
impl Scratch {
    pub fn new(label: &str) -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "orr-factor-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).expect("fresh private test directory");
        Self(root)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}
/// Same exact package bytes are used by bindings and renderer proofs.
pub fn package(destination: &std::path::Path, animated: bool) {
    std::fs::create_dir(destination).unwrap();
    let (v, b) = factor(animated, false);
    let (mut white, _) = factor(animated, true);
    white_textured(&mut white);
    for (name, data) in [
        ("factor.glb", glb(&v, &b)),
        ("white.glb", glb(&white, &b)),
        ("white.png", WHITE_PNG.to_vec()),
    ] {
        std::fs::write(destination.join(name), data).unwrap();
    }
    let value = json!({"schema":1,"name":"factor-fixture","version":"1.0.0","engine":"^0.0.1","capabilities":[if animated{"animation"}else{"models"}],"dependencies":{},"files":["factor.glb","white.glb","white.png"]});
    std::fs::write(
        destination.join("orr.package.json"),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();
}
