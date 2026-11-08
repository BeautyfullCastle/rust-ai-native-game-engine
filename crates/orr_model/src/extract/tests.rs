use super::*;
use serde_json::json;

const SOURCE_ID: &str = "props/subtree-source.glb";
const ORIGINAL: &[u8] = include_bytes!("../../tests/fixtures/model.glb");

fn fixture_parts() -> (Value, Vec<u8>) {
    let length = u32::from_le_bytes(ORIGINAL[12..16].try_into().unwrap()) as usize;
    let mut document: Value = serde_json::from_slice(&ORIGINAL[20..20 + length]).unwrap();
    document["nodes"] = json!([
        {"name":"assembly", "translation":[3,2,1], "children":[1,2,4]},
        {"name":"prop", "mesh":0,"translation":[1,0,0],"scale":[-2,1,1],"children":[3]},
        {"name":"sibling", "mesh":0,"translation":[-10,0,0]},
        {"name":"prop", "mesh":0,"translation":[0,4,0],"scale":[1,2,1]},
        {"name":"empty"}, {"name":"other scene", "mesh":0}
    ]);
    document["scenes"] = json!([{"nodes":[0]}, {"nodes":[5]}]);
    (document, ORIGINAL[28 + length..].to_vec())
}
fn pack(document: &Value, binary: &[u8]) -> Vec<u8> {
    let mut json = serde_json::to_vec(document).unwrap();
    while json.len() % 4 != 0 {
        json.push(b' ');
    }
    let mut output = b"glTF".to_vec();
    output.extend(2u32.to_le_bytes());
    output.extend(((28 + json.len() + binary.len()) as u32).to_le_bytes());
    output.extend((json.len() as u32).to_le_bytes());
    output.extend(b"JSON");
    output.extend(json);
    output.extend((binary.len() as u32).to_le_bytes());
    output.extend(b"BIN\0");
    output.extend(binary);
    output
}
fn reject(input: &[u8]) {
    assert!(list_glb_nodes(input, SOURCE_ID).is_err());
    assert!(extract_glb_node(input, SOURCE_ID, 1).is_err());
}

#[test]
fn public_byte_api_lists_exact_nodes_and_preserves_full_import_subtree_fields() {
    let (document, binary) = fixture_parts();
    let input = pack(&document, &binary);
    let original = crate::import::import_with_resolver(SOURCE_ID, &input, |_| {
        panic!("embedded fixture must not resolve resources")
    })
    .unwrap();
    let before = input.clone();
    let nodes = list_glb_nodes(&input, SOURCE_ID).unwrap();
    assert_eq!(
        nodes
            .iter()
            .map(|n| (n.index, n.primitive_count))
            .collect::<Vec<_>>(),
        [(0, 6), (1, 4), (2, 2), (3, 2)]
    );
    assert_eq!(nodes[1].name.as_deref(), Some("prop"));
    assert_eq!(nodes[1].name, nodes[3].name);
    let root = extract_glb_node(&input, SOURCE_ID, 0).unwrap();
    assert_eq!(root.to_bytes().unwrap(), original.to_bytes().unwrap());
    let selected = extract_glb_node(&input, SOURCE_ID, 3).unwrap();
    let mut expected = original.source().clone();
    expected
        .primitives
        .retain(|primitive| primitive.id.starts_with("props/subtree-source.glb#node=3/"));
    assert_eq!(expected.primitives.len(), 2);
    let placement = [
        [-2., 0., 0., 0.],
        [0., 2., 0., 0.],
        [0., 0., 1., 0.],
        [4., 6., 1., 1.],
    ];
    for primitive in &selected.source().primitives {
        assert_eq!(primitive.transform, placement);
    }
    // Canonical serialization checks geometry/UV/sampler/image tables and exact float bits.
    assert_eq!(
        selected.to_bytes().unwrap(),
        serde_json::to_vec(&expected).unwrap()
    );
    assert_eq!(
        StaticModel::from_bytes(&selected.to_bytes().unwrap())
            .unwrap()
            .to_bytes()
            .unwrap(),
        selected.to_bytes().unwrap()
    );
    assert_eq!(input, before);
}

#[test]
fn public_byte_api_uses_importer_admission_and_exact_default_scene_indices() {
    let (mut document, binary) = fixture_parts();
    let input = pack(&document, &binary);
    for index in [4, 5, 6, u32::MAX] {
        assert!(extract_glb_node(&input, SOURCE_ID, index).is_err());
    }
    document["scene"] = json!(1);
    let input = pack(&document, &binary);
    assert_eq!(
        list_glb_nodes(&input, SOURCE_ID)
            .unwrap()
            .iter()
            .map(|n| n.index)
            .collect::<Vec<_>>(),
        [5]
    );
    assert!(extract_glb_node(&input, SOURCE_ID, 1).is_err());
    assert_eq!(
        extract_glb_node(&input, SOURCE_ID, 5)
            .unwrap()
            .source()
            .primitives
            .len(),
        2
    );
    for mutate in 0..4 {
        let (mut document, binary) = fixture_parts();
        match mutate {
            0 => document["nodes"][2]["mesh"] = json!(900),
            1 => document["nodes"][2]["scale"] = json!([0, 1, 1]),
            2 => document["nodes"][2]["children"] = json!([0]),
            3 => document["animations"] = json!([]),
            _ => unreachable!(),
        }
        // A bad sibling is never hidden by selecting another valid subtree.
        reject(&pack(&document, &binary));
    }
    for id in [
        "",
        "../source.glb",
        "/source.glb",
        "source#part.glb",
        "x\\source.glb",
    ] {
        assert!(list_glb_nodes(&input, id).is_err());
        assert!(extract_glb_node(&input, id, 5).is_err());
    }
}

#[test]
fn public_byte_api_retains_strict_embedded_glb_envelope_and_resource_admission() {
    let (document, binary) = fixture_parts();
    let input = pack(&document, &binary);
    for length in 0..32 {
        reject(&input[..length]);
    }
    let mut bad = input.clone();
    bad[4..8].copy_from_slice(&1u32.to_le_bytes());
    reject(&bad);
    let mut bad = input.clone();
    bad.push(0);
    reject(&bad);
    let mut bad = input.clone();
    bad[12..16].copy_from_slice(&((MAX_JSON_BYTES + 4) as u32).to_le_bytes());
    reject(&bad);
    let mut bad = input.clone();
    bad.extend([0u8; 8]);
    let length = bad.len() as u32;
    bad[8..12].copy_from_slice(&length.to_le_bytes());
    reject(&bad);
    for uri in ["missing.bin", "data:application/octet-stream;base64,AAAA"] {
        let mut external = document.clone();
        external["buffers"][0]["uri"] = json!(uri);
        reject(&pack(&external, &binary));
        let mut external = document.clone();
        external["images"][0]["uri"] = json!(uri);
        reject(&pack(&external, &binary));
    }
}
