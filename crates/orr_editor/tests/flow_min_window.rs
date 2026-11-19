//! The smallest window (`flow::MIN_WINDOW_SIZE`) keeps the main controls inside
//! it in the layouts that add the most to the top bar and the panels: the demo
//! scene with every feature, the Yard3D scene, and a generated Room project.
//!
//! Run it with `--features full`. Screenshots are written only when
//! `ORR_FLOW_SHOTS` names a file prefix, for example `C:\tmp\flow_min_`.
#![allow(clippy::disallowed_types)]
#![allow(clippy::float_arithmetic)]

mod common;

use common::*;
use egui::{Rect, Vec2};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use orr_editor::app::{LBL_PAUSE, LBL_PLAY, LBL_STEP, LBL_STOP};
use orr_editor::editor::Mode;
use orr_editor::game::EditorGame;
use orr_editor::{Editor, EditorApp};
use orr_fp::{FPVec3, FP};

const MIN: [f32; 2] = orr_editor::flow::MIN_WINDOW_SIZE;

/// Every main control that must stay visible, with a scene that has unsaved changes.
const CONTROLS: [&str; 8] = [LBL_PLAY, LBL_PAUSE, LBL_STEP, LBL_STOP, "EDIT", "Unsaved changes", "Inspector", "File"];

fn shots() -> Option<std::ffi::OsString> {
    std::env::var_os("ORR_FLOW_SHOTS").filter(|_| common::gpu().is_some())
}

fn settle(h: &mut Harness<'_, EditorApp>) {
    for _ in 0..3 {
        h.state_mut().editor.sync();
        h.run_steps(2);
    }
}

fn app_with(editor: Editor, configure: impl FnOnce(&mut EditorApp) + 'static) -> Harness<'static, EditorApp> {
    let builder = Harness::builder().with_size(MIN).with_step_dt(1.0 / 60.0);
    if shots().is_some() {
        builder.wgpu().build_eframe(move |cc| {
            let mut app = EditorApp::new(editor, cc.wgpu_render_state.clone());
            configure(&mut app);
            app
        })
    } else {
        builder.build_eframe(move |_cc| {
            let mut app = EditorApp::new(editor, None);
            configure(&mut app);
            app
        })
    }
}

fn shot(h: &mut Harness<'_, EditorApp>, name: &str) {
    let Some(prefix) = shots() else { return };
    h.run_steps(2);
    let image = h.render().expect("render the editor window");
    let mut path = prefix;
    path.push(format!("{name}.png"));
    image.save(&path).expect("write the screenshot");
}

fn assert_inside(h: &Harness<'_, EditorApp>, labels: &[&str], what: &str) {
    let window = Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(MIN[0], MIN[1]));
    for label in labels {
        let rects = h.query_all_by_label(label).map(|n| n.rect()).collect::<Vec<_>>();
        assert!(!rects.is_empty(), "{what}: no control labelled {label:?}");
        for rect in rects {
            assert!(rect.width() > 1.0 && rect.height() > 1.0, "{what}: {label:?} has no size: {rect:?}");
            assert!(window.contains_rect(rect), "{what}: {label:?} is clipped by the window: {rect:?} outside {window:?}");
        }
    }
    let status = h.query_all_by_label_contains("Unsaved: ").map(|n| n.rect()).collect::<Vec<_>>();
    assert!(!status.is_empty(), "{what}: no unsaved count in the status bar");
    for rect in status {
        assert!(window.contains_rect(rect), "{what}: the status bar text is clipped: {rect:?}");
    }
}

/// The checks of one layout: edit mode with an unsaved change, then play mode.
fn check_layout(h: &mut Harness<'_, EditorApp>, name: &str) {
    h.run_steps(3);
    settle(h);
    assert!(h.state().editor.is_dirty(), "{name}: the scene must have an unsaved change");
    assert_inside(h, &CONTROLS, name);
    let viewport = h.state().ui.viewport_rect.expect("viewport");
    assert!(viewport.width() >= 300.0 && viewport.height() >= 200.0, "{name}: viewport {viewport:?}");
    shot(h, &format!("{name}_edit"));
    h.get_by_label(LBL_PLAY).click();
    let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while h.state().editor.mode() != Mode::Play {
        assert!(std::time::Instant::now() < end, "{name}: timed out waiting for play mode");
        h.step();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    settle(h);
    assert_inside(h, &["PLAY", LBL_STOP, "Inspector", "File"], &format!("{name} (play)"));
    shot(h, &format!("{name}_play"));
}

#[test]
fn demo_scene_with_every_feature_fits_the_smallest_window() {
    let mut editor = demo_editor();
    assert!(editor.spawn_body([0.0, 3.0]), "{:?}", editor.status());
    let mut h = app_with(editor, |_| {});
    check_layout(&mut h, "demo");
}

#[test]
fn yard3d_fits_the_smallest_window() {
    let path = temp_path("flow-min-yard3d.scene.yaml");
    std::fs::write(&path, include_str!("../../../scenes/yard3d_authoring.scene.yaml")).unwrap();
    let mut editor = Editor::open_game(&path, EditorGame::Yard3D).expect("open Yard3D");
    editor.sync();
    assert!(editor.spawn_yard_body(FPVec3::new(FP::ZERO, FP::from_int(2), FP::ZERO)), "{:?}", editor.status());
    let mut h = app_with(editor, |_| {});
    check_layout(&mut h, "yard3d");
}

#[cfg(all(feature = "room-project", feature = "project-create"))]
#[test]
fn room_project_fits_the_smallest_window() {
    use orr_editor::HostSpec;
    use orr_sample::project_create::{create, CreateOptions};
    use orr_sample::room_project::PreparedProject;

    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("flow-min-room");
    create(&CreateOptions { output: root.clone(), template: "room-escape-3d-v1".into(), seed: "flow-min-window".into() }).unwrap();
    let mut prepared = PreparedProject::open(&root).unwrap();
    let camera = prepared.take_camera().expect("the template declares a camera");
    let (_, path, scene, models) = prepared.into_parts();
    let mut editor = Editor::start(&HostSpec::PreparedRoom { scene: path, text: scene.text().into(), listen: None, debug_hooks: false }).unwrap();
    editor.sync();
    assert_eq!(editor.game(), EditorGame::RoomEscape);
    editor.install_room_camera(camera.document.clone()).unwrap();
    // Room actors cannot be added; moving the first one makes the scene unsaved.
    let guid = editor.rows().iter().find_map(|row| row.guid.clone()).expect("a Room actor");
    editor.select(Some(orr_editor::Target::Guid(guid)));
    let moved = FPVec3::new(FP::from_int(1), FP::ONE / FP::from_int(2), FP::from_int(1));
    let identity = [FP::ZERO, FP::ZERO, FP::ZERO, FP::ONE];
    assert!(editor.set_yard_transform(moved, identity), "{:?}", editor.status());
    let mut h = app_with(editor, move |app| {
        app.models.install_room(models).unwrap();
        app.room_camera = Some(orr_editor::room_camera_panel::Panel::new(camera));
    });
    check_layout(&mut h, "room");
}
