//! Scene lights in the editor, used the way a person uses them: "+ Light" adds
//! an entity with orr_gfx::Transform3 and orr_gfx::Light, the inspector edits
//! it, the scene file keeps it, undo removes it, and the viewport shows it.
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

mod common;

use common::*;
use egui::accesskit::Role;
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use orr_editor::editor::Owner;
use orr_editor::game::EditorGame;
use orr_editor::{Editor, EditorApp};
use orr_reflect::Value;
use std::path::{Path, PathBuf};

fn yard_scene(dir: &Path) -> PathBuf {
    let path = dir.join("yard.scene.yaml");
    std::fs::write(&path, include_str!("../../../scenes/yard3d_authoring.scene.yaml")).unwrap();
    path
}

fn yard_editor(scene: &Path) -> Editor {
    let mut editor = Editor::open_game(scene, EditorGame::Yard3D).unwrap();
    editor.sync();
    editor
}

fn settle(h: &mut Harness<'_, EditorApp>) {
    for _ in 0..3 {
        h.state_mut().editor.sync();
        h.run_steps(2);
    }
}

fn click(h: &mut Harness<'_, EditorApp>, label: &str) {
    h.get_by(|n| n.role() == Role::Button && n.label().as_deref() == Some(label)).click();
    settle(h);
}

fn light_rows(h: &Harness<'_, EditorApp>) -> Vec<String> {
    h.state().editor.rows().iter().filter(|row| row.components.iter().any(|c| c == "orr_gfx::Light")).filter_map(|row| row.name.clone()).collect()
}

#[test]
fn a_light_added_with_the_button_is_an_entity_with_components_in_the_scene_file() {
    let dir = tempfile::tempdir().unwrap();
    let scene = yard_scene(dir.path());
    let mut h = Harness::builder().with_size([1500.0, 900.0]).build_eframe(|_cc| EditorApp::new(yard_editor(&scene), None));
    settle(&mut h);
    assert!(h.state().editor.supports_scene_lights());
    assert!(light_rows(&h).is_empty());

    click(&mut h, "+ Light");
    assert_eq!(light_rows(&h), ["light_1"]);
    let row = h.state().editor.rows().iter().find(|row| row.name.as_deref() == Some("light_1")).unwrap().clone();
    assert_eq!(row.components, ["orr_gfx::Light", "orr_gfx::Transform3"]);
    // The new light is selected and the inspector shows its components.
    assert!(h.query_by_label("orr_gfx::Light").is_some() && h.query_by_label("orr_gfx::Transform3").is_some());
    assert!(h.query_by_label("Cast shadows").is_none(), "field names are the scene names");
    for field in ["color", "intensity", "range", "inner_angle", "outer_angle", "shadow_strength", "kind", "enabled", "cast_shadows", "layers"] {
        assert!(h.query_by_label(field).is_some(), "the inspector shows the field {field}");
    }

    // An inspector edit is one undo entry and lands in the saved scene file.
    let owner = Owner::Component("orr_gfx::Light".into());
    let editor = &mut h.state_mut().editor;
    assert!(editor.set_field(&owner, "kind", Value::Enum("spot".into())));
    assert!(editor.set_field(&owner, "cast_shadows", Value::Bool(true)));
    settle(&mut h);
    assert!(h.state_mut().editor.save());
    let text = std::fs::read_to_string(&scene).unwrap();
    for needle in ["name: light_1", "orr_gfx::Light:", "kind: spot", "cast_shadows: true", "orr_gfx::Transform3:", "position: [0, 4, 0]"] {
        assert!(text.contains(needle), "the scene file lacks {needle:?}");
    }

    // The saved file opens again with the light, and undo of the whole add removes it.
    let mut again = Harness::builder().with_size([1500.0, 900.0]).build_eframe(|_cc| EditorApp::new(yard_editor(&scene), None));
    settle(&mut again);
    assert_eq!(light_rows(&again), ["light_1"]);
    for _ in 0..3 {
        assert!(h.state_mut().editor.undo());
        settle(&mut h);
    }
    assert!(light_rows(&h).is_empty(), "undo removed the light entity");
}

#[test]
fn the_button_is_offered_only_for_games_with_scene_lights() {
    // The 2D physics demo has no orr_gfx types.
    let mut h = Harness::builder().with_size([1500.0, 900.0]).build_eframe(|_cc| EditorApp::new(demo_editor(), None));
    settle(&mut h);
    assert!(!h.state().editor.supports_scene_lights());
    assert!(h.query_by_label("+ Light").is_none());
}

/// Mean brightness of the viewport in a rendered UI image.
fn viewport_brightness(h: &mut Harness<'_, EditorApp>) -> f32 {
    let image = h.render().expect("kittest wgpu render");
    let rect = h.state().ui.viewport_rect.expect("the viewport was drawn");
    let (mut sum, mut count) = (0.0, 0.0);
    for y in (rect.min.y as u32 + 4..rect.max.y as u32 - 4).step_by(3) {
        for x in (rect.min.x as u32 + 4..rect.max.x as u32 - 4).step_by(3) {
            let p = image.get_pixel(x, y).0;
            sum += f32::from(p[0]) + f32::from(p[1]) + f32::from(p[2]);
            count += 3.0;
        }
    }
    sum / count
}

#[test]
fn the_viewport_follows_the_scene_lights() {
    let Some((_guard, probe)) = gpu() else { return };
    if probe.is_software() {
        eprintln!("SKIP: this pixel check needs a hardware adapter");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let scene = yard_scene(dir.path());
    let setup = egui_wgpu::WgpuSetup::CreateNew(egui_wgpu::WgpuSetupCreateNew::without_display_handle());
    let mut h = Harness::builder()
        .with_size([1400.0, 850.0])
        .wgpu_setup(setup)
        .build_eframe(|cc| EditorApp::new(yard_editor(&scene), cc.wgpu_render_state.clone()));
    settle(&mut h);
    let lit_by_the_game = viewport_brightness(&mut h);

    // A light entity replaces the game's sun: one dim point light leaves the yard darker.
    click(&mut h, "+ Light");
    let owner = Owner::Component("orr_gfx::Light".into());
    h.state_mut().editor.set_field(&owner, "intensity", Value::Fixed(orr_fp::FP::from_ratio(1, 10)));
    settle(&mut h);
    let dim = viewport_brightness(&mut h);
    assert!(dim < lit_by_the_game * 0.8, "game sun {lit_by_the_game}, one dim light {dim}");

    // The same point light, bright and with a long range, lights the yard around it.
    let editor = &mut h.state_mut().editor;
    editor.set_field(&owner, "intensity", Value::Fixed(orr_fp::FP::from_int(20)));
    editor.set_field(&owner, "range", Value::Fixed(orr_fp::FP::from_int(40)));
    settle(&mut h);
    let lamp = viewport_brightness(&mut h);
    assert!(lamp > dim * 1.3, "one dim light {dim}, one bright point light {lamp}");

    // The same entity as a bright directional sun lights the yard again.
    let editor = &mut h.state_mut().editor;
    editor.set_field(&owner, "kind", Value::Enum("directional".into()));
    editor.set_field(&owner, "intensity", Value::Fixed(orr_fp::FP::from_int(3)));
    editor.set_field(&Owner::Component("orr_gfx::Transform3".into()), "rotation", Value::Vec3(orr_fp::FPVec3::new(orr_fp::FP::from_int(60), orr_fp::FP::from_int(30), orr_fp::FP::ZERO)));
    settle(&mut h);
    let sunny = viewport_brightness(&mut h);
    assert!(sunny > dim * 1.5, "one dim light {dim}, bright sun {sunny}");

    // Turned off, the light gives nothing: the scene has lights, so no game sun comes back.
    h.state_mut().editor.set_field(&owner, "enabled", Value::Bool(false));
    settle(&mut h);
    let off = viewport_brightness(&mut h);
    assert!(off < sunny * 0.8, "bright sun {sunny}, light off {off}");
}
