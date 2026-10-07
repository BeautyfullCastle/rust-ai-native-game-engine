#![cfg(feature = "models")]

use orr_editor::model_bindings::{self, Binding, Bindings, Document, LocalTransform, ModelKind};
use orr_package::{Project, Runtime};
use orr_reflect::Guid;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

const PACKAGE: &str = "sample-imported-scene";
const ASSET: &str = "foreground.glb";

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/imported_scene_demo")
        .canonicalize()
        .unwrap()
}

fn runtime() -> Runtime {
    let mut runtime = Runtime::content_only();
    runtime.capabilities.insert("models".into());
    runtime
}

fn install(source: &Path, root: &Path) {
    Project::open_for_install(root, Runtime::content_only().engine_version)
        .unwrap()
        .install(&[source.to_path_buf()])
        .unwrap();
}

fn copy_fixture(destination: &Path) -> PathBuf {
    std::fs::create_dir_all(destination).unwrap();
    for entry in std::fs::read_dir(fixture_dir()).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), destination.join(entry.file_name())).unwrap();
    }
    destination.to_path_buf()
}

fn add_declared_file(source: &Path, asset: &str, bytes: &[u8]) {
    let path = source.join(asset);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
    let manifest = source.join("orr.package.json");
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
    value["files"].as_array_mut().unwrap().push(asset.into());
    std::fs::write(manifest, serde_json::to_vec(&value).unwrap()).unwrap();
}

fn loaded(root: &Path) -> model_bindings::LoadedAsset {
    model_bindings::load_asset(root, PACKAGE, ASSET).unwrap()
}

fn binding(loaded: &model_bindings::LoadedAsset) -> Binding {
    Binding::from_asset(
        PACKAGE.into(),
        ASSET.into(),
        loaded,
        LocalTransform::default(),
    )
    .unwrap()
}

fn guid(n: u32) -> Guid {
    Guid::parse(&format!("e_{n:08x}")).unwrap()
}

fn bindings(root: &Path) -> Bindings {
    Bindings::create(
        root.join("scene.models.json"),
        "scene.yaml".into(),
        ".".into(),
    )
    .unwrap()
}

fn gltf_from_fixture() -> (serde_json::Value, Vec<u8>) {
    let bytes = std::fs::read(fixture_dir().join(ASSET)).unwrap();
    let json_len = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    let json: serde_json::Value = serde_json::from_slice(&bytes[20..20 + json_len]).unwrap();
    // External buffers exclude the optional GLB BIN chunk padding.
    let bin_len = json["buffers"][0]["byteLength"].as_u64().unwrap() as usize;
    (json, bytes[28 + json_len..28 + json_len + bin_len].to_vec())
}

#[test]
fn imports_verified_glb_cooks_reloads_and_lists_declared_model_candidates() {
    let temp = tempfile::tempdir().unwrap();
    install(&fixture_dir(), temp.path());
    let loaded = loaded(temp.path());
    assert_eq!(loaded.package(), PACKAGE);
    assert_eq!(loaded.asset(), ASSET);
    assert_eq!(loaded.package_digest().len(), 64);
    assert_eq!(loaded.source_hash().len(), 64);
    assert!(!loaded.model().source().primitives.is_empty());
    assert!(!loaded.model().source().images.is_empty());
    let cooked = loaded.model().to_bytes().unwrap();
    assert_eq!(
        orr_model::StaticModel::from_bytes(&cooked)
            .unwrap()
            .source(),
        loaded.model().source()
    );
    let cloned = loaded.clone();
    assert!(Arc::ptr_eq(cloned.model(), loaded.model()));
    let assets = model_bindings::list_assets(temp.path()).unwrap();
    assert_eq!(
        assets
            .iter()
            .map(|asset| asset.asset.as_str())
            .collect::<Vec<_>>(),
        ["background.glb", ASSET]
    );
    assert!(assets
        .iter()
        .all(|asset| asset.package == PACKAGE && asset.kind == ModelKind::Static));
}

#[test]
fn installed_cooked_static_model_loads_without_a_clip_or_animation_capability() {
    let temp = tempfile::tempdir().unwrap();
    let source = copy_fixture(&temp.path().join("source"));
    let imported = orr_model::import::import_path(&source, ASSET).unwrap();
    add_declared_file(&source, "cooked.json", &imported.to_bytes().unwrap());
    add_declared_file(&source, "settings.json", br#"{"unrelated":true}"#);
    let root = temp.path().join("project");
    std::fs::create_dir(&root).unwrap();
    install(&source, &root);
    let loaded = model_bindings::load_asset(&root, PACKAGE, "cooked.json").unwrap();
    assert_eq!(loaded.model().source(), imported.source());
    let binding = Binding::from_asset(
        PACKAGE.into(),
        "cooked.json".into(),
        &loaded,
        LocalTransform::default(),
    )
    .unwrap();
    let serialized = serde_json::to_value(binding).unwrap();
    assert_eq!(serialized["kind"], "static");
    assert!(serialized.get("clip_index").is_none());
    assert!(serialized.get("playback").is_none());
    let assets = model_bindings::list_assets(&root).unwrap();
    assert!(assets.iter().any(|asset| asset.asset == "cooked.json"));
    assert!(!assets.iter().any(|asset| asset.asset == "settings.json"));
}

#[test]
fn declared_external_gltf_resource_import_is_package_scoped() {
    let temp = tempfile::tempdir().unwrap();
    let source = copy_fixture(&temp.path().join("source"));
    let (mut json, bin) = gltf_from_fixture();
    json["buffers"][0]["uri"] = "mesh.bin".into();
    add_declared_file(
        &source,
        "nested/scene.gltf",
        &serde_json::to_vec(&json).unwrap(),
    );
    add_declared_file(&source, "nested/mesh.bin", &bin);
    let root = temp.path().join("project");
    std::fs::create_dir(&root).unwrap();
    install(&source, &root);
    let loaded = model_bindings::load_asset(&root, PACKAGE, "nested/scene.gltf").unwrap();
    assert!(loaded
        .model()
        .source()
        .dependencies
        .iter()
        .any(|dep| dep.uri == "mesh.bin"));
}

#[test]
fn external_uri_escape_and_undeclared_resources_cannot_replace_a_valid_binding() {
    let temp = tempfile::tempdir().unwrap();
    let source = copy_fixture(&temp.path().join("source"));
    let (mut json, bin) = gltf_from_fixture();
    json["buffers"][0]["uri"] = "../outside.bin".into();
    add_declared_file(&source, "escape.gltf", &serde_json::to_vec(&json).unwrap());
    json["buffers"][0]["uri"] = "missing.bin".into();
    add_declared_file(&source, "missing.gltf", &serde_json::to_vec(&json).unwrap());
    // Neither an adjacent file nor undeclared source data is asset authority.
    std::fs::write(source.join("missing.bin"), &bin).unwrap();
    let root = temp.path().join("project");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(temp.path().join("outside.bin"), &bin).unwrap();
    std::fs::write(root.join("missing.bin"), &bin).unwrap();
    install(&source, &root);
    let loaded = loaded(&root);
    let mut bindings = bindings(&root);
    bindings
        .assign_validated(&[guid(1)], &binding(&loaded), &loaded)
        .unwrap();
    let before = bindings.document().clone();
    assert!(model_bindings::load_asset(&root, PACKAGE, "escape.gltf").is_err());
    assert!(model_bindings::load_asset(&root, PACKAGE, "missing.gltf").is_err());
    assert_eq!(bindings.document(), &before);
}

#[test]
fn assignment_is_guid_keyed_transactional_and_reopens_without_transient_history() {
    let temp = tempfile::tempdir().unwrap();
    install(&fixture_dir(), temp.path());
    let loaded = loaded(temp.path());
    let mut bindings = bindings(temp.path());
    let ids = [guid(1), guid(2)];
    let mut binding = binding(&loaded);
    binding.transform.translation = [1.0, -2.0, 3.0];
    binding.transform.scale = [2.0, 1.0, 0.5];
    binding.transform.rotation = [0.0, 1.0, 0.0, 0.0];
    assert!(bindings.dirty());
    let prepared = bindings
        .prepare_assignment(&ids, &binding, &loaded)
        .unwrap();
    assert!(bindings.document().bindings.is_empty());
    bindings.commit_assignment(prepared).unwrap();
    assert_eq!(bindings.document().bindings.len(), 2);
    bindings.undo();
    assert!(bindings.document().bindings.is_empty());
    bindings.redo();
    assert_eq!(bindings.document().bindings[&guid(1).to_string()], binding);
    bindings.save().unwrap();
    assert!(!bindings.dirty());
    let bytes = std::fs::read(&bindings.path).unwrap();
    let text = String::from_utf8(bytes).unwrap();
    assert!(text.contains("e_00000001"));
    assert!(!text.contains("handle"));
    assert!(!text.contains("clip"));
    let mut reopened = Bindings::open(bindings.path.clone()).unwrap();
    assert_eq!(reopened.document(), bindings.document());
    assert!(!reopened.dirty());
    reopened.undo();
    assert_eq!(reopened.document(), bindings.document());
    std::fs::write(temp.path().join("scene.yaml"), b"scene").unwrap();
    assert!(reopened.matches_scene(&temp.path().join("scene.yaml").display().to_string()));
    assert!(!reopened.matches_scene(&temp.path().join("other.yaml").display().to_string()));
    assert_eq!(
        reopened.project_root().unwrap(),
        temp.path().canonicalize().unwrap()
    );
    assert!(!reopened
        .document()
        .bindings
        .contains_key(&guid(3).to_string()));
}

#[test]
fn stale_or_foreign_prepared_assignments_cannot_overwrite_newer_edits() {
    let temp = tempfile::tempdir().unwrap();
    install(&fixture_dir(), temp.path());
    let loaded = loaded(temp.path());
    let binding = binding(&loaded);
    let mut bindings = bindings(temp.path());
    let prepared = bindings
        .prepare_assignment(&[guid(1)], &binding, &loaded)
        .unwrap();
    bindings
        .assign_validated(&[guid(2)], &binding, &loaded)
        .unwrap();
    let before = bindings.document().clone();
    assert!(bindings
        .commit_assignment(prepared)
        .unwrap_err()
        .contains("changed after preparation"));
    assert_eq!(bindings.document(), &before);
    let prepared = bindings
        .prepare_assignment(&[guid(1)], &binding, &loaded)
        .unwrap();
    // Even if undo/redo returns to equal content, preparation was invalidated.
    bindings.undo();
    bindings.redo();
    assert!(bindings.commit_assignment(prepared).is_err());
    assert_eq!(bindings.document(), &before);
    let prepared = bindings
        .prepare_assignment(&[guid(1)], &binding, &loaded)
        .unwrap();
    let mut other = Bindings::create(
        temp.path().join("other.models.json"),
        "scene.yaml".into(),
        ".".into(),
    )
    .unwrap();
    assert!(other.commit_assignment(prepared).is_err());
    assert!(other.document().bindings.is_empty());
}

#[test]
fn invalid_identity_path_hash_and_transform_preserve_valid_assignment_and_history() {
    let temp = tempfile::tempdir().unwrap();
    install(&fixture_dir(), temp.path());
    let loaded = loaded(temp.path());
    let valid = binding(&loaded);
    let mut bindings = bindings(temp.path());
    bindings
        .assign_validated(&[guid(1)], &valid, &loaded)
        .unwrap();
    bindings.save().unwrap();
    let before = bindings.document().clone();
    assert!(Binding::from_asset(
        "wrong-package".into(),
        ASSET.into(),
        &loaded,
        LocalTransform::default()
    )
    .is_err());
    assert!(bindings.assign_validated(&[], &valid, &loaded).is_err());
    let mut variants = Vec::new();
    for path in [
        "../model.glb",
        "/model.glb",
        "C:/model.glb",
        "model\\bad.glb",
        "a//b.glb",
        "CON.glb",
    ] {
        let mut invalid = valid.clone();
        invalid.asset = path.into();
        variants.push(invalid);
    }
    let mut invalid = valid.clone();
    invalid.source_hash = "0".repeat(64);
    variants.push(invalid);
    let mut invalid = valid.clone();
    invalid.package_digest = "0".repeat(64);
    variants.push(invalid);
    for transform in invalid_transforms() {
        let mut invalid = valid.clone();
        invalid.transform = transform;
        variants.push(invalid);
    }
    for invalid in variants {
        assert!(bindings
            .assign_validated(&[guid(1), guid(2)], &invalid, &loaded)
            .is_err());
        assert_eq!(bindings.document(), &before);
        assert!(!bindings.dirty());
    }
    bindings.undo();
    assert!(bindings.document().bindings.is_empty());
    bindings.redo();
    assert_eq!(bindings.document(), &before);
}

fn invalid_transforms() -> Vec<LocalTransform> {
    let mut transforms = Vec::new();
    for x in [f32::NAN, f32::INFINITY, -f32::INFINITY, 1_000_001.0] {
        transforms.push(LocalTransform {
            translation: [x, 0.0, 0.0],
            ..Default::default()
        });
    }
    for x in [0.0, -1.0, 0.0001, 1001.0, f32::NAN, f32::INFINITY] {
        transforms.push(LocalTransform {
            scale: [x, 1.0, 1.0],
            ..Default::default()
        });
    }
    for rotation in [
        [0.0; 4],
        [0.0, 0.0, 0.0, 2.0],
        [f32::NAN, 0.0, 0.0, 1.0],
        [f32::INFINITY; 4],
    ] {
        transforms.push(LocalTransform {
            rotation,
            ..Default::default()
        });
    }
    transforms
}

#[test]
fn static_import_rejects_animation_declarations_without_animation_feature() {
    let temp = tempfile::tempdir().unwrap();
    let source = copy_fixture(&temp.path().join("source"));
    let (mut json, _) = gltf_from_fixture();
    json["animations"] = serde_json::json!([]);
    add_declared_file(
        &source,
        "animated.gltf",
        &serde_json::to_vec(&json).unwrap(),
    );
    let root = temp.path().join("project");
    std::fs::create_dir(&root).unwrap();
    install(&source, &root);
    let error = model_bindings::load_asset(&root, PACKAGE, "animated.gltf").unwrap_err();
    assert!(
        error.contains("static importer does not support skins/animations"),
        "{error}"
    );
}

#[test]
fn remove_reinstall_and_changed_package_report_staleness_without_retargeting() {
    let temp = tempfile::tempdir().unwrap();
    let source = copy_fixture(&temp.path().join("source"));
    let root = temp.path().join("project");
    std::fs::create_dir(&root).unwrap();
    install(&source, &root);
    let original = loaded(&root);
    let binding = binding(&original);
    let mut bindings = bindings(&root);
    bindings
        .assign_validated(&[guid(1)], &binding, &original)
        .unwrap();
    bindings.save().unwrap();
    let before = bindings.document().clone();
    Project::open(&root, runtime())
        .unwrap()
        .remove(PACKAGE)
        .unwrap();
    assert!(model_bindings::load_binding(&root, &binding)
        .unwrap_err()
        .contains("package was removed"));
    install(&source, &root);
    let same = model_bindings::load_binding(&root, &binding).unwrap();
    assert_eq!(same.package_digest(), original.package_digest());
    std::fs::write(source.join("LICENSE.txt"), b"changed declared content\n").unwrap();
    Project::open(&root, runtime())
        .unwrap()
        .remove(PACKAGE)
        .unwrap();
    install(&source, &root);
    let changed = loaded(&root);
    assert_eq!(changed.source_hash(), original.source_hash());
    assert_ne!(changed.package_digest(), original.package_digest());
    assert!(model_bindings::load_binding(&root, &binding)
        .unwrap_err()
        .contains("package changed"));
    // Also identify changed package content before importing malformed replacement bytes.
    std::fs::write(source.join(ASSET), b"not a model").unwrap();
    Project::open(&root, runtime())
        .unwrap()
        .remove(PACKAGE)
        .unwrap();
    install(&source, &root);
    assert!(model_bindings::load_binding(&root, &binding)
        .unwrap_err()
        .contains("package changed"));
    assert_eq!(bindings.document(), &before);
    assert!(!bindings.dirty());
    assert!(!original.model().source().primitives.is_empty());
}

#[test]
fn installed_tampering_is_rejected_by_catalog_and_load() {
    let temp = tempfile::tempdir().unwrap();
    install(&fixture_dir(), temp.path());
    let original = loaded(temp.path());
    let path = temp
        .path()
        .join(".orr/packages/objects")
        .join(original.package_digest())
        .join(ASSET);
    std::fs::write(path, b"tampered package bytes").unwrap();
    assert!(model_bindings::list_assets(temp.path()).is_err());
    assert!(model_bindings::load_asset(temp.path(), PACKAGE, ASSET).is_err());
}

#[test]
fn remove_is_one_undo_step_and_save_failure_preserves_destination_state_and_history() {
    let temp = tempfile::tempdir().unwrap();
    install(&fixture_dir(), temp.path());
    let loaded = loaded(temp.path());
    let mut bindings = bindings(temp.path());
    bindings
        .assign_validated(&[guid(1), guid(2)], &binding(&loaded), &loaded)
        .unwrap();
    bindings.remove(&[guid(1), guid(2)]).unwrap();
    assert!(bindings.document().bindings.is_empty());
    bindings.undo();
    assert_eq!(bindings.document().bindings.len(), 2);
    std::fs::create_dir(&bindings.path).unwrap();
    std::fs::write(bindings.path.join("sentinel"), b"unchanged").unwrap();
    assert!(bindings.save().is_err());
    assert!(bindings.dirty());
    assert_eq!(
        std::fs::read(bindings.path.join("sentinel")).unwrap(),
        b"unchanged"
    );
    bindings.undo();
    assert!(bindings.document().bindings.is_empty());
    bindings.redo();
    assert_eq!(bindings.document().bindings.len(), 2);
}

#[test]
fn model_sidecar_save_does_not_write_scene_or_animation_sidecar() {
    let temp = tempfile::tempdir().unwrap();
    let scene = temp.path().join("scene.yaml");
    let animation = temp.path().join("scene.animations.json");
    std::fs::write(&scene, b"deterministic scene unchanged").unwrap();
    std::fs::write(&animation, b"independent animation sidecar unchanged").unwrap();
    install(&fixture_dir(), temp.path());
    let loaded = loaded(temp.path());
    let mut bindings = bindings(temp.path());
    bindings
        .assign_validated(&[guid(1)], &binding(&loaded), &loaded)
        .unwrap();
    bindings.save().unwrap();
    assert_eq!(
        std::fs::read(scene).unwrap(),
        b"deterministic scene unchanged"
    );
    assert_eq!(
        std::fs::read(animation).unwrap(),
        b"independent animation sidecar unchanged"
    );
}

#[test]
fn oversized_save_preserves_saved_destination_and_undo_redo() {
    let temp = tempfile::tempdir().unwrap();
    install(&fixture_dir(), temp.path());
    let loaded = loaded(temp.path());
    let mut bindings = bindings(temp.path());
    bindings.save().unwrap();
    let baseline = std::fs::read(&bindings.path).unwrap();
    let guids = (0..4096).map(guid).collect::<Vec<_>>();
    bindings
        .assign_validated(&guids, &binding(&loaded), &loaded)
        .unwrap();
    assert!(
        serde_json::to_vec_pretty(bindings.document())
            .unwrap()
            .len()
            > 1024 * 1024
    );
    assert!(bindings.save().unwrap_err().contains("byte limit"));
    assert_eq!(std::fs::read(&bindings.path).unwrap(), baseline);
    assert!(bindings.dirty());
    let before = bindings.document().clone();
    assert!(bindings
        .assign_validated(&[guid(4096)], &binding(&loaded), &loaded)
        .is_err());
    assert_eq!(bindings.document(), &before);
    bindings.undo();
    assert!(bindings.document().bindings.is_empty());
    assert!(!bindings.dirty());
    bindings.redo();
    assert_eq!(bindings.document().bindings.len(), 4096);
    assert!(bindings.dirty());
}

#[test]
fn parser_rejects_unsupported_kinds_bad_schema_handles_duplicates_and_invalid_transforms() {
    let temp = tempfile::tempdir().unwrap();
    install(&fixture_dir(), temp.path());
    let loaded = loaded(temp.path());
    let mut bindings = bindings(temp.path());
    bindings
        .assign_validated(&[guid(1)], &binding(&loaded), &loaded)
        .unwrap();
    let original = serde_json::to_value(bindings.document()).unwrap();
    let path = temp.path().join("bad.models.json");
    for kind in ["animated", "skinned", "future"] {
        let mut value = original.clone();
        value["bindings"]["e_00000001"]["kind"] = kind.into();
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(Bindings::open(path.clone())
            .err()
            .unwrap()
            .contains("only static models are supported"));
    }
    let mutations: [fn(&mut serde_json::Value); 10] = [
        |v| v["version"] = 2.into(),
        |v| v["extra"] = true.into(),
        |v| v["scene"] = "/remote/scene.yaml".into(),
        |v| v["project"] = "https://remote/project".into(),
        |v| {
            let binding = v["bindings"]["e_00000001"].take();
            v["bindings"] = serde_json::json!({"12v0":binding});
        },
        |v| {
            let binding = v["bindings"]["e_00000001"].take();
            v["bindings"] = serde_json::json!({"e_0000000A":binding});
        },
        |v| v["bindings"]["e_00000001"]["transform"]["rotation"] = serde_json::json!([0, 0, 0, 2]),
        |v| v["bindings"]["e_00000001"]["transform"]["scale"] = serde_json::json!([0, 1, 1]),
        |v| v["bindings"]["e_00000001"]["asset"] = "../model.glb".into(),
        |v| v["bindings"]["e_00000001"]["clip_index"] = 0.into(),
    ];
    for mutate in mutations {
        let mut value = original.clone();
        mutate(&mut value);
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(Bindings::open(path.clone()).is_err(), "{value}");
    }
    let binding_json = serde_json::to_string(&binding(&loaded)).unwrap();
    let duplicate = format!(
        r#"{{"version":1,"scene":"scene.yaml","project":".","bindings":{{"e_00000001":{binding_json},"e_00000001":{binding_json}}}}}"#
    );
    std::fs::write(&path, duplicate).unwrap();
    assert!(Bindings::open(path.clone())
        .err()
        .unwrap()
        .contains("duplicate model binding GUID"));
    std::fs::write(&path, vec![b' '; 1024 * 1024 + 1]).unwrap();
    assert!(Bindings::open(path).err().unwrap().contains("byte limit"));
}

#[test]
fn history_is_bounded_and_missing_guid_removal_does_not_create_an_undo_entry() {
    let temp = tempfile::tempdir().unwrap();
    install(&fixture_dir(), temp.path());
    let loaded = loaded(temp.path());
    let mut bindings = bindings(temp.path());
    for n in 0..130 {
        bindings
            .assign_validated(&[guid(n)], &binding(&loaded), &loaded)
            .unwrap();
    }
    assert!(bindings.remove(&[]).is_err());
    bindings.remove(&[guid(10000)]).unwrap();
    for _ in 0..128 {
        bindings.undo();
    }
    assert_eq!(bindings.document().bindings.len(), 2);
    bindings.undo();
    assert_eq!(bindings.document().bindings.len(), 2);
}

#[test]
fn project_resolution_requires_real_directories_and_preserves_canonical_bases() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let sidecars = root.join("sidecars");
    let project = root.join("project");
    std::fs::create_dir(&sidecars).unwrap();
    std::fs::create_dir(&project).unwrap();
    assert_eq!(
        model_bindings::resolve_project(&sidecars, "../project").unwrap(),
        project
    );
    assert_eq!(model_bindings::resolve_project(&root, ".").unwrap(), root);
    assert!(model_bindings::resolve_project(root.ancestors().last().unwrap(), "..").is_err());
    assert!(model_bindings::resolve_project(&root, "/remote").is_err());
    assert!(model_bindings::resolve_project(&root, "missing").is_err());
    std::fs::write(root.join("file"), b"not a directory").unwrap();
    assert!(model_bindings::resolve_project(&root, "file").is_err());
    #[cfg(unix)]
    {
        let alias = root.join("alias");
        std::os::unix::fs::symlink(&project, &alias).unwrap();
        assert!(model_bindings::resolve_project(&root, "alias").is_err());
        assert!(model_bindings::resolve_project(&alias, "../project").is_err());
        assert!(model_bindings::open_project(&alias).is_err());
    }
}

#[cfg(unix)]
#[test]
fn sidecar_open_rejects_symlinks_and_special_files_without_blocking() {
    use std::{os::unix::fs::symlink, process::Command};
    let temp = tempfile::tempdir().unwrap();
    let regular = temp.path().join("regular.json");
    std::fs::write(
        &regular,
        serde_json::to_vec(&Document {
            version: 1,
            scene: "scene.yaml".into(),
            project: ".".into(),
            bindings: Default::default(),
        })
        .unwrap(),
    )
    .unwrap();
    let alias = temp.path().join("alias.json");
    symlink(&regular, &alias).unwrap();
    assert!(Bindings::open(alias)
        .err()
        .unwrap()
        .contains("regular file"));
    let fifo = temp.path().join("sidecar.fifo");
    assert!(Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .unwrap()
        .success());
    assert!(Bindings::open(fifo).err().unwrap().contains("regular file"));
    let broken = temp.path().join("broken.json");
    symlink(temp.path().join("absent.json"), &broken).unwrap();
    assert!(Bindings::create(broken, "scene.yaml".into(), ".".into()).is_err());
}
