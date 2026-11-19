//! The settings of the Forest Valley window, applied to a navigation document.
//!
//! The document is the checked-in `navigation_point.scene.yaml` (a `NavigationYard3D`
//! scene with a sun light). A setting changed in the window must set the fields
//! of the document's components, the change must be one undo step, the route
//! must stay admitted, and the viewport must draw the result.
#![cfg(all(feature = "scenery", feature = "outdoor", feature = "navigation"))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

mod common;

use common::*;
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use orr_editor::game::EditorGame;
use orr_editor::scenery_scene;
use orr_editor::scenery_window::Target as WindowTarget;
use orr_editor::{Editor, EditorApp, Owner};
use orr_reflect::Value;
use orr_scenery::Settings;
use std::path::{Path, PathBuf};

/// A scene directory with the navigation terrain asset and the checked-in scene (or `yaml`).
fn fixture(root: &Path, yaml: &str) -> PathBuf {
    std::fs::create_dir(root.join("navigation")).unwrap();
    std::fs::write(root.join("navigation/point_demo.orrt"), include_bytes!("../../../scenes/navigation/point_demo.orrt")).unwrap();
    let scene = root.join("navigation.scene.yaml");
    std::fs::write(&scene, yaml).unwrap();
    scene
}

const SCENE: &str = include_str!("../../../scenes/navigation_point.scene.yaml");

/// The scene as it was saved before the document had render components: no entity at all.
fn old_scene() -> String {
    let at = SCENE.find("entities:
").expect("the scene has an entities map");
    format!("{}entities: {{}}
", &SCENE[..at])
}

fn navigation_editor(root: &Path, yaml: &str) -> Editor {
    let mut editor = Editor::open_game(&fixture(root, yaml), EditorGame::NavigationYard3D).unwrap();
    editor.sync();
    // The camera looks at the terrain of the point demo, as the navigation viewport tests do.
    editor.camera3d = orr_render::OrbitCamera::new([0.5, 0.1, 0.5], 0.45, 0.75, 3.8);
    editor
}

fn number(value: &Value) -> f32 {
    let Value::Fixed(f) = value else { panic!("a fixed-point number, got {value:?}") };
    f.to_f32()
}

fn singleton_number(editor: &mut Editor, name: &str, path: &str) -> f32 {
    number(&editor.read_field(&Owner::Singleton(name.into()), path).unwrap())
}

/// Applies the control to the scene as the window does, and lets the editor take the answer in.
fn apply(editor: &mut Editor, group: &str, name: &str, settings: &Settings) -> usize {
    let changes = scenery_scene::changes(editor, group, name, settings);
    let accepted = scenery_scene::apply(editor, &changes);
    assert!(editor.status().is_none_or(|m| m.text.is_empty() || !m.error), "{group} {name}: the scene refused an edit");
    editor.sync();
    accepted
}

#[test]
fn the_navigation_document_has_the_components_the_window_edits() {
    let root = tempfile::tempdir().unwrap();
    let editor = navigation_editor(root.path(), SCENE);
    assert!(editor.admitted_navigation().admitted, "the route is admitted");
    assert!(scenery_scene::available(&editor));
    let settings = scenery_scene::read(&editor, Settings::default());
    assert!((settings.sun.intensity - 2.1).abs() < 1.0e-3, "sun intensity: {}", settings.sun.intensity);
    assert!(settings.shadows.enabled);
    // The sun is a light, not a body: the document draws no shape.
    assert!(editor.yard_frame().items.is_empty());
    assert!(editor.yard_frame().look.as_ref().is_some_and(|look| look.lights.iter().any(|light| light.sun)));
}

#[test]
fn a_document_saved_before_the_components_still_opens_with_default_values() {
    let root = tempfile::tempdir().unwrap();
    let mut editor = navigation_editor(root.path(), &old_scene());
    assert!(editor.admitted_navigation().admitted);
    assert!(scenery_scene::available(&editor), "the singletons come with their default values");
    let sky = singleton_number(&mut editor, "orr_gfx::Sky", "brightness");
    assert!((sky - 1.5).abs() < 1.0e-3);
    // Without a sun light the window has nothing to turn: its Hour sets no field.
    assert!(scenery_scene::changes(&editor, "Sun", "Hour", &Settings::default()).is_empty());
    // The sky, the environment and the exposure still can be edited.
    let mut settings = scenery_scene::read(&editor, Settings::default());
    settings.look.exposure = 2.0;
    assert_eq!(apply(&mut editor, "Look", "Exposure", &settings), 1);
    assert!((singleton_number(&mut editor, "orr_gfx::PostProcess", "exposure") - 2.0).abs() < 1.0e-3);
}

#[test]
fn a_changed_setting_sets_the_fields_of_the_navigation_document_and_undo_reverses_it() {
    let root = tempfile::tempdir().unwrap();
    let mut editor = navigation_editor(root.path(), SCENE);
    let sun = target_named(&editor, "sun");
    let mut settings = scenery_scene::read(&editor, Settings::default());
    settings.sun.intensity = 3.0;
    settings.sky.cloud_cover = 0.9;
    settings.look.exposure = 1.7;
    settings.sun.hour = 8.0;
    let checksum = editor.checksum();
    for (group, name) in [("Sun", "Intensity"), ("Sky", "Cloud cover"), ("Look", "Exposure"), ("Sun", "Hour")] {
        assert!(apply(&mut editor, group, name, &settings) > 0, "{group} {name} changed nothing");
    }
    assert!((number(&field(&mut editor, &sun, "orr_gfx::Light", "intensity")) - 3.0).abs() < 1.0e-3);
    assert!((singleton_number(&mut editor, "orr_gfx::Sky", "cloud_cover") - 0.9).abs() < 1.0e-3);
    assert!((singleton_number(&mut editor, "orr_gfx::PostProcess", "exposure") - 1.7).abs() < 1.0e-3);
    let back = scenery_scene::read(&editor, Settings::default());
    assert!((back.sun.hour - 8.0).abs() < 0.05, "hour: {}", back.sun.hour);
    assert!(editor.admitted_navigation().admitted, "the route stays admitted after the edits");
    assert_ne!(editor.checksum(), checksum);

    // One undo step per control, in reverse order.
    for _ in 0..4 {
        assert!(editor.undo());
        editor.sync();
    }
    assert!(!editor.undo(), "nothing else was recorded");
    assert!((number(&field(&mut editor, &sun, "orr_gfx::Light", "intensity")) - 2.1).abs() < 1.0e-3);
    assert!((singleton_number(&mut editor, "orr_gfx::PostProcess", "exposure") - 1.0).abs() < 1.0e-3);
    assert_eq!(editor.checksum(), checksum, "undo restores the whole frame");
}

#[test]
fn the_window_edits_the_navigation_document_when_asked() {
    let root = tempfile::tempdir().unwrap();
    let editor = navigation_editor(root.path(), SCENE);
    let mut h = Harness::builder().with_size([1500.0, 1000.0]).build_eframe(move |_cc| EditorApp::new(editor, None));
    h.run_steps(3);
    h.state_mut().scenery.open = true;
    h.run_steps(3);
    h.get_by_label("Edit the open scene").click();
    h.run_steps(4);
    assert_eq!(h.state().scenery.target, WindowTarget::OpenScene);
    assert!(h.state().scenery.edits_open_scene(&h.state().editor), "the navigation document is editable through the window");
}

/// Mean color of the viewport area from `x0` to `x1` (columns) and `y0` to `y1` (rows), each 0..1 of the viewport.
fn mean(width: u32, rgba: &[u8], rect: egui::Rect, [x0, x1]: [f32; 2], [y0, y1]: [f32; 2]) -> [f32; 3] {
    let (x0, x1) = ((rect.min.x + rect.width() * x0) as u32, (rect.min.x + rect.width() * x1) as u32);
    let (y0, y1) = ((rect.min.y + rect.height() * y0) as u32, (rect.min.y + rect.height() * y1) as u32);
    let mut sum = [0.0f32; 3];
    let mut n = 0.0;
    for y in y0..y1 {
        for x in x0..x1 {
            let i = ((y * width + x) * 4) as usize;
            for c in 0..3 {
                sum[c] += f32::from(rgba[i + c]);
            }
            n += 1.0;
        }
    }
    sum.map(|s| s / n)
}

/// Where the terrain of the point demo is in the viewport, away from the window on the left.
const TERRAIN: ([f32; 2], [f32; 2]) = ([0.38, 0.62], [0.42, 0.62]);

/// Saves `rgba` (`width` pixels per row) cropped to `rect` as a PNG.
fn save_crop(path: &Path, width: u32, rgba: &[u8], rect: egui::Rect) {
    let (x0, y0, w, h) = (rect.min.x as u32, rect.min.y as u32, rect.width() as u32, rect.height() as u32);
    let mut data = Vec::with_capacity((w * h * 4) as usize);
    for y in y0..y0 + h {
        let start = ((y * width + x0) * 4) as usize;
        data.extend_from_slice(&rgba[start..start + (w * 4) as usize]);
    }
    let file = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
    let mut encoder = png::Encoder::new(file, w, h);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().unwrap().write_image_data(&data).unwrap();
}

fn frames(h: &mut Harness<'_, EditorApp>, n: usize) {
    for _ in 0..n {
        h.state_mut().editor.sync();
        h.run_steps(2);
    }
}

#[test]
fn the_navigation_document_looks_as_before_and_the_window_changes_its_picture() {
    let Some((_guard, probe)) = gpu() else { return };
    if probe.is_software() {
        eprintln!("SKIP: this picture check needs a hardware adapter");
        return;
    }
    let capture = std::env::var_os("ORR_SCENERY_CAPTURE").map(PathBuf::from);
    let setup = || egui_wgpu::WgpuSetup::CreateNew(egui_wgpu::WgpuSetupCreateNew::without_display_handle());
    // The picture of a document, with the window closed.
    let picture = |yaml: &str, name: &str| {
        let root = tempfile::tempdir().unwrap();
        let editor = navigation_editor(root.path(), yaml);
        let mut h = Harness::builder().with_size([1600.0, 1000.0]).wgpu_setup(setup()).build_eframe(move |cc| EditorApp::new(editor, cc.wgpu_render_state.clone()));
        frames(&mut h, 30);
        let rect = h.state().ui.viewport_rect.expect("the viewport was drawn");
        let image = h.render().expect("kittest wgpu render");
        if let Some(folder) = &capture {
            std::fs::create_dir_all(folder).unwrap();
            save_crop(&folder.join(name), image.width(), image.as_raw(), rect);
        }
        mean(image.width(), image.as_raw(), rect, TERRAIN.0, TERRAIN.1)
    };
    // The document as saved before it had components, and the document with the sun entity.
    let old = picture(&old_scene(), "navigation_doc_before.png");
    let new = picture(SCENE, "navigation_doc_after.png");
    for c in 0..3 {
        assert!((old[c] - new[c]).abs() < 4.0, "the default sun keeps the look: {old:?} vs {new:?}");
    }

    // The window changes the hour and the exposure of the document.
    let root = tempfile::tempdir().unwrap();
    let editor = navigation_editor(root.path(), SCENE);
    let mut h = Harness::builder().with_size([1600.0, 1000.0]).wgpu_setup(setup()).build_eframe(move |cc| EditorApp::new(editor, cc.wgpu_render_state.clone()));
    frames(&mut h, 30);
    h.state_mut().scenery.open = true;
    h.state_mut().scenery.target = WindowTarget::OpenScene;
    frames(&mut h, 4);
    let rect = h.state().ui.viewport_rect.expect("the viewport was drawn");
    let before = h.render().expect("kittest wgpu render");
    let mut settings = scenery_scene::read(&h.state().editor, Settings::default());
    settings.sun.hour = 7.0;
    settings.look.exposure = 0.5;
    for (group, name) in [("Sun", "Hour"), ("Look", "Exposure")] {
        apply(&mut h.state_mut().editor, group, name, &settings);
    }
    frames(&mut h, 30);
    let after = h.render().expect("kittest wgpu render");
    if let Some(folder) = &capture {
        save_crop(&folder.join("navigation_doc_window_before.png"), before.width(), before.as_raw(), rect);
        save_crop(&folder.join("navigation_doc_window_after.png"), after.width(), after.as_raw(), rect);
    }
    let (a, b) = (mean(before.width(), before.as_raw(), rect, TERRAIN.0, TERRAIN.1), mean(after.width(), after.as_raw(), rect, TERRAIN.0, TERRAIN.1));
    let (a, b) = (a.iter().sum::<f32>() / 3.0, b.iter().sum::<f32>() / 3.0);
    assert!(b < a * 0.8, "the darker settings darken the picture: {a:.1} -> {b:.1}");
    assert!(h.state().editor.admitted_navigation().admitted);
}
