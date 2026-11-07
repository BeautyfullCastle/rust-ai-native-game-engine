//! CPU egui acceptance using the production input adapter and synthetic viewport.
//! No native window, OS pointer injection, rendered pixels or GPU is exercised.
use crate::{app::EditorApp, backend::HostSpec, editor::Editor};
use egui_kittest::{kittest::Queryable, Harness};
use std::path::{Path, PathBuf};

struct State {
    app: EditorApp,
    camera: orr_render::Camera3D,
    consumed: usize,
    pixels_per_point: f32,
}
const SIZE: (u32, u32) = (800, 600);
fn rect() -> egui::Rect {
    egui::Rect::from_min_size(egui::pos2(80.0, 90.0), egui::vec2(400.0, 300.0))
}
fn open(root: &Path) -> orr_sample::room_project::PreparedProject {
    orr_sample::room_project::PreparedProject::open_with_capabilities(
        root,
        orr_sample::room_project::RoomProjectCapabilities {
            lighting: true,
            character: false,
        },
    )
    .unwrap()
}
fn setup(pixels_per_point: f32) -> (tempfile::TempDir, PathBuf, Harness<'static, State>) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("fixed light Room");
    let mut light = orr_sample::room_lighting::Document::enabled_default();
    light.point_light.as_mut().unwrap().position = [-2.0, 5.5, 3.0];
    light.activation = Some(orr_sample::room_lighting::Activation::BeforeEscape);
    orr_sample::room_lit_starter::create(&root, "light-pick-acceptance", &light).unwrap();
    let mut prepared = open(&root);
    let lighting = prepared.take_lighting().unwrap();
    let camera = prepared.take_camera().unwrap();
    let (_, scene_path, scene, models) = prepared.into_parts();
    let mut editor = Editor::start(&HostSpec::PreparedRoom {
        scene: scene_path,
        text: scene.text().into(),
        listen: None,
        debug_hooks: false,
    })
    .unwrap();
    editor.sync();
    editor
        .install_room_lighting(lighting.document.clone())
        .unwrap();
    editor.install_room_camera(camera.document.clone()).unwrap();
    let panel = crate::room_lighting_panel::Panel::new_for_editor(lighting, &editor).unwrap();
    let projection = editor.presentation_camera3d(SIZE).unwrap();
    let mut app = EditorApp::new(editor, None);
    app.models.install_room(models).unwrap();
    app.room_camera = Some(crate::room_camera_panel::Panel::new(camera));
    app.room_lighting = Some(panel);
    let mut h = Harness::builder()
        .with_size([1200.0, 1200.0])
        .build_ui_state(
            |ui, state: &mut State| {
                ui.ctx().set_pixels_per_point(state.pixels_per_point);
                state.app.sync_room_light_pick(ui.ctx());
                let response = ui.allocate_rect(rect(), egui::Sense::click());
                if state
                    .app
                    .room_light_viewport_pick(ui.ctx(), &response, &state.camera, SIZE)
                {
                    state.consumed += 1;
                }
                egui::Window::new("Lighting authoring test")
                    .fixed_pos(egui::pos2(540.0, 20.0))
                    .show(ui.ctx(), |ui| {
                        let app = &mut state.app;
                        app.room_lighting
                            .as_mut()
                            .unwrap()
                            .show_for_editor(ui, &mut app.editor);
                    });
            },
            State {
                app,
                camera: projection,
                consumed: 0,
                pixels_per_point,
            },
        );
    h.run_steps(5);
    h.get_by_label("Room lighting").click_accesskit();
    h.run_steps(3);
    (temp, root, h)
}
fn click(h: &mut Harness<'_, State>, label: &str) {
    h.get_by_label(label).scroll_to_me();
    h.run_steps(20);
    h.get_by_label(label).click_accesskit();
    h.run_steps(3);
}
fn pointer(h: &mut Harness<'_, State>, pos: egui::Pos2) {
    h.event(egui::Event::PointerMoved(pos));
    h.event(egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    h.run_steps(1);
    h.event(egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    h.run_steps(3);
}
#[test]
fn offset_hidpi_viewport_pick_stages_then_explicit_apply_save_reopens_actual_room_light() {
    for pixels_per_point in [2.0, 1.25] {
        let (_temp, root, mut h) = setup(pixels_per_point);
        let before = h
            .state()
            .app
            .editor
            .room_lighting_document()
            .unwrap()
            .clone();
        let frame = h.state().app.editor.checksum();
        let scene = std::fs::read(root.join("room.scene.yaml")).unwrap();
        let saved = std::fs::read(root.join("room.lighting.json")).unwrap();
        click(&mut h, "Pick fixed light X/Z");
        assert!(h.state().app.room_lighting.as_ref().unwrap().pick_pending());
        let expected = [2.0, 0.0, -1.0];
        let pixel = h.state().camera.world_to_screen(expected, SIZE).unwrap();
        let point = rect().min + egui::vec2(pixel[0] / 2.0, pixel[1] / 2.0);
        assert!(rect().contains(point));
        pointer(&mut h, point);
        assert!(!h.state().app.room_lighting.as_ref().unwrap().pick_pending());
        assert!(h.state().consumed > 0);
        assert_eq!(h.state().app.editor.room_lighting_document(), Some(&before));
        assert_eq!(
            h.state().app.room_lighting.as_ref().unwrap().document(),
            &before
        );
        assert_eq!(
            std::fs::read(root.join("room.lighting.json")).unwrap(),
            saved
        );
        assert_eq!(h.state().app.editor.checksum(), frame);
        click(&mut h, "Apply lighting");
        let applied = h
            .state()
            .app
            .editor
            .room_lighting_document()
            .unwrap()
            .clone();
        let position = applied.point_light.unwrap().position;
        assert!((position[0] - expected[0]).abs() < 0.002, "{position:?}");
        assert!((position[2] - expected[2]).abs() < 0.002, "{position:?}");
        assert_eq!(position[1], before.point_light.unwrap().position[1]);
        let mut appearance = applied.clone();
        appearance.point_light.as_mut().unwrap().position = before.point_light.unwrap().position;
        assert_eq!(
            appearance, before,
            "pick only changes X/Z; preserves height, appearance, activation"
        );
        click(&mut h, "Undo lighting edit");
        assert_eq!(h.state().app.editor.room_lighting_document(), Some(&before));
        click(&mut h, "Redo lighting edit");
        assert_eq!(
            h.state().app.editor.room_lighting_document(),
            Some(&applied)
        );
        click(&mut h, "Save lighting");
        let reopened = open(&root);
        assert_eq!(reopened.lighting().unwrap().document, applied);
        assert_eq!(
            reopened
                .point_light_settings_for_frame(orr_bridge::FrameView::of(reopened.scene().frame()))
                .unwrap(),
            applied.settings()
        );
        assert_eq!(std::fs::read(root.join("room.scene.yaml")).unwrap(), scene);
        assert_eq!(h.state().app.editor.checksum(), frame);
    }
}

#[test]
fn outside_viewport_and_escape_cancel_leave_room_document_frame_and_files_unchanged() {
    let (_temp, root, mut h) = setup(2.0);
    let before = h
        .state()
        .app
        .editor
        .room_lighting_document()
        .unwrap()
        .clone();
    let frame = h.state().app.editor.checksum();
    let saved = std::fs::read(root.join("room.lighting.json")).unwrap();
    click(&mut h, "Pick fixed light X/Z");
    pointer(&mut h, egui::pos2(20.0, 500.0));
    assert!(h.state().app.room_lighting.as_ref().unwrap().pick_pending());
    assert_eq!(h.state().consumed, 0);
    let selected = h.state().app.editor.selected_guid();
    let camera = h.state().app.editor.presentation_camera3d(SIZE).unwrap();
    let pressed = rect().center();
    h.event(egui::Event::PointerMoved(pressed));
    h.event(egui::Event::PointerButton {
        pos: pressed,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    h.run_steps(1);
    assert!(h.state().app.room_light_pick_owns_input());
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(
        h.state().app.room_light_pick_owns_input(),
        "cancelled held press stays owned"
    );
    h.event(egui::Event::PointerButton {
        pos: pressed,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    h.run_steps(1);
    assert!(
        h.state().app.room_light_pick_owns_input(),
        "release frame stays owned"
    );
    h.run_steps(3);
    assert!(
        !h.state().app.room_light_pick_owns_input(),
        "quiet frame retires fence"
    );
    assert_eq!(h.state().app.editor.selected_guid(), selected);
    assert_eq!(
        h.state().app.editor.presentation_camera3d(SIZE).unwrap(),
        camera
    );
    assert!(!h.state().app.room_lighting.as_ref().unwrap().pick_pending());
    assert_eq!(h.state().app.editor.room_lighting_document(), Some(&before));
    assert_eq!(
        h.state().app.room_lighting.as_ref().unwrap().document(),
        &before
    );
    assert_eq!(h.state().app.editor.checksum(), frame);
    assert_eq!(
        std::fs::read(root.join("room.lighting.json")).unwrap(),
        saved
    );
}
