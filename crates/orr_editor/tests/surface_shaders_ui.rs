//! Surface shaders in the editor, used the way a person uses them: put a
//! `.wgsl` file in the scene's `shaders` folder, give an entity an
//! orr_gfx::Material, pick the shader in the inspector, set its parameters by
//! name, break the file and fix it again while the editor runs.
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

mod common;

use common::*;
use egui::accesskit::Role;
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use orr_editor::game::EditorGame;
use orr_editor::{Editor, EditorApp};
use std::path::{Path, PathBuf};

const STRIPES: &str = include_str!("../../../scenes/shaders/stripes.wgsl");
const SOLID: &str = "// @param tint color = 1, 0, 0\nfn surface(in: SurfaceInput) -> Surface { var s = default_surface(in); s.albedo = params.tint; s.roughness = 1.0; return s; }\n";

fn project(dir: &Path) -> PathBuf {
    let scene = dir.join("yard.scene.yaml");
    std::fs::write(&scene, include_str!("../../../scenes/yard3d_authoring.scene.yaml")).unwrap();
    std::fs::create_dir(dir.join("shaders")).unwrap();
    std::fs::write(dir.join("shaders").join("stripes.wgsl"), STRIPES).unwrap();
    std::fs::write(dir.join("shaders").join("solid.wgsl"), SOLID).unwrap();
    scene
}

fn editor(scene: &Path) -> Editor {
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

/// Gives the entity `name` a material from the inspector's "+ Add component" path.
fn add_material(h: &mut Harness<'_, EditorApp>, name: &str) {
    let target = target_named(&h.state().editor, name);
    h.state_mut().editor.select(Some(target));
    settle(h);
    assert!(h.state_mut().editor.add_component("orr_gfx::Material"));
    settle(h);
}

/// The material section's shader list, showing `current`.
fn shader_list<'a>(h: &'a Harness<'_, EditorApp>, current: &'a str) -> egui_kittest::Node<'a> {
    h.get_by(move |n| n.role() == Role::ComboBox && n.value().as_deref() == Some(current))
}

/// Picks `shader` in the material section's shader list.
fn pick_shader(h: &mut Harness<'_, EditorApp>, current: &str, shader: &str) {
    h.get_by(|n| n.role() == Role::ComboBox && n.value().as_deref() == Some(current)).click();
    h.run_steps(2);
    h.get_by(|n| n.label().as_deref() == Some(shader) && n.role() != Role::ComboBox).click();
    settle(h);
}

#[test]
fn a_shader_picked_in_the_inspector_shows_its_named_parameters_and_is_saved() {
    let dir = tempfile::tempdir().unwrap();
    let scene = project(dir.path());
    let mut h = Harness::builder().with_size([1500.0, 1000.0]).build_eframe(|_cc| EditorApp::new(editor(&scene), None));
    settle(&mut h);
    add_material(&mut h, "box_left");
    h.get_by_label("orr_gfx::Material");
    shader_list(&h, "(none)");

    pick_shader(&mut h, "(none)", "stripes");
    for name in ["tint", "stripes", "sharpness"] {
        assert!(h.query_by_label(name).is_some(), "the parameter {name} is listed by its name");
    }
    h.get_by_label_contains("3 parameters");
    assert!(h.state_mut().editor.save());
    let text = std::fs::read_to_string(&scene).unwrap();
    assert!(text.contains("orr_gfx::Material:") && text.contains("shader: stripes"), "{text}");

    // Undo brings back the material without a shader.
    assert!(h.state_mut().editor.undo());
    settle(&mut h);
    shader_list(&h, "(none)");
}

#[test]
fn a_broken_shader_file_shows_its_line_and_a_fix_is_picked_up_while_the_editor_runs() {
    let dir = tempfile::tempdir().unwrap();
    let scene = project(dir.path());
    let mut h = Harness::builder().with_size([1500.0, 1000.0]).build_eframe(|_cc| EditorApp::new(editor(&scene), None));
    settle(&mut h);
    add_material(&mut h, "box_left");
    pick_shader(&mut h, "(none)", "solid");
    h.get_by_label("tint");

    std::fs::write(dir.path().join("shaders").join("solid.wgsl"), "fn surface(in: SurfaceInput) -> Surface {\n    return 1.0;\n}\n").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(600));
    settle(&mut h);
    h.get_by_label_contains("shaders/solid.wgsl line 2");
    h.get_by_label_contains("drawn magenta");

    std::fs::write(dir.path().join("shaders").join("solid.wgsl"), SOLID).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(600));
    settle(&mut h);
    assert!(h.query_by_label_contains("drawn magenta").is_none());
    h.get_by_label("tint");
}

/// Number of pixels in the viewport close to `rgb`.
fn count_near(h: &mut Harness<'_, EditorApp>, rgb: [u8; 3]) -> usize {
    let image = h.render().expect("kittest wgpu render");
    let rect = h.state().ui.viewport_rect.expect("the viewport was drawn");
    let mut count = 0;
    for y in rect.min.y as u32..rect.max.y as u32 {
        for x in rect.min.x as u32..rect.max.x as u32 {
            let p = image.get_pixel(x, y).0;
            count += usize::from((0..3).all(|c| p[c].abs_diff(rgb[c]) <= 40));
        }
    }
    count
}

#[test]
fn the_viewport_draws_the_shape_with_its_shader_and_magenta_when_the_shader_is_broken() {
    let Some((_guard, probe)) = gpu() else { return };
    if probe.is_software() {
        eprintln!("SKIP: this pixel check needs a hardware adapter");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let scene = project(dir.path());
    let setup = egui_wgpu::WgpuSetup::CreateNew(egui_wgpu::WgpuSetupCreateNew::without_display_handle());
    let mut h = Harness::builder()
        .with_size([1400.0, 900.0])
        .wgpu_setup(setup)
        .build_eframe(|cc| EditorApp::new(editor(&scene), cc.wgpu_render_state.clone()));
    settle(&mut h);
    let magenta = [255, 0, 255];
    let red = [200, 20, 20];
    let red_before = count_near(&mut h, red);
    assert_eq!(count_near(&mut h, magenta), 0);

    // The solid shader paints the box red; the rest of the yard keeps the built-in material.
    add_material(&mut h, "box_left");
    assert_eq!(count_near(&mut h, magenta), 0, "a material without a shader keeps the built-in look");
    pick_shader(&mut h, "(none)", "solid");
    settle(&mut h);
    let red_after = count_near(&mut h, red);
    assert!(red_after > red_before + 200, "the shader colored the box: {red_before} red pixels before, {red_after} after");
    let path = temp_path("surface_shader_viewport.png");
    h.render().unwrap().save(&path).unwrap();
    eprintln!("viewport image: {}", path.display());

    // A broken file turns the same box magenta.
    std::fs::write(dir.path().join("shaders").join("solid.wgsl"), "fn surface(in: SurfaceInput) -> Surface { return 1.0; }").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(600));
    settle(&mut h);
    assert!(count_near(&mut h, magenta) > 200, "a broken shader shows magenta");
    assert!(count_near(&mut h, red) < red_after / 2);
}

/// The same material on an entity drawn as a model: the model is drawn by the
/// shader, and magenta when the shader file is broken.
#[cfg(feature = "models")]
#[test]
fn a_model_entity_with_a_material_is_drawn_by_its_shader() {
    let Some((_guard, probe)) = gpu() else { return };
    if probe.is_software() {
        eprintln!("SKIP: this pixel check needs a hardware adapter");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let scene = project(dir.path());
    let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    orr_package::Project::open_for_install(dir.path(), orr_package::Runtime::content_only().engine_version)
        .unwrap()
        .install(&[assets.join("imported_scene_demo").canonicalize().unwrap()])
        .unwrap();
    let setup = egui_wgpu::WgpuSetup::CreateNew(egui_wgpu::WgpuSetupCreateNew::without_display_handle());
    let mut h = Harness::builder()
        .with_size([1400.0, 900.0])
        .wgpu_setup(setup)
        .build_eframe(|cc| EditorApp::new(editor(&scene), cc.wgpu_render_state.clone()));
    settle(&mut h);
    {
        let app = h.state_mut();
        app.models.open_for_editor(&app.editor, true).unwrap();
        assert!(app.editor.select_named("box_left"));
        app.models.kind = orr_editor::model_bindings::ModelKind::Static;
        app.models.package = "sample-imported-scene".into();
        app.models.asset = "foreground.glb".into();
        app.models.assign(&app.editor).unwrap();
    }
    settle(&mut h);
    let magenta = [255, 0, 255];
    let red = [200, 20, 20];
    let red_before = count_near(&mut h, red);

    add_material(&mut h, "box_left");
    pick_shader(&mut h, "(none)", "solid");
    settle(&mut h);
    let red_after = count_near(&mut h, red);
    assert!(red_after > red_before + 200, "the shader colored the model: {red_before} red pixels before, {red_after} after");
    let path = temp_path("surface_shader_model_viewport.png");
    h.render().unwrap().save(&path).unwrap();
    eprintln!("viewport image: {}", path.display());

    std::fs::write(dir.path().join("shaders").join("solid.wgsl"), "fn surface(in: SurfaceInput) -> Surface { return 1.0; }").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(600));
    settle(&mut h);
    assert!(count_near(&mut h, magenta) > 200, "a broken shader shows the model magenta");
    assert!(count_near(&mut h, red) < red_after / 2);
}
