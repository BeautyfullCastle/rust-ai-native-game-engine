//! Actual EditorApp source transitions must retire the independently saved Room UI.
#![cfg(all(
    feature = "room-ui",
    feature = "project-create",
    target_os = "linux",
    target_arch = "x86_64"
))]
#![allow(clippy::disallowed_types)]
use egui_kittest::Harness;
use orr_editor::{collect_ui_panel::Panel, Editor, EditorApp, HostSpec};
use orr_sample::{
    authored_ui::Kind,
    project_create::{create, CreateOptions, ROOM_UI_TEMPLATE},
    room_project::PreparedProject,
};
use std::{
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap().join("room");
        create(&CreateOptions {
            output: root.clone(),
            template: ROOM_UI_TEMPLATE.into(),
            seed: "editor-room-ui-lifecycle".into(),
        })
        .unwrap();
        // Prove UI retirement is independent of optional camera attachment.
        let path = root.join("orr.project.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        manifest["entry"].as_object_mut().unwrap().remove("camera");
        fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        Self { _temp: temp, root }
    }
    fn app(&self) -> Harness<'static, EditorApp> {
        let mut project = PreparedProject::open_with_ui(&self.root, true).unwrap();
        let ui = project.take_ui().unwrap();
        let (_, path, scene, models) = project.into_parts();
        let mut editor = Editor::start(&HostSpec::PreparedRoom {
            scene: path,
            text: scene.text().into(),
            listen: None,
            debug_hooks: false,
        })
        .unwrap();
        editor.sync();
        let mut panel = Panel::new_room(ui);
        panel.bind_room(&editor).unwrap();
        Harness::builder()
            .with_size([1400.0, 1000.0])
            .build_eframe(move |_| {
                let mut app = EditorApp::new(editor, None);
                app.models.install_room(models).unwrap();
                app.collect_ui = Some(panel);
                app
            })
    }
}
fn pending(h: &mut Harness<'_, EditorApp>) -> orr_sample::authored_ui::Document {
    h.run_steps(3);
    let panel = h.state_mut().collect_ui.as_mut().unwrap();
    let mut doc = panel.document().clone();
    if let Kind::Label { text, .. } =
        &mut doc.nodes.iter_mut().find(|n| n.id == "title").unwrap().kind
    {
        *text = "Unsaved Room UI".into();
    }
    panel.apply_document(doc.clone()).unwrap();
    doc
}
#[test]
fn actual_editor_failed_open_preserves_but_open_aba_and_save_as_retire_ui() {
    for save_as in [false, true] {
        let fixture = Fixture::new();
        let mut h = fixture.app();
        let edited = pending(&mut h);
        let ui = fixture.root.join("room.ui.json");
        let bytes = fs::read(&ui).unwrap();
        assert!(!h
            .state_mut()
            .editor
            .open_path(&fixture.root.join("missing.yaml")));
        let invalid = fixture.root.join("invalid.scene.yaml");
        fs::write(&invalid, "not a valid scene: [").unwrap();
        assert!(!h.state_mut().editor.open_path(&invalid));
        h.state_mut().editor.sync();
        h.run_steps(2);
        assert!(h.state_mut().collect_ui.as_mut().unwrap().room_active());
        assert_eq!(h.state().collect_ui.as_ref().unwrap().document(), &edited);
        let a = fixture.root.join("room.scene.yaml");
        let b = fixture.root.join("other.scene.yaml");
        if save_as {
            assert!(h.state_mut().editor.save_as(&b));
        } else {
            fs::copy(&a, &b).unwrap();
            assert!(h.state_mut().editor.open_path(&b));
        }
        assert!(h.state_mut().collect_ui.as_mut().unwrap().save().is_err());
        assert_eq!(fs::read(&ui).unwrap(), bytes);
        assert_eq!(h.state().collect_ui.as_ref().unwrap().document(), &edited);
        assert!(h.state_mut().editor.open_path(&a));
        h.run_steps(3);
        assert!(!h.state_mut().collect_ui.as_mut().unwrap().room_active());
        assert!(h.state_mut().collect_ui.as_mut().unwrap().save().is_err());
        assert!(!h.state_mut().collect_ui.as_mut().unwrap().undo());
        assert_eq!(fs::read(&ui).unwrap(), bytes);
        assert_eq!(h.state().collect_ui.as_ref().unwrap().document(), &edited);
    }
}
#[test]
fn actual_external_erp_load_retires_room_ui_and_manifest_change_blocks_save() {
    for external_save in [false, true] {
        let fixture = Fixture::new();
        let mut h = fixture.app();
        let edited = pending(&mut h);
        let ui = fixture.root.join("room.ui.json");
        let bytes = fs::read(&ui).unwrap();
        let a = fixture.root.join("room.scene.yaml");
        let text = fs::read_to_string(&a).unwrap();
        if external_save {
            h.state_mut().editor.agent_client("room-ui-lifecycle").unwrap().call("scene.save",serde_json::json!({"write":true,"path":fixture.root.join("external.scene.yaml").display().to_string()})).unwrap();
        } else {
            h.state_mut()
                .editor
                .agent_client("room-ui-lifecycle")
                .unwrap()
                .call(
                    "scene.load",
                    serde_json::json!({"text":text,"path":a.display().to_string()}),
                )
                .unwrap();
        }
        let start = Instant::now();
        loop {
            h.state_mut().editor.sync();
            h.run_steps(1);
            if !h.state_mut().collect_ui.as_mut().unwrap().room_active() {
                break;
            }
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "external load not observed"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(h.state_mut().collect_ui.as_mut().unwrap().save().is_err());
        assert_eq!(fs::read(ui).unwrap(), bytes);
        assert_eq!(h.state().collect_ui.as_ref().unwrap().document(), &edited);
    }
    let fixture = Fixture::new();
    let mut h = fixture.app();
    pending(&mut h);
    let ui = fixture.root.join("room.ui.json");
    let bytes = fs::read(&ui).unwrap();
    let path = fixture.root.join("orr.project.json");
    let mut changed = fs::read(&path).unwrap();
    changed.push(b'\n');
    fs::write(path, changed).unwrap();
    assert!(h.state_mut().collect_ui.as_mut().unwrap().save().is_err());
    assert_eq!(fs::read(ui).unwrap(), bytes);
}
