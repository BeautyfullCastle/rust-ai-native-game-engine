//! CPU actual-widget focus-loss regression; never initializes a GPU.
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
use super::*;
use crate::{Editor, EditorApp, HostSpec};
use egui::accesskit::{Action, ActionRequest, Role};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use orr_model_bindings::model_bindings::{self, Binding, Document, LocalTransform};
use orr_sample::{
    room_game::{KEY, PLAYER, RoomActor, RoomConfig, RoomEscapeV1},
    room_project::{self, PreparedProject, PreparedScene, SEED},
};
use std::{fs, path::Path};
fn fixture(root: &Path) {
    fs::create_dir(root).unwrap();
    let sim = orr_sim::Simulation::<RoomEscapeV1>::new(RoomConfig, 60, SEED);
    let text = orr_reflect::Scene::unbake(&room_project::types(), sim.frame(), None)
        .unwrap()
        .to_yaml();
    fs::write(root.join("room.scene.yaml"), &text).unwrap();
    fs::write(
        root.join("room.camera.json"),
        orr_sample::room_camera::Document::readable_default()
            .to_bytes()
            .unwrap(),
    )
    .unwrap();
    fs::write(root.join("orr.project.json"),serde_json::to_vec(&serde_json::json!({"schema":2,"engine":"*","entry":{"game":"room-escape-v1","scene":"room.scene.yaml","models":"room.models.json","camera":"room.camera.json"}})).unwrap()).unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/imported_scene_demo")
        .canonicalize()
        .unwrap();
    orr_package::Project::open_for_install(
        root,
        orr_package::Runtime::content_only().engine_version,
    )
    .unwrap()
    .install(&[source])
    .unwrap();
    let loaded =
        model_bindings::load_asset(root, "sample-imported-scene", "foreground.glb").unwrap();
    let binding = Binding::from_asset(
        "sample-imported-scene".into(),
        "foreground.glb".into(),
        &loaded,
        LocalTransform::default(),
    )
    .unwrap();
    let scene = PreparedScene::parse(&text).unwrap();
    let bindings = scene
        .frame()
        .entities()
        .filter(|entity| {
            scene
                .frame()
                .get::<RoomActor>(*entity)
                .is_some_and(|actor| [PLAYER, KEY].contains(&actor.kind))
        })
        .map(|entity| {
            (
                scene.index().guid(entity).unwrap().to_string(),
                binding.clone(),
            )
        })
        .collect();
    fs::write(
        root.join("room.models.json"),
        serde_json::to_vec(&Document {
            version: 2,
            scene: "room.scene.yaml".into(),
            project: ".".into(),
            bindings,
        })
        .unwrap(),
    )
    .unwrap();
}

struct State {
    app: EditorApp,
    probe: u32,
}
fn setup() -> (tempfile::TempDir, Harness<'static, State>) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("room");
    fixture(&root);
    let mut prepared = PreparedProject::open(&root).unwrap();
    let camera = prepared.take_camera().unwrap();
    let (_, path, scene, models) = prepared.into_parts();
    let mut editor = Editor::start(&HostSpec::PreparedRoom {
        scene: path,
        text: scene.text().into(),
        listen: None,
        debug_hooks: false,
    })
    .unwrap();
    editor.sync();
    editor.install_room_camera(camera.document.clone()).unwrap();
    let mut app = EditorApp::new(editor, None);
    app.models.install_room(models).unwrap();
    app.room_camera = Some(crate::room_camera_panel::Panel::new(camera));
    let mut h = Harness::builder().with_size([1000., 1500.]).build_ui_state(
        |ui, state: &mut State| {
            state.app.editor.sync();
            if ui.button("Dispatch probe").clicked() {
                state.probe += 1;
            }
            state.app.show_room_camera_capture(ui);
        },
        State { app, probe: 0 },
    );
    h.run_steps(6);
    assert!(h.state().app.room_capture_context(false).is_some());
    (temp, h)
}
fn action(h: &Harness<'_, State>, label: &str) -> egui::Event {
    let (target_node, target_tree) = h
        .get_by_role_and_label(Role::Button, label)
        .accesskit_node()
        .locate();
    egui::Event::AccessKitActionRequest(ActionRequest {
        target_node,
        target_tree,
        action: Action::Click,
        data: None,
    })
}
fn queue(h: &mut Harness<'_, State>, label: &str) {
    let event = action(h, label);
    h.event(event);
}
fn click(h: &mut Harness<'_, State>, label: &str) {
    h.run_steps(2);
    assert!(
        !h.get_by_role_and_label(Role::Button, label)
            .accesskit_node()
            .is_disabled(),
        "{label}"
    );
    queue(h, label);
    h.run_steps(3);
}
fn lost_click(h: &mut Harness<'_, State>, label: &str) {
    h.run_steps(2);
    let before = h.state().probe;
    let probe = action(h, "Dispatch probe");
    let target = action(h, label);
    // Harness::event queues individual frames, so put the complete adversarial
    // input batch directly in RawInput before one production UI invocation.
    h.input_mut().focused = true;
    h.input_mut().events.extend([
        egui::Event::WindowFocused(false),
        egui::Event::WindowFocused(true),
        probe,
        target,
    ]);
    h.step();
    assert_eq!(
        h.state().probe,
        before + 1,
        "batched AccessKit click must actually dispatch despite focus events"
    );
}
#[test]
fn same_frame_focus_loss_blocks_stage_apply_undo_redo_save_but_normal_dispatch_works() {
    let (temp, mut h) = setup();
    let camera_path = temp.path().join("room/room.camera.json");
    let original = fs::read(&camera_path).unwrap();
    let authored = h
        .state()
        .app
        .room_camera
        .as_ref()
        .unwrap()
        .document()
        .clone();
    let frame = h
        .state()
        .app
        .editor
        .snapshot()
        .unwrap()
        .predicted()
        .checksum();
    let history = h.state().app.editor.history().clone();
    h.state_mut().app.editor.camera3d.distance *= 1.25;
    let live = h.state().app.editor.camera3d;
    lost_click(&mut h, "Stage viewport camera");
    assert!(!h.state().app.room_camera_capture.staged.busy());
    click(&mut h, "Stage viewport camera");
    assert!(h.state().app.room_camera_capture.staged.busy());
    assert_eq!(h.state().app.editor.camera3d, live);
    lost_click(&mut h, "Apply staged viewport camera");
    assert!(h.state().app.room_camera_capture.edit.is_none());
    assert_eq!(
        h.state().app.room_camera.as_ref().unwrap().document(),
        &authored
    );
    click(&mut h, "Apply staged viewport camera");
    let captured = h
        .state()
        .app
        .room_camera
        .as_ref()
        .unwrap()
        .document()
        .clone();
    assert_ne!(captured, authored);
    lost_click(&mut h, "Undo captured camera");
    assert!(
        !h.state()
            .app
            .room_camera_capture
            .edit
            .as_ref()
            .unwrap()
            .undone
    );
    assert_eq!(
        h.state().app.room_camera.as_ref().unwrap().document(),
        &captured
    );
    click(&mut h, "Undo captured camera");
    assert!(
        h.state()
            .app
            .room_camera_capture
            .edit
            .as_ref()
            .unwrap()
            .undone
    );
    lost_click(&mut h, "Redo captured camera");
    assert!(
        h.state()
            .app
            .room_camera_capture
            .edit
            .as_ref()
            .unwrap()
            .undone
    );
    assert_eq!(
        h.state().app.room_camera.as_ref().unwrap().document(),
        &authored
    );
    click(&mut h, "Redo captured camera");
    assert_eq!(
        h.state().app.room_camera.as_ref().unwrap().document(),
        &captured
    );
    lost_click(&mut h, "Save captured camera");
    assert_eq!(fs::read(&camera_path).unwrap(), original);
    click(&mut h, "Save captured camera");
    assert_eq!(
        fs::read(&camera_path).unwrap(),
        captured.to_bytes().unwrap()
    );
    assert_eq!(
        h.state()
            .app
            .editor
            .snapshot()
            .unwrap()
            .predicted()
            .checksum(),
        frame
    );
    assert_eq!(h.state().app.editor.history(), &history);
}
#[test]
fn focus_loss_keeps_cancel_and_finish_available_without_authoring_or_saving() {
    let (temp, mut h) = setup();
    let path = temp.path().join("room/room.camera.json");
    let original = fs::read(&path).unwrap();
    h.state_mut().app.editor.camera3d.distance *= 1.25;
    click(&mut h, "Stage viewport camera");
    lost_click(&mut h, "Cancel staged viewport camera");
    assert!(!h.state().app.room_camera_capture.staged.busy());
    assert!(h.state().app.room_camera_capture.edit.is_none());
    click(&mut h, "Stage viewport camera");
    click(&mut h, "Apply staged viewport camera");
    assert!(h.state().app.room_camera_capture.edit.is_some());
    let document = h
        .state()
        .app
        .room_camera
        .as_ref()
        .unwrap()
        .document()
        .clone();
    lost_click(&mut h, "Finish captured camera history");
    assert!(h.state().app.room_camera_capture.edit.is_none());
    assert_eq!(
        h.state().app.room_camera.as_ref().unwrap().document(),
        &document
    );
    assert_eq!(fs::read(path).unwrap(), original);
}
