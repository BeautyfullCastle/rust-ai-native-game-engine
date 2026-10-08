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

const SAVE_TRACE_CLIENT: &str = "room-ui-save-aba";

fn trace_external_save(
    client: &mut orr_remote::ErpClient,
    case: &str,
    params: serde_json::Value,
    succeeds: bool,
    read_only: bool,
    expected_path: &std::path::Path,
) -> Option<serde_json::Value> {
    // A synchronous response is the completion barrier for this exact request.
    // This client never pumps the Editor or its separate notification queue.
    let result = client.call("scene.save", params.clone());
    assert_eq!(result.is_ok(), succeeds, "{case}: {result:?}");
    if !succeeds {
        assert!(
            matches!(&result, Err(orr_remote::ClientError::Rpc(error)) if error.kind() == Some("io"))
        );
    }
    let error = result.as_ref().err().map(ToString::to_string);
    let reply = result.ok();
    let state = client.call("sim.state", serde_json::Value::Null).unwrap();
    assert_eq!(state["scene_path"], expected_path.display().to_string());
    let history = client
        .call(
            "activity.list",
            serde_json::json!({"include_reads": true, "limit": 200}),
        )
        .unwrap();
    let entry = history["entries"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|entry| entry["client"] == SAVE_TRACE_CLIENT && entry["method"] == "scene.save")
        .expect("completed save must have an attributable activity entry");
    assert_eq!(entry["ok"], succeeds, "{case}: {entry}");
    assert_eq!(entry["read"], read_only, "{case}: {entry}");
    assert_eq!(entry["kind"], if read_only { "read" } else { "edit" });
    if matches!(case, "a-to-b" | "b-to-a") {
        assert_eq!(entry["scene_path_changed"], true, "{case}: {entry}");
    } else {
        assert!(entry.get("scene_path_changed").is_none(), "{case}: {entry}");
    }
    eprintln!(
        "room-ui-save-trace {}",
        serde_json::json!({
            "case": case,
            "request": params,
            "reply_written": reply.as_ref().and_then(|value| value.get("written")),
            "rpc_error": error,
            "state_scene_path": state["scene_path"],
            "activity": entry,
        })
    );
    reply
}

#[test]
fn actual_external_save_aba_before_editor_pump_permanently_retires_room_ui() {
    let fixture = Fixture::new();
    let mut h = fixture.app();
    let edited = pending(&mut h);
    h.state_mut().editor.sync();
    let a = fixture.root.join("room.scene.yaml");
    let b = fixture.root.join("external.scene.yaml");
    let ui = fixture.root.join("room.ui.json");
    let original_ui = fs::read(&ui).unwrap();
    let failed = fixture.root.join("missing-directory/failed.scene.yaml");
    {
        let client = h
            .state_mut()
            .editor
            .agent_client(SAVE_TRACE_CLIENT)
            .unwrap();
        let reply = trace_external_save(
            client,
            "no-write",
            serde_json::json!({"write": false, "path": b}),
            true,
            true,
            &a,
        )
        .unwrap();
        assert!(reply.get("written").is_none());
        assert!(!b.exists());
        assert!(
            trace_external_save(
                client,
                "failed-write",
                serde_json::json!({"write": true, "path": failed}),
                false,
                false,
                &a,
            )
            .is_none()
        );
        assert!(!failed.exists());
        let reply = trace_external_save(
            client,
            "same-path",
            serde_json::json!({"write": true, "path": a}),
            true,
            false,
            &a,
        )
        .unwrap();
        assert_eq!(reply["written"], a.display().to_string());
        assert_eq!(
            fs::read_to_string(&a).unwrap(),
            reply["text"].as_str().unwrap()
        );
    }
    h.state_mut().editor.sync();
    assert!(h.state_mut().collect_ui.as_mut().unwrap().room_active());
    assert_eq!(h.state().collect_ui.as_ref().unwrap().document(), &edited);
    assert_eq!(fs::read(&ui).unwrap(), original_ui);

    // No Editor pump, sync, call, or harness step is allowed inside this block.
    // Both successful writes and their ordered response/state/activity barriers
    // complete on the external ERP client before the Editor can observe either.
    {
        let client = h
            .state_mut()
            .editor
            .agent_client(SAVE_TRACE_CLIENT)
            .unwrap();
        assert!(!b.exists());
        let away = trace_external_save(
            client,
            "a-to-b",
            serde_json::json!({"write": true, "path": b}),
            true,
            false,
            &b,
        )
        .unwrap();
        assert_eq!(away["written"], b.display().to_string());
        assert_eq!(
            fs::read_to_string(&b).unwrap(),
            away["text"].as_str().unwrap()
        );
        // Prove the B-to-A request performs a real write, not only a path update.
        fs::write(
            &a,
            "test sentinel: the external save must replace these bytes",
        )
        .unwrap();
        let back = trace_external_save(
            client,
            "b-to-a",
            serde_json::json!({"write": true, "path": a}),
            true,
            false,
            &a,
        )
        .unwrap();
        assert_eq!(back["written"], a.display().to_string());
        assert_eq!(
            fs::read_to_string(&a).unwrap(),
            back["text"].as_str().unwrap()
        );
        assert_eq!(fs::read(&a).unwrap(), fs::read(&b).unwrap());
    }
    assert_eq!(h.state().editor.path().as_ref(), Some(&a));
    h.state_mut().editor.sync();
    let panel = h.state_mut().collect_ui.as_mut().unwrap();
    assert!(
        !panel.room_active(),
        "successful external Save As A-to-B-to-A must retire the Room UI even when the Editor only observes final path A"
    );
    assert!(panel.apply_document(edited.clone()).is_err());
    assert!(!panel.undo());
    assert!(panel.save().is_err());
    assert_eq!(panel.document(), &edited);
    assert_eq!(fs::read(&ui).unwrap(), original_ui);
    h.state_mut().editor.sync();
    assert!(!h.state_mut().collect_ui.as_mut().unwrap().room_active());
}
