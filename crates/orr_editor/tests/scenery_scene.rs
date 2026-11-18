//! The settings of the Forest Valley window, applied to an editor scene.
//!
//! The scene is the component-built Forest Valley (a Yard3D scene). A setting
//! changed in the window must set the fields of the scene's components, the
//! change must be one undo step, and the viewport must draw the result.
#![cfg(all(feature = "scenery", feature = "outdoor"))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

mod common;

use common::*;
use egui::accesskit::{Role, Toggled};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use orr_editor::game::EditorGame;
use orr_editor::scenery_scene::{self, Change, Place};
use orr_editor::scenery_window::Target as WindowTarget;
use orr_editor::{Editor, EditorApp, Owner};
use orr_reflect::Value;
use orr_scenery::settings::{COLORS, SLIDERS, TOGGLES};
use orr_scenery::Settings;
use std::path::{Path, PathBuf};

fn forest_scene() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scenes/forest_valley/forest_valley.scene.yaml")
}

fn forest_editor() -> Editor {
    let mut editor = Editor::open_game(&forest_scene(), EditorGame::Yard3D).unwrap();
    editor.sync();
    editor
}

fn number(value: &Value) -> f32 {
    let Value::Fixed(f) = value else { panic!("a fixed-point number, got {value:?}") };
    f.to_f32()
}

fn singleton_number(editor: &mut Editor, name: &str, path: &str) -> f32 {
    number(&editor.read_field(&Owner::Singleton(name.into()), path).unwrap())
}

fn flag(value: &Value) -> bool {
    let Value::Bool(b) = value else { panic!("a flag, got {value:?}") };
    *b
}

/// Applies the control to the scene as the window does, and lets the editor take the answer in.
fn apply(editor: &mut Editor, group: &str, name: &str, settings: &Settings) -> usize {
    let changes = scenery_scene::changes(editor, group, name, settings);
    // A field that already has the value is not "changed" and is not an error.
    let accepted = scenery_scene::apply(editor, &changes);
    assert!(editor.status().is_none_or(|m| m.text.is_empty() || !m.error), "{group} {name}: the scene refused an edit");
    editor.sync();
    accepted
}

#[test]
fn a_game_without_the_components_cannot_be_edited_through_the_window() {
    let mut editor = demo_editor();
    assert!(!scenery_scene::available(&editor));
    assert!(scenery_scene::changes(&editor, "Sky", "Brightness", &Settings::default()).is_empty());
    // Reading leaves the settings as they are.
    assert_eq!(scenery_scene::read(&editor, Settings::default()), Settings::default());
    editor.sync();
}

#[test]
fn a_changed_setting_sets_the_fields_of_the_open_scene_and_undo_reverses_it() {
    let mut editor = forest_editor();
    assert!(scenery_scene::available(&editor), "the forest scene has the sun, sky and post process");
    let sun = target_named(&editor, "sun");
    let mut settings = scenery_scene::read(&editor, Settings::default());
    // What the scene says now shows in the controls.
    assert!((settings.sky.cloud_cover - 0.34).abs() < 1.0e-3, "cloud cover: {}", settings.sky.cloud_cover);
    assert!((settings.sun.intensity - 8.5).abs() < 1.0e-3, "sun intensity: {}", settings.sun.intensity);
    assert!((settings.fog.density - 1.0).abs() < 1.0e-3, "the scene's 0.7 is the window's 1: {}", settings.fog.density);
    assert!(settings.look.auto_exposure && settings.shadows.enabled);

    settings.sky.cloud_cover = 0.9;
    settings.sun.intensity = 3.0;
    settings.fog.density = 2.0;
    apply(&mut editor, "Sky", "Cloud cover", &settings);
    apply(&mut editor, "Sun", "Intensity", &settings);
    apply(&mut editor, "Fog", "Density", &settings);
    assert!((singleton_number(&mut editor, "orr_gfx::Sky", "cloud_cover") - 0.9).abs() < 1.0e-3);
    assert!((singleton_number(&mut editor, "orr_gfx::Sky", "fog_density") - 1.4).abs() < 1.0e-3, "fog density is 0.7 times the setting");
    assert!((number(&field(&mut editor, &sun, "orr_gfx::Light", "intensity")) - 3.0).abs() < 1.0e-3);

    // The change is one undo step per control.
    assert!(editor.undo());
    assert!((singleton_number(&mut editor, "orr_gfx::Sky", "fog_density") - 0.7).abs() < 1.0e-3);
    assert!(editor.undo());
    assert!((number(&field(&mut editor, &sun, "orr_gfx::Light", "intensity")) - 8.5).abs() < 1.0e-3);
}

#[test]
fn every_control_of_the_window_that_has_a_field_round_trips_through_the_scene() {
    let mut editor = forest_editor();
    let mut settings = Settings::default();
    // A value away from the scene's own, inside every range, for each slider.
    for slider in SLIDERS.iter().filter(|s| scenery_scene::applies(s.group, s.name)) {
        let value = slider.min + (slider.max - slider.min) * 0.37;
        (slider.set)(&mut settings, value);
    }
    for toggle in TOGGLES.iter().filter(|t| scenery_scene::applies(t.group, t.name)) {
        (toggle.set)(&mut settings, false);
    }
    for color in COLORS.iter().filter(|c| scenery_scene::applies(c.group, c.name)) {
        (color.set)(&mut settings, [0.1, 0.2, 0.3]);
    }
    let settings = settings.sanitized();
    let controls = SLIDERS.iter().map(|s| (s.group, s.name)).chain(TOGGLES.iter().map(|t| (t.group, t.name))).chain(COLORS.iter().map(|c| (c.group, c.name)));
    for (group, name) in controls.filter(|(g, n)| scenery_scene::applies(g, n)) {
        let changes = scenery_scene::changes(&editor, group, name, &settings);
        assert!(!changes.is_empty(), "{group} {name} sets nothing in the scene");
        apply(&mut editor, group, name, &settings);
    }
    let back = scenery_scene::read(&editor, Settings::default());
    for slider in SLIDERS.iter().filter(|s| scenery_scene::applies(s.group, s.name)) {
        let (want, got) = ((slider.get)(&settings), (slider.get)(&back));
        // The scene keeps four decimals. The hour comes back from the sun's direction.
        let tolerance = if slider.name == "Hour" { 0.02 } else { 2.0e-3 * want.abs().max(1.0) };
        assert!((want - got).abs() <= tolerance, "{} {}: set {want}, scene has {got}", slider.group, slider.name);
    }
    for toggle in TOGGLES {
        assert_eq!((toggle.get)(&back), (toggle.get)(&settings), "{}", toggle.name);
    }
    let (want, got) = ((COLORS[0].get)(&settings), (COLORS[0].get)(&back));
    assert!((0..3).all(|i| (want[i] - got[i]).abs() < 2.0e-3), "water color: set {want:?}, scene has {got:?}");
}

#[test]
fn a_post_effect_the_scene_lacks_is_added_in_a_free_slot_and_switched_by_its_strength() {
    let mut editor = forest_editor();
    let mut settings = scenery_scene::read(&editor, Settings::default());
    // Turn the vignette off, then on again after removing it from the scene's slots by kind.
    settings.look.vignette = 0.0;
    let changes = scenery_scene::changes(&editor, "Look", "Vignette", &settings);
    assert!(changes.iter().any(|c| c.path.ends_with("enabled") && c.value == Value::Bool(false)), "{changes:?}");
    apply(&mut editor, "Look", "Vignette", &settings);
    let back = scenery_scene::read(&editor, Settings::default());
    assert_eq!(back.look.vignette, 0.0);
    settings.look.vignette = 0.5;
    apply(&mut editor, "Look", "Vignette", &settings);
    let back = scenery_scene::read(&editor, Settings::default());
    assert!((back.look.vignette - 0.5).abs() < 1.0e-3);
}

/// The slot of the effect `kind` in the scene's post process, with its `enabled` flag.
fn effect_slot(editor: &mut Editor, kind: &str) -> Option<(usize, bool)> {
    (0..8).find_map(|slot| {
        let Value::Enum(k) = editor.read_field(&Owner::Singleton("orr_gfx::PostProcess".into()), &format!("effects[{slot}].kind")).ok()? else { return None };
        let enabled = flag(&editor.read_field(&Owner::Singleton("orr_gfx::PostProcess".into()), &format!("effects[{slot}].enabled")).ok()?);
        (k == kind).then_some((slot, enabled))
    })
}

fn color_grade_of_the_picture(editor: &Editor) -> Option<(f32, f32)> {
    let look = editor.yard_frame().look.as_ref()?;
    look.effects.iter().find_map(|e| match e {
        orr_render::ScreenEffect::ColorGrade { saturation, contrast, .. } => Some((*saturation, *contrast)),
        _ => None,
    })
}

#[test]
fn saturation_and_contrast_make_a_missing_color_grade_that_the_picture_uses() {
    let mut editor = forest_editor();
    let post = Owner::Singleton("orr_gfx::PostProcess".into());
    // A scene with the default post process: the grade is not in any slot.
    let (slot, _) = effect_slot(&mut editor, "color_grade").expect("the forest scene has a color grade");
    assert!(editor.set_field(&post, &format!("effects[{slot}].kind"), Value::Enum("none".into())));
    assert!(editor.set_field(&post, &format!("effects[{slot}].enabled"), Value::Bool(false)));
    editor.sync();
    assert!(effect_slot(&mut editor, "color_grade").is_none());
    assert!(color_grade_of_the_picture(&editor).is_none());

    let mut settings = scenery_scene::read(&editor, Settings::default());
    settings.look.saturation = 0.4;
    apply(&mut editor, "Look", "Saturation", &settings);
    let (_, enabled) = effect_slot(&mut editor, "color_grade").expect("the saturation control adds the grade");
    assert!(enabled, "a new color grade is switched on, as no control switches it");
    let (saturation, _) = color_grade_of_the_picture(&editor).expect("the renderer gets the grade");
    assert!((saturation - 0.4).abs() < 1.0e-3, "saturation in the picture: {saturation}");

    // The contrast control does the same for a grade that the scene keeps switched off.
    let (slot, _) = effect_slot(&mut editor, "color_grade").unwrap();
    assert!(editor.set_field(&post, &format!("effects[{slot}].enabled"), Value::Bool(false)));
    editor.sync();
    assert!(color_grade_of_the_picture(&editor).is_none());
    settings.look.contrast = 0.3;
    apply(&mut editor, "Look", "Contrast", &settings);
    let (saturation, contrast) = color_grade_of_the_picture(&editor).expect("a changed number switches the grade on");
    assert!((contrast - 0.3).abs() < 1.0e-3 && (saturation - 0.4).abs() < 1.0e-3, "grade {saturation} {contrast}");
}

#[test]
fn the_window_edits_the_open_scene_when_asked_and_dims_what_the_scene_lacks() {
    let mut h = Harness::builder().with_size([1500.0, 1000.0]).build_eframe(|_cc| EditorApp::new(forest_editor(), None));
    h.run_steps(3);
    let on = |h: &Harness<'_, EditorApp>, label: &str| h.query_by(|n| n.role() == Role::CheckBox && n.label().as_deref() == Some(label) && n.toggled() == Some(Toggled::True)).is_some();
    h.state_mut().scenery.open = true;
    h.run_steps(3);
    // The valley is the default target. A click on "Sun shadows" changes the window only.
    assert_eq!(h.state().scenery.target, WindowTarget::Valley);
    h.get_by_label("Edit the generated valley");
    h.get_by_label("Edit the open scene").click();
    h.run_steps(4);
    assert_eq!(h.state().scenery.target, WindowTarget::OpenScene);
    assert!(on(&h, "Sun shadows") && on(&h, "Auto exposure"), "the controls show the values of the scene");

    let sun = target_named(&h.state().editor, "sun");
    assert!(flag(&field(&mut h.state_mut().editor, &sun, "orr_gfx::Light", "cast_shadows")));
    h.get_by(|n| n.role() == Role::CheckBox && n.label().as_deref() == Some("Sun shadows")).click();
    for _ in 0..8 {
        h.state_mut().editor.sync();
        h.run_steps(2);
    }
    assert!(!flag(&field(&mut h.state_mut().editor, &sun, "orr_gfx::Light", "cast_shadows")), "the sun no longer casts shadows");
    assert!(!on(&h, "Sun shadows"));

    // A control with no field in the scene is dimmed: the click does not reach the scene.
    let foam = h.get_by(|n| n.role() == Role::Slider && n.label().as_deref() == Some("Foam"));
    assert!(foam.accesskit_node().is_disabled(), "foam has no field in an editor scene");
    let sky = h.get_by(|n| n.role() == Role::Slider && n.label().as_deref() == Some("Brightness"));
    assert!(!sky.accesskit_node().is_disabled());

    // The edit is in the editor's history and undo takes it back.
    assert!(h.state_mut().editor.undo());
    for _ in 0..4 {
        h.state_mut().editor.sync();
        h.run_steps(2);
    }
    assert!(flag(&field(&mut h.state_mut().editor, &sun, "orr_gfx::Light", "cast_shadows")));
}

#[test]
fn the_window_keeps_its_valley_for_a_game_without_the_components() {
    let mut h = Harness::builder().with_size([1500.0, 900.0]).build_eframe(|_cc| EditorApp::new(demo_editor(), None));
    h.run_steps(3);
    h.state_mut().scenery.open = true;
    h.state_mut().scenery.target = WindowTarget::OpenScene;
    h.run_steps(4);
    // The choice cannot hold: the demo scene has no sun, sky and post process.
    assert!(!h.state().scenery.edits_open_scene(&h.state().editor));
    h.get_by_label("The scene is not drawn.");
    assert!(h.query_by_label_contains("change only the generated valley").is_some());
}

/// Mean color of the viewport columns from `left` to the right edge and rows from `top` to `bottom` (0..1).
fn mean(width: u32, rgba: &[u8], rect: egui::Rect, left: f32, top: f32, bottom: f32) -> [f32; 3] {
    let (x0, x1) = ((rect.min.x + rect.width() * left) as u32, rect.max.x as u32);
    let (y0, y1) = ((rect.min.y + rect.height() * top) as u32, (rect.min.y + rect.height() * bottom) as u32);
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

#[test]
fn a_setting_changed_in_the_window_changes_the_picture_of_the_open_scene() {
    let Some((_guard, probe)) = gpu() else { return };
    if probe.is_software() {
        eprintln!("SKIP: this picture check needs a hardware adapter");
        return;
    }
    let setup = egui_wgpu::WgpuSetup::CreateNew(egui_wgpu::WgpuSetupCreateNew::without_display_handle());
    let mut h = Harness::builder().with_size([1600.0, 1000.0]).wgpu_setup(setup).build_eframe(|cc| EditorApp::new(forest_editor(), cc.wgpu_render_state.clone()));
    let frames = |h: &mut Harness<'_, EditorApp>, n: usize| {
        for _ in 0..n {
            h.state_mut().editor.sync();
            h.run_steps(2);
        }
    };
    frames(&mut h, 4);
    h.get_by_label("Camera view").click();
    frames(&mut h, 30);
    assert!(h.query_by_label_contains("Terrain:").is_none(), "the heightmaps load");

    let capture = std::env::var_os("ORR_SCENERY_CAPTURE").map(PathBuf::from);
    let rect = h.state().ui.viewport_rect.expect("the viewport was drawn");
    let before = h.render().expect("kittest wgpu render");
    if let Some(folder) = &capture {
        std::fs::create_dir_all(folder).unwrap();
        save_crop(&folder.join("scenery_forest_valley_window_before.png"), before.width(), before.as_raw(), rect);
    }

    // Frame time of the viewport with the window closed, then with the window editing the scene.
    let time = |h: &mut Harness<'_, EditorApp>| {
        let start = std::time::Instant::now();
        frames(h, 30);
        start.elapsed().as_secs_f64() * 1000.0 / 30.0
    };
    let closed = time(&mut h);
    h.state_mut().scenery.open = true;
    h.state_mut().scenery.target = WindowTarget::OpenScene;
    frames(&mut h, 4);
    let open = time(&mut h);
    eprintln!("viewport frame time (ms per sync and two steps): window closed {closed:.2}, window on the scene {open:.2}");

    // Evening light, a dark sky and no clouds, set through the window's own mapping.
    let mut settings = scenery_scene::read(&h.state().editor, Settings::default());
    settings.sun.hour = 7.0;
    settings.sky.brightness = 0.3;
    settings.look.exposure = 0.5;
    for (group, name) in [("Sun", "Hour"), ("Sky", "Brightness"), ("Look", "Exposure")] {
        apply(&mut h.state_mut().editor, group, name, &settings);
    }
    frames(&mut h, 30);
    let after = h.render().expect("kittest wgpu render");
    if let Some(folder) = &capture {
        save_crop(&folder.join("scenery_forest_valley_window_after.png"), after.width(), after.as_raw(), rect);
    }
    // The window covers the left of the viewport; compare the part to its right.
    let (a, b) = (mean(before.width(), before.as_raw(), rect, 0.4, 0.02, 0.9), mean(after.width(), after.as_raw(), rect, 0.4, 0.02, 0.9));
    let (a, b) = (a.iter().sum::<f32>() / 3.0, b.iter().sum::<f32>() / 3.0);
    assert!(b < a * 0.8, "the darker settings darken the picture: {a:.1} -> {b:.1}");
    // Nothing broke: the viewport still draws and the shader compiles.
    assert!(h.query_by_label_contains("Terrain:").is_none());
}

/// A change in the open scene is not a change of the window's own valley.
#[test]
fn a_change_outside_the_window_shows_after_the_controls_read_the_scene_again() {
    let mut editor = forest_editor();
    let sky = Change { place: Place::Singleton("orr_gfx::Sky"), path: "cloud_cover".into(), value: Value::Fixed(orr_fp::FP::from_ratio(1, 2)) };
    assert_eq!(scenery_scene::apply(&mut editor, &[sky]), 1);
    editor.sync();
    let read = scenery_scene::read(&editor, Settings::default());
    assert!((read.sky.cloud_cover - 0.5).abs() < 1.0e-3);
}

/// The forest scene with `extra` more water surfaces, in a temporary folder.
fn many_waters(extra: usize) -> (tempfile::TempDir, Editor) {
    let mut text = std::fs::read_to_string(forest_scene()).unwrap().replace("max_entities: 128", "max_entities: 1024");
    for i in 0..extra {
        text.push_str(&format!(
            "  e_{:08x}:\n    name: extra_pool_{i}\n    orr_gfx::Transform3:\n      position: [{}, 60, -300]\n      rotation: [0, 0, 0]\n      scale: [1, 1, 1]\n    orr_gfx::Water:\n      size: [20, 20]\n      color: [0.012, 0.05, 0.04]\n      clarity: 1\n      ripples: 1\n      flow: [0.3, 0.4]\n      glint: 1\n      terrain: e_00000001\n",
            0x1_0000 + i,
            i * 25
        ));
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("many_waters.scene.yaml");
    std::fs::write(&path, text).unwrap();
    let mut editor = Editor::open_game(&path, EditorGame::Yard3D).unwrap();
    editor.sync();
    (dir, editor)
}

fn water_values(editor: &Editor, f: impl Fn(&orr_gfx_view::WaterRef) -> f32) -> Vec<f32> {
    editor.yard_frame().waters.iter().map(f).collect()
}

#[test]
fn a_scene_with_a_hundred_waters_takes_a_water_control_as_one_undo_step() {
    let (_dir, mut editor) = many_waters(100);
    let count = editor.yard_frame().waters.len();
    assert!(count > 100, "{count} water surfaces");
    let mut settings = scenery_scene::read(&editor, Settings::default());
    settings.water.clarity = 2.5;
    settings.water.ripples = 0.4;
    settings.water.flow_speed = 1.5;
    let before_flow = water_values(&editor, |w| w.flow[0]);
    let checksum = editor.checksum();
    assert_eq!(apply(&mut editor, "Water", "Clarity", &settings), count, "every surface took the value");
    assert_eq!(apply(&mut editor, "Water", "Ripples", &settings), count);
    assert_eq!(apply(&mut editor, "Water", "Flow speed", &settings), count);
    assert!(water_values(&editor, |w| w.clarity).iter().all(|c| (c - 2.5).abs() < 1.0e-3));
    assert!(water_values(&editor, |w| w.ripples).iter().all(|r| (r - 0.4).abs() < 1.0e-3));
    assert_ne!(water_values(&editor, |w| w.flow[0]), before_flow, "the flow keeps each surface's direction but takes the new speed");

    // One control is one undo step for all the surfaces.
    for _ in 0..3 {
        assert!(editor.undo());
        editor.sync();
    }
    assert!(!editor.undo(), "nothing else is in the history");
    assert_eq!(editor.checksum(), checksum, "three undos restore every surface");
}

#[test]
fn a_drag_on_a_water_control_with_a_hundred_waters_is_one_undo_step_and_is_not_cancelled() {
    let (_dir, mut editor) = many_waters(100);
    let count = editor.yard_frame().waters.len();
    let checksum = editor.checksum();
    let mut settings = scenery_scene::read(&editor, Settings::default());
    // The window starts a drag, writes the control on every frame of it, and ends the drag.
    editor.begin_edit("scene window: Water Clarity");
    for step in 1..=5 {
        settings.water.clarity = 1.0 + step as f32 * 0.3;
        let changes = scenery_scene::changes(&editor, "Water", "Clarity", &settings);
        assert_eq!(changes.len(), count);
        scenery_scene::apply(&mut editor, &changes);
        editor.sync();
    }
    editor.end_edit();
    for _ in 0..40 {
        editor.sync();
        if !editor.in_gesture() {
            break;
        }
    }
    assert!(!editor.in_gesture());
    assert!(editor.status().is_none_or(|m| m.text.is_empty() || !m.error), "the drag was refused: {:?}", editor.status().map(|m| m.text.clone()));
    assert!(water_values(&editor, |w| w.clarity).iter().all(|c| (c - 2.5).abs() < 1.0e-3), "every surface has the last value of the drag");
    assert!(editor.undo());
    editor.sync();
    assert_eq!(editor.checksum(), checksum, "one undo reverses the drag on every surface");
    assert!(!editor.undo());
}
