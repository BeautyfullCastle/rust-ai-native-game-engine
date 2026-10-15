//! GPU-free extracted-subtree -> installed cooked asset -> persistent binding proof.
//! The package contains only the cooked file; the source GLB is hidden before load.
#![allow(clippy::float_arithmetic)]

#[allow(dead_code)]
#[path = "../../orr_model/examples/static_model_extract/authoring.rs"]
mod authoring;

use orr_model::{StaticModel, Vertex};
use orr_model_bindings::model_bindings::{self, Binding, Document, LocalTransform, ModelKind};
use orr_package::{Project, Runtime};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

const LOGICAL_SOURCE: &str = "scene/source.glb";
const PACKAGE: &str = "static-extract-acceptance";
const ASSET: &str = "props/selected.model.json";
const ORIGINAL: &[u8] = include_bytes!("../../orr_model/tests/fixtures/model.glb");

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "orr-static-extract-bindings-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).expect("fresh owned test directory");
        Self(fs::canonicalize(root).unwrap())
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

// Test-authored scene topology, independent of the extractor. Original binary
// geometry/images remain byte-identical. Node 1 selects itself and child 4;
// sibling 3 and separate root 2 must be omitted. Node 5 is empty, 6 out of scene.
fn fixture() -> Vec<u8> {
    let json_len = u32::from_le_bytes(ORIGINAL[12..16].try_into().unwrap()) as usize;
    let mut source: Value = serde_json::from_slice(&ORIGINAL[20..20 + json_len]).unwrap();
    let binary = &ORIGINAL[28 + json_len..];
    source["scenes"] = json!([{"nodes":[0,2,5]}]);
    source["nodes"] = json!([
        {"name":"ancestor","translation":[-2,1,0],"scale":[0.5,0.5,1],"children":[1,3]},
        {"name":"selected mirrored prop","mesh":0,"translation":[0.5,0,0.1],"scale":[-1,1,1],"children":[4]},
        {"name":"omitted separate root","mesh":0,"translation":[2,-1,0.1],"scale":[0.5,0.5,1]},
        {"name":"omitted sibling","mesh":0,"translation":[8,0,0.1]},
        {"name":"selected descendant","mesh":0,"translation":[0,-4,0]},
        {"name":"empty root"},
        {"name":"outside default scene","mesh":0}
    ]);
    // Unused image/material entries must survive, with all original slots intact.
    source["images"]
        .as_array_mut()
        .unwrap()
        .push(json!({"bufferView":3,"mimeType":"image/png"}));
    source["textures"]
        .as_array_mut()
        .unwrap()
        .push(json!({"source":2,"sampler":0}));
    source["materials"].as_array_mut().unwrap().push(json!({"pbrMetallicRoughness":{
        "baseColorTexture":{"index":2},"baseColorFactor":[0.25,0.5,0.75,1],"metallicFactor":0,"roughnessFactor":1
    }}));
    let mut document = serde_json::to_vec(&source).unwrap();
    while document.len() % 4 != 0 {
        document.push(b' ');
    }
    let mut output = Vec::new();
    for word in [
        0x46546c67_u32,
        2,
        (28 + document.len() + binary.len()) as u32,
        document.len() as u32,
        0x4e4f534a,
    ] {
        output.extend(word.to_le_bytes());
    }
    output.extend(document);
    output.extend((binary.len() as u32).to_le_bytes());
    output.extend(0x004e4942_u32.to_le_bytes());
    output.extend(binary);
    output
}

fn assert_selected(model: &StaticModel, input: &[u8]) {
    let source = model.source();
    assert_eq!(source.asset_id, LOGICAL_SOURCE);
    assert_eq!(source.dependencies.len(), 1);
    assert_eq!(source.dependencies[0].uri, "$source");
    assert_eq!(source.dependencies[0].sha256, digest(input));
    assert_eq!(source.primitives.len(), 4);
    assert_eq!(source.materials.len(), 3);
    assert_eq!(source.images.len(), 3);
    let positions = [
        vec![
            [-1.0, -1.0, 0.0],
            [1.0, -1.0, 0.0],
            [1.0, 0.4, 0.0],
            [0.4, 1.0, 0.0],
            [-1.0, 1.0, 0.0],
        ],
        vec![[1.1, -0.75, 0.0], [1.8, -0.75, 0.0], [1.4, 0.4, 0.0]],
    ];
    let uvs = [
        vec![[0.0, 1.0], [1.0, 1.0], [1.0, 0.3], [0.7, 0.0], [0.0, 0.0]],
        vec![[0.25, 0.25]; 3],
    ];
    for (index, primitive) in source.primitives.iter().enumerate() {
        let slot = index % 2;
        let node = if index < 2 { 1 } else { 4 };
        assert_eq!(
            primitive.id,
            format!("{LOGICAL_SOURCE}#node={node}/mesh=0/primitive={slot}")
        );
        assert_eq!(primitive.material, slot as u32);
        let expected_vertices: Vec<Vertex> = positions[slot]
            .iter()
            .zip(&uvs[slot])
            .map(|(&position, &uv)| Vertex {
                position,
                normal: [0.0, 0.0, 1.0],
                uv,
            })
            .collect();
        assert_eq!(primitive.vertices, expected_vertices);
        assert_eq!(
            primitive.indices,
            if slot == 0 {
                vec![0, 1, 2, 0, 2, 3, 0, 3, 4]
            } else {
                vec![0, 1, 2]
            }
        );
        assert_eq!(
            primitive.transform,
            [
                [-0.5, 0.0, 0.0, 0.0],
                [0.0, 0.5, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [-1.75, if node == 1 { 1.0 } else { -1.0 }, 0.1, 1.0]
            ]
        );
    }
    // Independent importer comparison additionally checks complete immutable tables.
    let full = orr_model::import::import_with_resolver(LOGICAL_SOURCE, input, |_| {
        panic!("embedded fixture must never request external data")
    })
    .unwrap();
    assert_eq!(full.source().primitives.len(), 8);
    assert_eq!(source.materials, full.source().materials);
    assert_eq!(source.images, full.source().images);
}

#[test]
fn installed_extracted_static_subtree_binding_is_self_contained_and_tamper_evident() {
    let scratch = Scratch::new();
    let input = fixture();
    let source_path = scratch.0.join("original.glb");
    fs::write(&source_path, &input).unwrap();
    let nodes = authoring::list_glb_nodes(&input, LOGICAL_SOURCE).unwrap();
    let mut inventory: Vec<_> = nodes
        .iter()
        .map(|node| (node.index, node.primitive_count))
        .collect();
    inventory.sort_unstable();
    assert_eq!(inventory, [(0, 6), (1, 4), (2, 2), (3, 2), (4, 2)]);
    assert_eq!(
        nodes
            .iter()
            .find(|node| node.index == 1)
            .unwrap()
            .name
            .as_deref(),
        Some("selected mirrored prop")
    );
    for rejected in [5, 6, 99] {
        assert!(authoring::extract_glb_node(&input, LOGICAL_SOURCE, rejected).is_err());
    }
    let extracted = authoring::extract_glb_node(&input, LOGICAL_SOURCE, 1).unwrap();
    assert_selected(&extracted, &input);
    let cooked = extracted.to_bytes().unwrap();
    assert_eq!(
        StaticModel::from_bytes(&cooked)
            .unwrap()
            .to_bytes()
            .unwrap(),
        cooked
    );
    assert_ne!(digest(&cooked), digest(&input));

    let package_source = scratch.0.join("authored-package");
    fs::create_dir_all(package_source.join("props")).unwrap();
    fs::write(package_source.join(ASSET), &cooked).unwrap();
    fs::write(
        package_source.join("orr.package.json"),
        serde_json::to_vec(&json!({
            "schema":1,"name":PACKAGE,"version":"1.0.0","engine":"^0.0.1",
            "capabilities":["models"],"dependencies":{},"files":[ASSET]
        }))
        .unwrap(),
    )
    .unwrap();
    let root = scratch.0.join("project");
    fs::create_dir(&root).unwrap();
    let installer =
        Project::open_for_install(&root, Runtime::content_only().engine_version).unwrap();
    installer
        .install(std::slice::from_ref(&package_source))
        .unwrap();
    fs::rename(&package_source, scratch.0.join("hidden-package-source")).unwrap();
    fs::rename(&source_path, scratch.0.join("hidden-original.glb")).unwrap();
    assert!(!package_source.exists());
    assert!(!source_path.exists());
    assert!(!root.join(LOGICAL_SOURCE).exists());

    let project = model_bindings::open_project(&root).unwrap();
    let lock = project.verify().unwrap();
    let locked = &lock.packages[PACKAGE];
    assert_eq!(locked.files.len(), 1, "no GLB or external image installed");
    assert_eq!(locked.files[ASSET], digest(&cooked));
    let catalog = model_bindings::list_assets(&root).unwrap();
    assert_eq!(catalog.len(), 1);
    assert_eq!(catalog[0].package, PACKAGE);
    assert_eq!(catalog[0].asset, ASSET);
    assert_eq!(catalog[0].kind, ModelKind::Static);
    let loaded =
        model_bindings::load_asset_for_kind(&root, PACKAGE, ASSET, ModelKind::Static).unwrap();
    assert_eq!(loaded.source_hash(), digest(&cooked));
    assert_eq!(loaded.package_digest(), locked.digest);
    assert_selected(loaded.static_model().unwrap(), &input);
    let transform = LocalTransform {
        translation: [0.25, 0.5, -0.25],
        scale: [1.0, 2.0, 0.5],
        ..LocalTransform::default()
    };
    let binding = Binding::from_asset(PACKAGE.into(), ASSET.into(), &loaded, transform).unwrap();
    binding.validate(&loaded).unwrap();
    let document = Document {
        version: 2,
        scene: "scene.yaml".into(),
        project: ".".into(),
        bindings: BTreeMap::from([("e_00000001".into(), binding.clone())]),
    };
    document.validate().unwrap();
    let sidecar = root.join("scene.models.json");
    let saved = serde_json::to_vec_pretty(&document).unwrap();
    fs::write(&sidecar, &saved).unwrap();
    drop(loaded);
    drop(project);
    let reopened: Document = serde_json::from_slice(&fs::read(&sidecar).unwrap()).unwrap();
    reopened.validate().unwrap();
    assert_eq!(reopened, document);
    let rebound = model_bindings::load_binding(&root, &reopened.bindings["e_00000001"]).unwrap();
    assert_eq!(rebound.source_hash(), digest(&cooked));
    assert_eq!(rebound.package_digest(), locked.digest);
    assert_eq!(rebound.static_model().unwrap().to_bytes().unwrap(), cooked);
    assert_selected(rebound.static_model().unwrap(), &input);

    // Binding identity is independently enforced, even while installed bytes are valid.
    let lock_before = fs::read(root.join("orr.packages.lock.json")).unwrap();
    for change_source_hash in [false, true] {
        let mut stale = binding.clone();
        if change_source_hash {
            stale.source_hash = "0".repeat(64);
        } else {
            stale.package_digest = "0".repeat(64);
        }
        assert!(model_bindings::load_binding(&root, &stale).is_err());
    }
    // Valid cooked bytes with changed geometry still fail package digest verification.
    let installed_file = root
        .join(".orr/packages/objects")
        .join(&locked.digest)
        .join(ASSET);
    let mut changed = extracted.source().clone();
    changed.primitives[0].transform[3][0] += 0.25;
    let tampered = StaticModel::new(changed).unwrap().to_bytes().unwrap();
    assert_ne!(digest(&tampered), digest(&cooked));
    fs::write(&installed_file, tampered).unwrap();
    assert!(model_bindings::load_binding(&root, &binding).is_err());
    assert!(model_bindings::list_assets(&root).is_err());
    assert_eq!(fs::read(&sidecar).unwrap(), saved);
    assert_eq!(
        fs::read(root.join("orr.packages.lock.json")).unwrap(),
        lock_before
    );
    fs::write(&installed_file, &cooked).unwrap();
    let restored = model_bindings::load_binding(&root, &binding).unwrap();
    assert_eq!(restored.static_model().unwrap().to_bytes().unwrap(), cooked);
    assert_eq!(
        fs::read(scratch.0.join("hidden-original.glb")).unwrap(),
        input
    );
}
