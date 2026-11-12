//! Pure test-fixture corner expansion shared by model and Editor consumers.
use serde_json::{json, Value};
pub fn original_parts(animated: bool) -> (Value, Vec<u8>) {
    let bytes: &[u8] = if animated {
        include_bytes!("../fixtures/animated_strip.glb")
    } else {
        include_bytes!("../fixtures/model.glb")
    };
    let json_len = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    let bin_header = 20 + json_len;
    let bin_len =
        u32::from_le_bytes(bytes[bin_header..bin_header + 4].try_into().unwrap()) as usize;
    (
        serde_json::from_slice(&bytes[20..20 + json_len]).unwrap(),
        bytes[bin_header + 8..bin_header + 8 + bin_len].to_vec(),
    )
}
fn width(accessor: &Value) -> usize {
    let component = match accessor["componentType"].as_u64().unwrap() {
        5121 => 1,
        5123 => 2,
        5125 | 5126 => 4,
        _ => panic!("fixture component"),
    };
    component
        * match accessor["type"].as_str().unwrap() {
            "SCALAR" => 1,
            "VEC2" => 2,
            "VEC3" => 3,
            "VEC4" => 4,
            _ => panic!("fixture attribute"),
        }
}
fn element<'a>(doc: &Value, bytes: &'a [u8], accessor: usize, index: usize) -> &'a [u8] {
    let a = &doc["accessors"][accessor];
    let view = &doc["bufferViews"][a["bufferView"].as_u64().unwrap() as usize];
    let size = width(a);
    let stride = view["byteStride"].as_u64().map_or(size, |n| n as usize);
    let start = view["byteOffset"].as_u64().unwrap_or(0) as usize
        + a["byteOffset"].as_u64().unwrap_or(0) as usize
        + stride * index;
    &bytes[start..start + size]
}
fn append(
    doc: &mut Value,
    bytes: &mut Vec<u8>,
    mut accessor: Value,
    data: &[u8],
    count: usize,
    target: u32,
) -> usize {
    while bytes.len() & 3 != 0 {
        bytes.push(0);
    }
    let view = doc["bufferViews"].as_array().unwrap().len();
    doc["bufferViews"]
        .as_array_mut()
        .unwrap()
        .push(json!({"buffer":0,"byteOffset":bytes.len(),"byteLength":data.len(),"target":target}));
    bytes.extend_from_slice(data);
    accessor["bufferView"] = json!(view);
    accessor["byteOffset"] = json!(0);
    accessor["count"] = json!(count);
    let index = doc["accessors"].as_array().unwrap().len();
    doc["accessors"].as_array_mut().unwrap().push(accessor);
    index
}
pub fn expanded_parts(animated: bool, count: Option<usize>) -> (Value, Vec<u8>) {
    let (original, input) = original_parts(animated);
    let mut bytes = input.clone();
    let mut doc = original.clone();
    for (p, primitive) in original["meshes"][0]["primitives"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let indices = primitive["indices"].as_u64().unwrap() as usize;
        let corners: Vec<usize> = (0..original["accessors"][indices]["count"].as_u64().unwrap()
            as usize)
            .map(|i| {
                u16::from_le_bytes(element(&original, &input, indices, i).try_into().unwrap())
                    as usize
            })
            .collect();
        let count = count.unwrap_or(corners.len());
        for (name, index) in primitive["attributes"].as_object().unwrap() {
            let index = index.as_u64().unwrap() as usize;
            let mut data = Vec::new();
            for i in 0..count {
                data.extend_from_slice(element(
                    &original,
                    &input,
                    index,
                    corners[i % corners.len()],
                ));
            }
            let new = append(
                &mut doc,
                &mut bytes,
                original["accessors"][index].clone(),
                &data,
                count,
                34962,
            );
            doc["meshes"][0]["primitives"][p]["attributes"][name] = json!(new);
        }
        let data: Vec<u8> = (0..count as u32).flat_map(u32::to_le_bytes).collect();
        let new = append(
            &mut doc,
            &mut bytes,
            json!({"componentType":5125,"type":"SCALAR"}),
            &data,
            count,
            34963,
        );
        doc["meshes"][0]["primitives"][p]["indices"] = json!(new);
    }
    (doc, bytes)
}

fn glb(animated: bool, omitted: bool) -> Vec<u8> {
    let (mut doc, mut bin) = expanded_parts(animated, None);
    if omitted {
        for p in doc["meshes"][0]["primitives"].as_array_mut().unwrap() {
            p.as_object_mut().unwrap().remove("indices");
        }
    }
    doc["buffers"][0].as_object_mut().unwrap().remove("uri");
    doc["buffers"][0]["byteLength"] = json!(bin.len());
    let mut json = serde_json::to_vec(&doc).unwrap();
    while json.len() % 4 != 0 {
        json.push(b' ');
    }
    while bin.len() % 4 != 0 {
        bin.push(0);
    }
    let mut out = Vec::new();
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&((28 + json.len() + bin.len()) as u32).to_le_bytes());
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend(json);
    out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend(bin);
    out
}
pub fn animated_glb(omit_indices: bool) -> Vec<u8> {
    glb(true, omit_indices)
}
pub fn static_glb(omit_indices: bool) -> Vec<u8> {
    glb(false, omit_indices)
}
