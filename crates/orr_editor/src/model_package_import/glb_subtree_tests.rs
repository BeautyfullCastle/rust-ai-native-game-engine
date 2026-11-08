use super::*;
use serde_json::{json, Value};

const GLB: &[u8] = include_bytes!("../../../orr_model/tests/fixtures/model.glb");

fn rewrite(edit: impl FnOnce(&mut Value)) -> Vec<u8> {
    let len = u32::from_le_bytes(GLB[12..16].try_into().unwrap()) as usize;
    let mut document: Value = serde_json::from_slice(&GLB[20..20 + len]).unwrap();
    edit(&mut document);
    let mut json = serde_json::to_vec(&document).unwrap();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    let tail = &GLB[20 + len..];
    let mut out = Vec::new();
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&((20 + json.len() + tail.len()) as u32).to_le_bytes());
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(&0x4e4f534au32.to_le_bytes());
    out.extend_from_slice(&json);
    out.extend_from_slice(tail);
    out
}

fn multi_object() -> Vec<u8> {
    rewrite(|d| {
        // Equal names cannot replace exact selection by source index.
        d["nodes"][1]["name"] = json!("prop");
        d["nodes"]
            .as_array_mut()
            .unwrap()
            .push(json!({"name":"prop","mesh":0,"translation":[7,0,0]}));
        d["nodes"]
            .as_array_mut()
            .unwrap()
            .push(json!({"name":"outside default","mesh":0}));
        d["scenes"][0]["nodes"] = json!([0, 2]);
        d["scenes"]
            .as_array_mut()
            .unwrap()
            .push(json!({"nodes":[3]}));
    })
}

fn variant() -> GlbSubtreeVariant {
    GlbSubtreeVariant {
        node_index: 1,
        asset_id: "props/selected.glb".into(),
    }
}

fn request(root: &Path, source: &[u8]) -> Request {
    let source_path = root.join("source.glb");
    fs::write(&source_path, source).unwrap();
    let license = root.join("LICENSE.txt");
    fs::write(&license, b"Redistribution license\n").unwrap();
    Request {
        source: source_path,
        license,
        destination: root.join("new-package"),
        name: "selected-prop".into(),
        version: "1.0.0".into(),
        kind: ModelKind::Static,
        source_format: SourceFormat::RawGlb,
        license_acknowledged: true,
    }
}

fn unopened() -> Request {
    Request {
        source: PathBuf::new(),
        license: PathBuf::new(),
        destination: PathBuf::new(),
        name: String::new(),
        version: String::new(),
        kind: ModelKind::Static,
        source_format: SourceFormat::RawGlb,
        license_acknowledged: true,
    }
}

#[test]
fn subtree_inspection_lists_only_default_scene_geometry_without_publication() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let bytes = multi_object();
    let req = request(&root, &bytes);
    let catalog = inspect_glb_subtree_source(&req.source).unwrap();
    assert_eq!(catalog.source_sha256, digest(&bytes));
    assert_eq!(
        catalog.nodes.iter().map(|n| n.index).collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
    assert_eq!(catalog.nodes[1].name, catalog.nodes[2].name);
    assert_eq!(
        catalog.nodes[1].primitive_count,
        catalog.nodes[2].primitive_count
    );
    assert!(!req.destination.exists());
    assert_eq!(fs::read(req.source).unwrap(), bytes);
}

#[test]
fn subtree_publication_preserves_selected_world_geometry_and_source_provenance() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let bytes = multi_object();
    let req = request(&root, &bytes);
    let source = req.source.clone();
    let license = req.license.clone();
    let full =
        orr_model::import::import_with_resolver("props/selected.glb", &bytes, |_| unreachable!())
            .unwrap();
    let selected: Vec<_> = full
        .source()
        .primitives
        .iter()
        .filter(|p| p.id.contains("#node=1/"))
        .cloned()
        .collect();
    assert!(!selected.is_empty());
    assert!(full.source().primitives.len() > selected.len());
    let report = import_glb_subtree_variant_checked(req, variant(), Some(digest(&bytes))).unwrap();
    assert_eq!(report.asset, "model.orrmodel");
    assert_eq!(report.kind, ModelKind::Static);
    let payload = fs::read(report.root.join(&report.asset)).unwrap();
    let cooked = orr_model::StaticModel::from_bytes(&payload).unwrap();
    assert_eq!(cooked.source().primitives, selected);
    assert_eq!(cooked.source().materials, full.source().materials);
    assert_eq!(cooked.source().images, full.source().images);
    assert_eq!(cooked.source().dependencies, full.source().dependencies);
    assert!(cooked
        .source()
        .dependencies
        .iter()
        .any(|d| d.uri == "$source" && d.sha256 == digest(&bytes)));
    assert_eq!(report.source_sha256, digest(&payload));
    assert_eq!(fs::read(&source).unwrap(), bytes);
    assert_eq!(fs::read(&license).unwrap(), b"Redistribution license\n");
    assert_eq!(
        fs::read(report.root.join("LICENSE.txt")).unwrap(),
        fs::read(license).unwrap()
    );
    let package =
        orr_package::inspect(&report.root, &orr_sample::room_project::compiled_runtime()).unwrap();
    assert!(package.manifest.files.contains("model.orrmodel"));
    assert!(!package.manifest.files.contains("model.glb"));
    assert!(!report.root.join("model.glb").exists());
}

#[test]
fn subtree_option_admission_precedes_filesystem_access() {
    for id in ["", "../bad.glb", "CON.glb", "bad.model.json", "x//bad.glb"] {
        let mut v = variant();
        v.asset_id = id.into();
        assert!(import_glb_subtree_variant(unopened(), v)
            .err()
            .unwrap()
            .contains("variant identity"));
    }
    let mut v = variant();
    v.node_index = u32::MAX;
    assert!(import_glb_subtree_variant(unopened(), v)
        .err()
        .unwrap()
        .contains("node index"));
    let mut req = unopened();
    req.kind = ModelKind::Animated;
    assert!(import_glb_subtree_variant(req, variant())
        .err()
        .unwrap()
        .contains("Static model mode"));
    let mut req = unopened();
    req.source_format = SourceFormat::CookedStatic;
    assert!(import_glb_subtree_variant(req, variant())
        .err()
        .unwrap()
        .contains("RawGlb"));
    let mut req = unopened();
    req.license_acknowledged = false;
    assert!(import_glb_subtree_variant(req, variant())
        .err()
        .unwrap()
        .contains("acknowledgement"));
    assert!(import_glb_subtree_variant_checked(unopened(), variant(), Some("bad".into())).is_err());
}

#[test]
fn subtree_stale_hash_rejects_before_license_read_or_publication() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let req = request(&root, &multi_object());
    let catalog = inspect_glb_subtree_source(&req.source).unwrap();
    fs::write(&req.source, GLB).unwrap();
    fs::write(&req.license, [0xff]).unwrap();
    let destination = req.destination.clone();
    let error = import_glb_subtree_variant_checked(req, variant(), Some(catalog.source_sha256))
        .err()
        .unwrap();
    assert!(error.contains("inspected GLB source changed"), "{error}");
    assert!(!destination.exists());
}

#[test]
fn subtree_invalid_selection_and_external_resources_publish_nothing() {
    for (bytes, index) in [
        (multi_object(), 3),
        (multi_object(), 999),
        (
            rewrite(|d| d["buffers"][0]["uri"] = json!("external.bin")),
            1,
        ),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let req = request(&root, &bytes);
        let destination = req.destination.clone();
        let mut v = variant();
        v.node_index = index;
        assert!(import_glb_subtree_variant(req, v).is_err());
        assert!(!destination.exists());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 2);
    }
}

#[test]
fn ordinary_raw_glb_import_still_publishes_original_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let bytes = multi_object();
    let report = import_package(request(&root, &bytes)).unwrap();
    assert_eq!(report.asset, "model.glb");
    assert_eq!(fs::read(report.root.join("model.glb")).unwrap(), bytes);
    assert!(!report.root.join("model.orrmodel").exists());
}

#[test]
fn subtree_existing_destination_and_invalid_license_are_never_published_or_replaced() {
    for invalid_license in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let req = request(&root, &multi_object());
        let destination = req.destination.clone();
        if invalid_license {
            fs::write(&req.license, [0xff]).unwrap();
        } else {
            fs::create_dir(&destination).unwrap();
            fs::write(destination.join("keep"), b"untouched").unwrap();
        }
        assert!(import_glb_subtree_variant(req, variant()).is_err());
        if invalid_license {
            assert!(!destination.exists());
        } else {
            assert_eq!(fs::read(destination.join("keep")).unwrap(), b"untouched");
        }
    }
}

#[cfg(unix)]
#[test]
fn subtree_inspector_and_import_reject_linked_sources() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let mut req = request(&root, &multi_object());
    let linked = root.join("linked.glb");
    std::os::unix::fs::symlink(&req.source, &linked).unwrap();
    assert!(inspect_glb_subtree_source(&linked).is_err());
    req.source = linked;
    let destination = req.destination.clone();
    assert!(import_glb_subtree_variant(req, variant()).is_err());
    assert!(!destination.exists());
}
