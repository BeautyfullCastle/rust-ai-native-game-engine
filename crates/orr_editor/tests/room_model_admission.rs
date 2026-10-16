//! Room sidecar mutations use the same closed admission policy as reopening.
#![cfg(feature = "room-project")]
use orr_editor::{model_panel::ModelPanel, Editor, HostSpec, Target};
use orr_model_bindings::model_bindings::{self, Binding, Document, LocalTransform};
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
    sync::Arc,
    time::Duration,
};

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    guids: BTreeMap<u32, Guid>,
}
impl Fixture {
    fn new(large: bool) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("project");
        fs::create_dir(&root).unwrap();
        let source = temp.path().join("source");
        fs::create_dir(&source).unwrap();
        let demo = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/imported_scene_demo/foreground.glb");
        let bytes = fs::read(demo).unwrap();
        let bytes = if large { large_glb(&bytes) } else { bytes };
        for name in ["one.glb", "two.glb"] {
            fs::write(source.join(name), &bytes).unwrap();
        }
        fs::write(source.join("orr.package.json"), serde_json::to_vec(&serde_json::json!({"schema":1,"name":"admission-models","version":"1.0.0","engine":"*","capabilities":["models"],"dependencies":{},"files":["one.glb","two.glb"]})).unwrap()).unwrap();
        fs::write(root.join("orr.project.json"), serde_json::to_vec(&serde_json::json!({"schema":2,"engine":"*","entry":{"game":"room-escape-v1","scene":"room.scene.yaml","models":"room.models.json"}})).unwrap()).unwrap();
        let simulation = Simulation::<RoomEscapeV1>::new(RoomConfig, 60, SEED);
        let text = Scene::unbake(&room_project::types(), simulation.frame(), None)
            .unwrap()
            .to_yaml();
        fs::write(root.join("room.scene.yaml"), &text).unwrap();
        Project::open_for_install(&root, Runtime::content_only().engine_version)
            .unwrap()
            .install(&[source])
            .unwrap();
        let loaded = model_bindings::load_asset(&root, "admission-models", "one.glb").unwrap();
        if large {
            assert_eq!(
                loaded
                    .static_model()
                    .unwrap()
                    .source()
                    .images
                    .iter()
                    .map(|i| i.rgba8.len())
                    .sum::<usize>(),
                31 * 1024 * 1024
            );
        }
        let binding = Binding::from_asset(
            "admission-models".into(),
            "one.glb".into(),
            &loaded,
            LocalTransform::default(),
        )
        .unwrap();
        let scene = PreparedScene::parse(&text).unwrap();
        let guids: BTreeMap<_, _> = scene
            .frame()
            .entities()
            .filter_map(|e| {
                let kind = scene.frame().get::<RoomActor>(e).unwrap().kind;
                [PLAYER, KEY]
                    .contains(&kind)
                    .then(|| (kind, scene.index().guid(e).unwrap().clone()))
            })
            .collect();
        let document = Document {
            version: 2,
            scene: "room.scene.yaml".into(),
            project: ".".into(),
            bindings: [(guids[&PLAYER].to_string(), binding)].into(),
        };
        fs::write(
            root.join("room.models.json"),
            serde_json::to_vec(&document).unwrap(),
        )
        .unwrap();
        Self {
            _temp: temp,
            root,
            guids,
        }
    }
    fn panel(&self) -> (Editor, ModelPanel) {
        let (_, path, scene, models) = PreparedProject::open(&self.root).unwrap().into_parts();
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
                break;
            }
            assert!(attempt < 1599, "room model panel did not become coherent");
            std::thread::sleep(Duration::from_millis(5));
        }
        (editor, panel)
    }
}
// Actual bounded GLB: 31 MiB of RGBA pixels in two legal PNG resources.
// Zero pixels serialize below the 64 MiB cooked JSON limit. Two distinct assets
// exceed the Room aggregate only once required metadata/geometry are counted.
fn large_glb(bytes: &[u8]) -> Vec<u8> {
    let json_len = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    let mut doc: serde_json::Value = serde_json::from_slice(&bytes[20..20 + json_len]).unwrap();
    let mut bin = bytes[28 + json_len..].to_vec();
    let mut images = Vec::new();
    for height in [2048, 1920] {
        let mut png = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png, 2048, height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer
                .write_image_data(&vec![0; 2048 * height as usize * 4])
                .unwrap();
        }
        while !bin.len().is_multiple_of(4) {
            bin.push(0);
        }
        let view = doc["bufferViews"].as_array().unwrap().len();
        doc["bufferViews"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({"buffer":0,"byteOffset":bin.len(),"byteLength":png.len()}));
        bin.extend(png);
        images.push(serde_json::json!({"bufferView":view,"mimeType":"image/png"}));
    }
    doc["images"] = images.into();
    doc["buffers"][0]["byteLength"] = bin.len().into();
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
    let mut json = serde_json::to_vec(&doc).unwrap();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    let mut out = Vec::new();
    for n in [
        0x46546c67u32,
        2,
        (28 + json.len() + bin.len()) as u32,
        json.len() as u32,
        0x4e4f534a,
    ] {
        out.extend(n.to_le_bytes());
    }
    out.extend(json);
    out.extend((bin.len() as u32).to_le_bytes());
    out.extend(0x004e4942u32.to_le_bytes());
    out.extend(bin);
    out
}
#[test]
fn aggregate_assignment_rejection_preserves_cache_document_dirty_and_history() {
    let fixture = Fixture::new(true);
    let (mut editor, mut panel) = fixture.panel();
    editor.select(Some(Target::Guid(fixture.guids[&KEY].clone())));
    panel.package = "admission-models".into();
    panel.asset = "one.glb".into();
    panel.assign(&editor).unwrap(); // Shared identity is charged only once.
    let assigned = panel.bindings.as_ref().unwrap().document().clone();
    panel.undo(&editor).unwrap();
    let before = panel.bindings.as_ref().unwrap().document().clone();
    assert!(!panel.bindings.as_ref().unwrap().dirty());
    let old = panel.placements(&editor)[0].model.clone();
    panel.asset = "two.glb".into();
    assert!(panel
        .assign(&editor)
        .unwrap_err()
        .contains("aggregate decoded"));
    assert_eq!(panel.bindings.as_ref().unwrap().document(), &before);
    assert!(!panel.bindings.as_ref().unwrap().dirty());
    assert!(Arc::ptr_eq(&old, &panel.placements(&editor)[0].model));
    panel.redo(&editor).unwrap();
    assert_eq!(panel.bindings.as_ref().unwrap().document(), &assigned);
    panel.save(&editor).unwrap();
    PreparedProject::open(&fixture.root).unwrap();
}
#[test]
fn last_binding_removal_and_forged_hints_preserve_last_good_state() {
    let fixture = Fixture::new(false);
    let (mut editor, mut panel) = fixture.panel();
    editor.select(Some(Target::Guid(fixture.guids[&PLAYER].clone())));
    let before = panel.bindings.as_ref().unwrap().document().clone();
    let old = panel.placements(&editor)[0].model.clone();
    assert!(panel.remove(&editor).is_err());
    assert_eq!(panel.bindings.as_ref().unwrap().document(), &before);
    assert!(!panel.bindings.as_ref().unwrap().dirty());
    assert!(Arc::ptr_eq(&old, &panel.placements(&editor)[0].model));
    panel.undo(&editor).unwrap();
    assert_eq!(panel.bindings.as_ref().unwrap().document(), &before);
    let old = panel.placements(&editor)[0].model.clone();
    // The alternate scene has identical contents: identity, not content equivalence, is required.
    fs::copy(
        fixture.root.join("room.scene.yaml"),
        fixture.root.join("copy.scene.yaml"),
    )
    .unwrap();
    for (project, scene) in [
        ("../project", "room.scene.yaml"),
        (".", "copy.scene.yaml"),
        (".", "./room.scene.yaml"),
    ] {
        let mut forged = before.clone();
        forged.project = project.into();
        forged.scene = scene.into();
        fs::write(
            fixture.root.join("room.models.json"),
            serde_json::to_vec(&forged).unwrap(),
        )
        .unwrap();
        assert!(panel.open_for_editor(&editor, false).is_err());
        assert_eq!(panel.bindings.as_ref().unwrap().document(), &before);
        assert!(!panel.bindings.as_ref().unwrap().dirty());
        assert!(Arc::ptr_eq(&old, &panel.placements(&editor)[0].model));
    }
    // Even a caller using the generic editing API cannot use the Room panel's Save to persist an empty sidecar.
    panel
        .bindings
        .as_mut()
        .unwrap()
        .remove(&[fixture.guids[&PLAYER].clone()])
        .unwrap();
    let bytes = fs::read(fixture.root.join("room.models.json")).unwrap();
    assert!(panel.save(&editor).is_err());
    assert_eq!(
        fs::read(fixture.root.join("room.models.json")).unwrap(),
        bytes
    );
}

#[test]
fn unknown_guid_open_rejection_preserves_document_cache_and_redo() {
    let fixture = Fixture::new(false);
    let (mut editor, mut panel) = fixture.panel();
    editor.select(Some(Target::Guid(fixture.guids[&KEY].clone())));
    panel.package = "admission-models".into();
    panel.asset = "one.glb".into();
    panel.assign(&editor).unwrap();
    let assigned = panel.bindings.as_ref().unwrap().document().clone();
    panel.undo(&editor).unwrap();
    let before = panel.bindings.as_ref().unwrap().document().clone();
    let old = panel.placements(&editor)[0].model.clone();
    let mut forged = before.clone();
    let unknown = Guid::parse("e_ffffffffffffffffffffffffffffffff").unwrap();
    assert!(editor
        .rows()
        .iter()
        .all(|row| row.guid.as_ref() != Some(&unknown)));
    forged.bindings.insert(
        unknown.to_string(),
        before.bindings.values().next().unwrap().clone(),
    );
    fs::write(
        fixture.root.join("room.models.json"),
        serde_json::to_vec(&forged).unwrap(),
    )
    .unwrap();
    assert!(panel
        .open_for_editor(&editor, false)
        .unwrap_err()
        .contains("GUID is absent"));
    assert_eq!(panel.bindings.as_ref().unwrap().document(), &before);
    assert!(!panel.bindings.as_ref().unwrap().dirty());
    assert!(Arc::ptr_eq(&old, &panel.placements(&editor)[0].model));
    panel.redo(&editor).unwrap();
    assert_eq!(panel.bindings.as_ref().unwrap().document(), &assigned);
    // Save also validates orphaned bindings inserted through the generic API.
    let loaded = model_bindings::load_asset(&fixture.root, "admission-models", "one.glb").unwrap();
    let binding = before.bindings.values().next().unwrap();
    panel
        .bindings
        .as_mut()
        .unwrap()
        .assign_validated(&[unknown], binding, &loaded)
        .unwrap();
    let bytes = fs::read(fixture.root.join("room.models.json")).unwrap();
    assert!(panel.save(&editor).unwrap_err().contains("GUID is absent"));
    assert_eq!(
        fs::read(fixture.root.join("room.models.json")).unwrap(),
        bytes
    );
}
