//! CPU-only saved-Room recovery through the existing editor binding API.
//! This does not exercise widgets, rendering, or the editor's live dirty guard.
#![cfg(feature = "package-manager")]

#[allow(dead_code)]
#[path = "../../orr_sample/examples/room_package_usage/authoring.rs"]
mod authoring;

use orr_editor::{model_panel::ModelPanel, package_manager, Editor, HostSpec, Target};
use orr_model_bindings::model_bindings::{self, Binding, Document, LocalTransform, ModelKind};
use orr_package::{Project, Runtime};
use orr_reflect::{Guid, Scene};
use orr_sample::{
    room_game::{RoomActor, RoomConfig, RoomEscapeV1, KEY, PLAYER},
    room_project::{self, PreparedProject, PreparedScene, SEED},
};
use orr_sim::Simulation;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

const SOURCE: &str = "usage-source";
const REPLACEMENT: &str = "usage-replacement";
const ASSET: &str = "foreground.glb";

fn files(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(root, &path, out);
            } else {
                out.insert(
                    path.strip_prefix(root).unwrap().into(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut result = BTreeMap::new();
    visit(root, root, &mut result);
    result
}

fn source(root: &Path, name: &str) -> PathBuf {
    let path = root.join(name);
    fs::create_dir(&path).unwrap();
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/imported_scene_demo/foreground.glb"),
        path.join(ASSET),
    )
    .unwrap();
    fs::write(
        path.join("orr.package.json"),
        serde_json::to_vec(&serde_json::json!({
            "schema": 1, "name": name, "version": "1.0.0", "engine": "*",
            "capabilities": ["models"], "dependencies": {}, "files": [ASSET]
        }))
        .unwrap(),
    )
    .unwrap();
    path
}

fn fixture(root: &Path, sources: &Path) -> BTreeMap<u32, Guid> {
    fs::create_dir(root).unwrap();
    fs::create_dir(sources).unwrap();
    fs::write(
        root.join("orr.project.json"),
        serde_json::to_vec(&serde_json::json!({
            "schema": 2, "engine": "*", "entry": {
                "game": "room-escape-v1", "scene": "room.scene.yaml",
                "models": "room.models.json"
            }
        }))
        .unwrap(),
    )
    .unwrap();
    let simulation = Simulation::<RoomEscapeV1>::new(RoomConfig, 60, SEED);
    let text = Scene::unbake(&room_project::types(), simulation.frame(), None)
        .unwrap()
        .to_yaml();
    fs::write(root.join("room.scene.yaml"), &text).unwrap();
    Project::open_for_install(root, Runtime::content_only().engine_version)
        .unwrap()
        .install(&[source(sources, SOURCE), source(sources, REPLACEMENT)])
        .unwrap();
    let loaded = model_bindings::load_asset(root, SOURCE, ASSET).unwrap();
    let binding = Binding::from_asset(
        SOURCE.into(),
        ASSET.into(),
        &loaded,
        LocalTransform::default(),
    )
    .unwrap();
    let scene = PreparedScene::parse(&text).unwrap();
    let guids: BTreeMap<_, _> = scene
        .frame()
        .entities()
        .filter_map(|entity| {
            let kind = scene.frame().get::<RoomActor>(entity).unwrap().kind;
            [PLAYER, KEY]
                .contains(&kind)
                .then(|| (kind, scene.index().guid(entity).unwrap().clone()))
        })
        .collect();
    assert_eq!(guids.len(), 2);
    fs::write(
        root.join("room.models.json"),
        serde_json::to_vec(&Document {
            version: 2,
            scene: "room.scene.yaml".into(),
            project: ".".into(),
            bindings: guids
                .values()
                .map(|guid| (guid.to_string(), binding.clone()))
                .collect(),
        })
        .unwrap(),
    )
    .unwrap();
    guids
}

fn open_editor(root: &Path) -> (Editor, ModelPanel) {
    let (_, path, scene, models) = PreparedProject::open(root).unwrap().into_parts();
    let mut editor = Editor::start(&HostSpec::PreparedRoom {
        scene: path,
        text: scene.text().into(),
        listen: None,
        debug_hooks: false,
    })
    .unwrap();
    let mut panel = ModelPanel::default();
    panel.install_room(models).unwrap();
    for attempt in 0..1600 {
        editor.sync();
        if editor.yard_rows_coherent() && panel.scene_matches(&editor) {
            return (editor, panel);
        }
        assert!(attempt < 1599, "Room editor did not become coherent");
        std::thread::sleep(Duration::from_millis(5));
    }
    unreachable!()
}

/// Inspecting saved authority must not touch live editor state or disk bytes.
fn inspect(
    request: &authoring::Request,
    editor: &Editor,
    panel: &ModelPanel,
) -> authoring::UsageReport {
    let disk = files(&request.project);
    let frame = PreparedProject::open(&request.project)
        .unwrap()
        .scene()
        .frame()
        .to_bytes();
    let checksum = editor.checksum();
    let history = editor.history().clone();
    let bindings = panel.bindings.as_ref().unwrap();
    let document = bindings.document().clone();
    let dirty = bindings.dirty();
    let report = authoring::inspect_usage(request).unwrap();
    assert_eq!(files(&request.project), disk);
    assert_eq!(
        PreparedProject::open(&request.project)
            .unwrap()
            .scene()
            .frame()
            .to_bytes(),
        frame
    );
    assert_eq!(editor.checksum(), checksum);
    assert_eq!(editor.history(), &history);
    assert_eq!(panel.bindings.as_ref().unwrap().document(), &document);
    assert_eq!(panel.bindings.as_ref().unwrap().dirty(), dirty);
    report
}

fn removal_blocked(root: &Path) {
    let before = files(root);
    assert!(package_manager::plan_remove(root, SOURCE).is_err());
    assert_eq!(files(root), before);
}

#[test]
fn saved_usage_guides_partial_then_complete_editor_reassignment_and_checked_removal() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("room");
    let sources = temp.path().join("sources");
    let guids = fixture(&root, &sources);
    let source_bytes = files(&sources);
    let (mut editor, mut panel) = open_editor(&root);
    let checksum = editor.checksum();
    let history = editor.history().clone();
    let frame = PreparedProject::open(&root)
        .unwrap()
        .scene()
        .frame()
        .to_bytes();
    let lock = Project::open(&root, room_project::compiled_runtime())
        .unwrap()
        .verify()
        .unwrap();
    let request = authoring::Request {
        project: root.clone(),
        package: SOURCE.into(),
        digest: lock.packages[SOURCE].digest.clone(),
    };
    let initial = inspect(&request, &editor, &panel);
    assert_eq!(initial.bindings.len(), 2);
    assert_eq!(initial.package.name, SOURCE);
    assert_eq!(initial.package.digest, request.digest);
    assert!(initial.package.direct);
    let mut expected_guids: Vec<_> = guids.values().map(ToString::to_string).collect();
    expected_guids.sort();
    assert_eq!(
        initial
            .bindings
            .iter()
            .map(|binding| binding.guid.clone())
            .collect::<Vec<_>>(),
        expected_guids
    );
    for binding in &initial.bindings {
        assert_eq!(binding.asset, ASSET);
        assert_eq!(binding.kind, ModelKind::Static);
        assert_eq!(binding.package_digest, request.digest);
        assert_eq!(
            binding.source_hash,
            panel.bindings.as_ref().unwrap().document().bindings[&binding.guid].source_hash
        );
    }
    removal_blocked(&root);

    editor.select(Some(Target::Guid(guids[&PLAYER].clone())));
    panel.package = REPLACEMENT.into();
    panel.asset = ASSET.into();
    panel.assign(&editor).unwrap();
    assert!(panel.bindings.as_ref().unwrap().dirty());
    assert_eq!(inspect(&request, &editor, &panel).bindings.len(), 2);
    panel.save(&editor).unwrap();
    let partial = inspect(&request, &editor, &panel);
    assert_eq!(partial.bindings.len(), 1);
    assert_eq!(partial.bindings[0].guid, guids[&KEY].to_string());
    removal_blocked(&root);

    editor.select(Some(Target::Guid(guids[&KEY].clone())));
    panel.assign(&editor).unwrap();
    let all_reassigned = panel.bindings.as_ref().unwrap().document().clone();
    assert!(all_reassigned
        .bindings
        .values()
        .all(|binding| binding.package == REPLACEMENT));
    assert!(panel.bindings.as_ref().unwrap().dirty());
    // The read-only helper intentionally sees the SAVED sidecar, not dirty UI state.
    assert_eq!(
        serde_json::to_value(inspect(&request, &editor, &panel)).unwrap(),
        serde_json::to_value(&partial).unwrap()
    );
    removal_blocked(&root);
    panel.undo(&editor).unwrap();
    assert_eq!(
        panel.bindings.as_ref().unwrap().document().bindings[&guids[&KEY].to_string()].package,
        SOURCE
    );
    assert_eq!(inspect(&request, &editor, &panel).bindings.len(), 1);
    panel.redo(&editor).unwrap();
    assert_eq!(panel.bindings.as_ref().unwrap().document(), &all_reassigned);
    assert_eq!(inspect(&request, &editor, &panel).bindings.len(), 1);
    panel.save(&editor).unwrap();
    assert!(!panel.bindings.as_ref().unwrap().dirty());
    assert!(inspect(&request, &editor, &panel).bindings.is_empty());

    let before_removal = files(&root);
    let plan = package_manager::plan_remove(&root, SOURCE).unwrap();
    assert_eq!(files(&root), before_removal);
    let removed = package_manager::apply_remove(&plan).unwrap();
    assert!(!removed.packages.contains_key(SOURCE));
    assert!(removed.packages.contains_key(REPLACEMENT));
    let after_removal = files(&root);
    assert_eq!(
        before_removal.keys().collect::<Vec<_>>(),
        after_removal.keys().collect::<Vec<_>>()
    );
    for (path, bytes) in &before_removal {
        if path != Path::new("orr.packages.lock.json") {
            assert_eq!(&after_removal[path], bytes, "preserved {}", path.display());
        }
    }
    assert_eq!(files(&sources), source_bytes);
    let reopened = PreparedProject::open(&root).unwrap();
    assert_eq!(reopened.models().document, all_reassigned);
    assert_eq!(reopened.scene().frame().to_bytes(), frame);
    assert_eq!(editor.checksum(), checksum);
    assert_eq!(editor.history(), &history);
    drop(editor);
    let (reopened_editor, reopened_panel) = open_editor(&root);
    assert_eq!(reopened_editor.checksum(), checksum);
    assert_eq!(
        reopened_panel.bindings.as_ref().unwrap().document(),
        &all_reassigned
    );
    assert!(!reopened_panel.bindings.as_ref().unwrap().dirty());
}
