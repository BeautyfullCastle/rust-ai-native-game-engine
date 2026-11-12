//! The Forest Valley scene window, used the way a person uses it: opened from
//! the menu, settings changed with its widgets, presets saved to and loaded
//! from a file, and the scene drawn on a GPU.
#![cfg(feature = "scenery")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

mod common;

use common::*;
use egui::accesskit::{Role, Toggled};
use egui::{Key, Modifiers};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use orr_editor::EditorApp;
use orr_scenery::{Preset, Settings};

const NO_GPU_NOTICE: &str = "A GPU is required to draw the Forest Valley scene. The settings can still be edited, saved and loaded.";
const DEFAULT_FILE: &str = "forest_valley.preset.json";

fn editor_without_gpu() -> Harness<'static, EditorApp> {
    let mut h = Harness::builder().with_size([1500.0, 900.0]).build_eframe(|_cc| EditorApp::new(demo_editor(), None));
    h.run_steps(3);
    h
}

fn open_window(h: &mut Harness<'_, EditorApp>) {
    assert!(h.query_by_label("Forest Valley scene").is_none(), "the window starts closed");
    h.get_by(|n| n.role() == Role::Button && n.label().as_deref() == Some("Edit")).click();
    h.run_steps(3);
    h.get_by_label("Forest Valley scene\u{2026}").click();
    h.run_steps(4);
    assert!(h.state().scenery.open);
}

/// Scrolls the settings list to the button, as a person does, then clicks it.
fn click_button(h: &mut Harness<'_, EditorApp>, label: &str) {
    h.get_by(|n| n.role() == Role::Button && n.label().as_deref() == Some(label)).scroll_to_me();
    h.run_steps(3);
    h.get_by(|n| n.role() == Role::Button && n.label().as_deref() == Some(label)).click();
    h.run_steps(3);
}

fn checked(h: &Harness<'_, EditorApp>, label: &str) -> bool {
    // The checkbox must exist; then ask whether it is the checked one.
    h.get_by(|n| n.role() == Role::CheckBox && n.label().as_deref() == Some(label));
    h.query_by(|n| n.role() == Role::CheckBox && n.label().as_deref() == Some(label) && n.toggled() == Some(Toggled::True)).is_some()
}

/// Scrolls the settings list to the checkbox, as a person does, then clicks it.
fn click_checkbox(h: &mut Harness<'_, EditorApp>, label: &str) {
    h.get_by(|n| n.role() == Role::CheckBox && n.label().as_deref() == Some(label)).scroll_to_me();
    h.run_steps(3);
    h.get_by(|n| n.role() == Role::CheckBox && n.label().as_deref() == Some(label)).click();
    h.run_steps(3);
}

/// Replaces the text of the field that shows `current`.
fn type_into(h: &mut Harness<'_, EditorApp>, current: &str, text: &str) {
    h.get_by(|n| n.role() == Role::TextInput && n.value().as_deref() == Some(current)).focus();
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run_steps(2);
    if text.is_empty() {
        h.key_press(Key::Backspace);
    } else {
        h.get_by(|n| n.role() == Role::TextInput && n.value().as_deref() == Some(current)).type_text(text);
    }
    h.run_steps(3);
}

#[test]
fn the_window_opens_from_the_edit_menu_and_lists_every_setting_without_a_gpu() {
    let mut h = editor_without_gpu();
    open_window(&mut h);
    h.get_by_label(NO_GPU_NOTICE);
    h.get_by_label("The scene is not drawn.");
    // Every group, toggle and slider of the renderer has a control.
    for group in orr_scenery::settings::GROUPS {
        assert!(h.query_all_by_label(group).next().is_some(), "group {group}");
    }
    for toggle in orr_scenery::settings::TOGGLES {
        assert_eq!(checked(&h, toggle.name), (toggle.get)(&Settings::default()), "toggle {}", toggle.name);
    }
    assert_eq!(h.query_all_by_role(Role::Slider).count(), orr_scenery::settings::SLIDERS.len(), "one slider per setting");

    // A changed setting is still there after the window was closed and opened again.
    click_checkbox(&mut h, "Sun shadows");
    assert!(!checked(&h, "Sun shadows"));
    h.state_mut().scenery.open = false;
    h.run_steps(3);
    assert!(h.query_by_label(NO_GPU_NOTICE).is_none());
    h.state_mut().scenery.open = true;
    h.run_steps(3);
    assert!(!checked(&h, "Sun shadows"));
}

#[test]
fn reset_returns_changed_settings_to_their_defaults() {
    let mut h = editor_without_gpu();
    open_window(&mut h);
    click_checkbox(&mut h, "Sun shadows");
    click_checkbox(&mut h, "Auto exposure");
    assert!(!checked(&h, "Sun shadows") && !checked(&h, "Auto exposure"));
    click_button(&mut h, "Reset settings");
    assert!(checked(&h, "Sun shadows") && checked(&h, "Auto exposure"));
    h.get_by_label("All settings are back to their defaults.");
}

#[test]
fn a_preset_is_saved_to_its_file_and_loaded_back() {
    let mut h = editor_without_gpu();
    open_window(&mut h);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("no shadows.preset.json");
    let file = path.display().to_string();
    type_into(&mut h, DEFAULT_FILE, &file);

    click_checkbox(&mut h, "Sun shadows");
    click_button(&mut h, "Save preset");
    h.get_by_label(format!("Saved {file}").as_str());
    let mut expected = Preset::default();
    expected.settings.shadows.enabled = false;
    assert_eq!(Preset::from_json(&std::fs::read_to_string(&path).unwrap()), Ok(expected));
    // The folder holds the preset only: no temporary file is left.
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);

    click_button(&mut h, "Reset settings");
    assert!(checked(&h, "Sun shadows"));
    click_button(&mut h, "Load preset");
    h.get_by_label(format!("Loaded {file}").as_str());
    assert!(!checked(&h, "Sun shadows"), "the loaded preset turned sun shadows off again");

    // A preset written by hand loads too, and missing values take their defaults.
    std::fs::write(&path, r#"{ "settings": { "look": { "auto_exposure": false } } }"#).unwrap();
    click_button(&mut h, "Load preset");
    assert!(checked(&h, "Sun shadows") && !checked(&h, "Auto exposure"));

    // A broken file and a file that is not a preset change nothing and say why.
    for broken in [r#"{ "settings": { "sun": { "hour": "late" } } }"#, r#"{ "sun": { "hour": 9.0 } }"#, "[1, 2"] {
        std::fs::write(&path, broken).unwrap();
        click_button(&mut h, "Load preset");
        assert!(h.query_by_label_contains("Not loaded: scene preset:").is_some(), "after loading {broken}");
        assert!(checked(&h, "Sun shadows") && !checked(&h, "Auto exposure"), "after loading {broken}");
    }
    std::fs::remove_file(&path).unwrap();
    click_button(&mut h, "Load preset");
    assert!(h.query_by_label_contains("Not loaded:").is_some());
    assert!(!checked(&h, "Auto exposure"));

    type_into(&mut h, &file, "");
    click_button(&mut h, "Save preset");
    h.get_by_label("Not saved: Enter a file name first.");
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}

/// Number of distinct colors in the part of the UI image where the scene view is.
/// `pixels` holds RGBA rows from the top.
fn view_colors(pixels: &[u8], width: u32, height: u32) -> usize {
    let (x0, x1, y0, y1) = (width / 8, width / 2, height / 4, height * 3 / 4);
    let mut colors = Vec::new();
    for y in y0..y1 {
        for x in x0..x1 {
            let p = &pixels[((y * width + x) * 4) as usize..][..4];
            colors.push([p[0] >> 3, p[1] >> 3, p[2] >> 3]);
        }
    }
    colors.sort_unstable();
    colors.dedup();
    colors.len()
}

/// Share of the scene view where two UI images differ clearly.
fn view_difference(a: &[u8], b: &[u8], width: u32, height: u32) -> f32 {
    let (x0, x1, y0, y1) = (width / 8, width / 2, height / 4, height * 3 / 4);
    let mut different = 0;
    for y in y0..y1 {
        for x in x0..x1 {
            let at = ((y * width + x) * 4) as usize;
            different += u32::from((0..3).any(|channel| a[at + channel].abs_diff(b[at + channel]) > 24));
        }
    }
    different as f32 / ((x1 - x0) * (y1 - y0)) as f32
}

#[test]
fn the_scene_is_drawn_in_the_window_and_follows_its_controls() {
    let Some((_guard, probe)) = gpu() else { return };
    if probe.is_software() {
        // The scene has millions of triangles: a software adapter needs seconds for one frame.
        eprintln!("SKIP: the Forest Valley scene needs a hardware adapter");
        return;
    }
    // The default harness setup prefers a software adapter. This one takes the hardware adapter.
    let setup = egui_wgpu::WgpuSetup::CreateNew(egui_wgpu::WgpuSetupCreateNew::without_display_handle());
    let mut h = Harness::builder()
        .with_size([1500.0, 900.0])
        .wgpu_setup(setup)
        .build_eframe(|cc| EditorApp::new(demo_editor(), cc.wgpu_render_state.clone()));
    h.run_steps(3);
    open_window(&mut h);
    h.run_steps(3);
    assert!(h.query_by_label(NO_GPU_NOTICE).is_none());
    assert!(h.query_by_label_contains("million triangles").is_some(), "the status line reports the drawn frame");
    // Still pictures can be compared.
    click_checkbox(&mut h, "Pause motion");
    let first = h.render().expect("kittest wgpu render");
    let (width, height) = (first.width(), first.height());
    let path = temp_path("forest_valley_window.png");
    first.save(&path).expect("save the window image");
    eprintln!("window image: {}", path.display());
    assert!(view_colors(first.as_raw(), width, height) > 400, "the view shows a detailed picture");

    // Without sun shadows the shaded ground and water are lit.
    click_checkbox(&mut h, "Sun shadows");
    let flat = h.render().expect("render");
    assert!(view_difference(first.as_raw(), flat.as_raw(), width, height) > 0.03, "sun shadows off changed the picture");
    click_checkbox(&mut h, "Sun shadows");

    // A viewpoint button moves the camera.
    click_button(&mut h, "5");
    let aerial = h.render().expect("render");
    assert!(view_difference(first.as_raw(), aerial.as_raw(), width, height) > 0.5, "viewpoint 5 shows another picture");
    assert!(view_colors(aerial.as_raw(), width, height) > 400);

    // Closing the window releases the scene; opening it again draws it again.
    h.state_mut().scenery.open = false;
    h.run_steps(3);
    assert!(h.query_by_label_contains("million triangles").is_none());
    h.state_mut().scenery.open = true;
    h.run_steps(4);
    assert!(h.query_by_label_contains("million triangles").is_some());
    let reopened = h.render().expect("render");
    assert!(view_colors(reopened.as_raw(), width, height) > 400);
}
