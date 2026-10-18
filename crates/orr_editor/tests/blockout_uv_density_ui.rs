//! Opt-in, models-only production EditorApp acceptance. All recipe operations
//! use real widgets; GPU evidence distinguishes draft/recipe/immutable binding.
#![cfg(feature = "models")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

use egui::{Key, Modifiers, accesskit::Role};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use orr_editor::{Editor, EditorApp, blockout::Recipe, game::EditorGame};
use orr_package::{Project, Runtime};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

const TEST: &str = "density_widgets_preserve_draft_and_render_after_existing_bake";
const RECIPE: &str = "density.blockout.json";
const SIZE: [f32; 2] = [1800.0, 1500.0];

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn tree(root: &Path) -> BTreeMap<String, String> {
    fn visit(root: &Path, dir: &Path, out: &mut BTreeMap<String, String>) {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let kind = entry.file_type().unwrap();
            assert!(!kind.is_symlink());
            if kind.is_dir() {
                visit(root, &entry.path(), out);
            } else {
                out.insert(
                    entry
                        .path()
                        .strip_prefix(root)
                        .unwrap()
                        .display()
                        .to_string(),
                    digest(&fs::read(entry.path()).unwrap()),
                );
            }
        }
    }
    let mut files = BTreeMap::new();
    visit(root, root, &mut files);
    files
}
fn settle(h: &mut Harness<'_, EditorApp>) {
    for _ in 0..3 {
        h.state_mut().editor.sync();
        h.run_steps(2);
    }
    assert!(h.state().editor.down().is_none());
}
fn click(h: &mut Harness<'_, EditorApp>, label: &str) {
    h.get_by_label(label).scroll_to_me();
    h.run_steps(3);
    h.get_by_label(label).click();
    settle(h);
}
fn input(h: &mut Harness<'_, EditorApp>, label: &str, text: &str, role: Role) {
    h.get_by_role_and_label(role, label).scroll_to_me();
    h.run_steps(3);
    h.get_by_role_and_label(role, label).focus();
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run_steps(2);
    h.get_by_role_and_label(role, label).type_text(text);
    h.run_steps(2);
    h.key_press(Key::Enter);
    settle(h);
}
fn number(h: &mut Harness<'_, EditorApp>, label: &str, text: &str) {
    input(h, label, text, Role::SpinButton);
}
fn text(h: &mut Harness<'_, EditorApp>, label: &str, text: &str) {
    input(h, label, text, Role::TextInput);
}
fn recipe(h: &Harness<'_, EditorApp>) -> Recipe {
    h.state()
        .models
        .blockout
        .document
        .as_ref()
        .unwrap()
        .recipe()
        .clone()
}
fn history(h: &Harness<'_, EditorApp>) -> (usize, usize) {
    h.state()
        .models
        .blockout
        .document
        .as_ref()
        .unwrap()
        .history_len()
}
fn save_png(path: &Path, size: (u32, u32), pixels: &[u8]) {
    let mut encoder = png::Encoder::new(fs::File::create(path).unwrap(), size.0, size.1);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(pixels).unwrap();
    writer.finish().unwrap();
}
fn changed(a: &[u8], b: &[u8]) -> usize {
    assert_eq!(a.len(), b.len());
    a.as_chunks::<4>()
        .0
        .iter()
        .zip(b.as_chunks::<4>().0)
        .filter(|(a, b)| a != b)
        .count()
}
fn visible_controls(h: &mut Harness<'_, EditorApp>, output: &Path) -> Vec<u8> {
    // Keep the authoring controls visible in the complete composed EditorApp,
    // not just a cropped renderer readback. Static binding details are collapsed.
    if h.query_by_label("Load model asset").is_some() {
        click(h, "Static model binding");
    }
    if h.query_by_label("New box recipe").is_none() {
        click(h, "Box blockout");
    }
    if h.query_by_label("Calculate UV scale").is_none() {
        click(h, "Recipe-space UV density");
    }
    h.run_steps(20);
    settle(h);
    for label in [
        "New box recipe",
        "Apply face UV",
        "Save box recipe",
        "Model units per repeat",
        "Calculate UV scale",
        "Stage UV scale",
    ] {
        let rect = h.get_by_label(label).rect();
        assert!(
            h.ctx.viewport_rect().contains(rect.min) && h.ctx.viewport_rect().contains(rect.max),
            "{label} is clipped: {rect:?}"
        );
    }
    let image = h.render().expect("complete production editor composes");
    assert_eq!(
        (image.width(), image.height()),
        (SIZE[0] as u32, SIZE[1] as u32)
    );
    let pixels = image.into_raw();
    save_png(output, (SIZE[0] as u32, SIZE[1] as u32), &pixels);
    pixels
}
fn default_density_controls(h: &mut Harness<'_, EditorApp>, output: &Path) {
    if h.query_by_label("New box recipe").is_some() {
        click(h, "Box blockout");
    }
    if h.query_by_label("Load model asset").is_some() {
        click(h, "Static model binding");
    }
    // Existing component editors and the reserved Box panel have independent
    // width requirements. Collapse them through their real headers to isolate
    // the new helper's default-width usability without changing those panels.
    click(h, "orr_physics3d::Body");
    click(h, "orr_physics3d::Collider");
    h.run_steps(20);
    let id = egui::Id::new("inspector");
    let mut state = egui::containers::panel::PanelState::load(&h.ctx, id).unwrap();
    state.outer_rect.min.x = state.outer_rect.max.x - 340.0;
    h.ctx.data_mut(|data| data.insert_persisted(id, state));
    h.run_steps(20);
    settle(h);
    let inspector = egui::containers::panel::PanelState::load(&h.ctx, id)
        .unwrap()
        .outer_rect;
    assert_eq!(
        inspector.width(),
        340.0,
        "new density helper must fit default Inspector width"
    );
    for label in [
        "Model units per repeat",
        "Calculate UV scale",
        "Stage UV scale",
        "Scale staged in Face U/V scale. Use Apply face UV to update the recipe.",
    ] {
        let rect = h.get_by_label(label).rect();
        assert!(
            inspector.contains(rect.min) && inspector.contains(rect.max),
            "default-width control/status clipped: {label}: {rect:?}"
        );
    }
    let field = h
        .get_by_role_and_label(Role::TextInput, "Model units per repeat")
        .rect();
    assert!(inspector.contains(field.min) && inspector.contains(field.max));
    let image = h.render().unwrap();
    save_png(output, (image.width(), image.height()), &image.into_raw());
    click(h, "Box blockout");
}
fn fixed_view(h: &mut Harness<'_, EditorApp>, output: &Path, positive_x: bool) -> Vec<u8> {
    let box_open = h.query_by_label("New box recipe").is_some();
    let models_open = h.query_by_label("Load model asset").is_some();
    let density_open = h.query_by_label("Calculate UV scale").is_some();
    for (label, open) in [
        ("Box blockout", box_open),
        ("Static model binding", models_open),
        ("Recipe-space UV density", density_open),
    ] {
        if open {
            click(h, label);
        }
    }
    // Same remembered public panel geometry for each fixed-camera readback.
    h.run_steps(20);
    for (name, width) in [("inspector", 560.0), ("hierarchy", 230.0)] {
        let id = egui::Id::new(name);
        let mut state = egui::containers::panel::PanelState::load(&h.ctx, id).unwrap();
        if name == "inspector" {
            state.outer_rect.min.x = state.outer_rect.max.x - width;
        } else {
            state.outer_rect.max.x = state.outer_rect.min.x + width;
        }
        h.ctx.data_mut(|data| data.insert_persisted(id, state));
    }
    h.state_mut().editor.camera3d = orr_render::OrbitCamera::new(
        [2.5, 10.5, 1.125],
        if positive_x {
            std::f32::consts::FRAC_PI_2
        } else {
            0.0
        },
        0.0,
        10.0,
    );
    h.run_steps(20);
    settle(h);
    let size = h.state().ui.viewport_px;
    assert_eq!(size, (1010, 1383), "fixed editor layout changed");
    let pixels = h.state().viewport3d_gpu().unwrap().gpu().read_rgba8();
    assert!(
        pixels
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| *p != &pixels[..4])
            .count()
            > 1000,
        "blank production viewport"
    );
    save_png(output, size, &pixels);
    for (label, open) in [
        ("Box blockout", box_open),
        ("Static model binding", models_open),
        ("Recipe-space UV density", density_open),
    ] {
        if open {
            click(h, label);
        }
    }
    h.run_steps(20);
    settle(h);
    pixels
}
fn bake_assign(h: &mut Harness<'_, EditorApp>) -> String {
    click(h, "Save box recipe");
    click(h, "Bake immutable box");
    assert!(
        h.state().models.blockout.error().is_none(),
        "{:?}",
        h.state().models.blockout.error()
    );
    let package = h
        .state()
        .models
        .blockout
        .last_bake()
        .unwrap()
        .package
        .clone();
    if h.query_by_label("Load model asset").is_none() {
        click(h, "Static model binding");
    }
    click(h, "Load model asset");
    click(h, "Assign static model");
    assert!(
        h.state().models.error().is_none(),
        "{:?}",
        h.state().models.error()
    );
    click(h, "Save model bindings");
    click(h, "Static model binding");
    package
}
fn app(scene: &Path) -> (Harness<'static, EditorApp>, egui_wgpu::wgpu::AdapterInfo) {
    let mut editor = Editor::open_game(scene, EditorGame::Yard3D).unwrap();
    editor.sync();
    let info = std::sync::Arc::new(std::sync::Mutex::new(None));
    let captured = std::sync::Arc::clone(&info);
    let harness = Harness::builder()
        .with_size(SIZE)
        .with_step_dt(1.0 / 60.0)
        .wgpu()
        .build_eframe(move |cc| {
            *captured.lock().unwrap() =
                Some(cc.wgpu_render_state.as_ref().unwrap().adapter.get_info());
            EditorApp::new(editor, cc.wgpu_render_state.clone())
        });
    let info = info.lock().unwrap().take().unwrap();
    (harness, info)
}

#[test]
#[ignore = "explicit production EditorApp GPU acceptance; set ORR_REQUIRE_GPU=1 and a new ORR_UV_DENSITY_CAPTURE_DIR"]
fn density_widgets_preserve_draft_and_render_after_existing_bake() {
    assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
    let evidence =
        PathBuf::from(std::env::var_os("ORR_UV_DENSITY_CAPTURE_DIR").expect("capture directory"));
    assert!(!evidence.exists(), "use a new evidence directory");
    fs::create_dir_all(&evidence).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/imported_scene_demo")
        .canonicalize()
        .unwrap();
    Project::open_for_install(root, Runtime::content_only().engine_version)
        .unwrap()
        .install(&[source])
        .unwrap();
    let scene = root.join("yard.scene.yaml");
    let scene_bytes = include_bytes!("../../../scenes/yard3d_authoring.scene.yaml");
    fs::write(&scene, scene_bytes).unwrap();
    let (mut h, adapter_info) = app(&scene);
    settle(&mut h);
    assert!(h.state().has_gpu());
    let adapter = h.state().viewport3d_gpu().unwrap().gpu().adapter_name();
    assert_eq!(adapter, adapter_info.name);
    let software_gpu = adapter_info.device_type == egui_wgpu::wgpu::DeviceType::Cpu;
    click(&mut h, "box_right");
    click(&mut h, "Static model binding");
    click(&mut h, "Create model bindings");
    assert!(
        h.state().models.error().is_none(),
        "{:?}",
        h.state().models.error()
    );
    h.state_mut().models.transform.translation = [0.0, 6.0, 0.0];
    h.state_mut().models.transform.scale = [1.0, 1.5, 0.75];
    click(&mut h, "Static model binding");
    click(&mut h, "Box blockout");
    text(&mut h, "Box recipe path", RECIPE);
    click(&mut h, "New box recipe");
    click(&mut h, "Recipe-space UV density");
    visible_controls(
        &mut h,
        &evidence.join("01-full-editor-initial-controls.png"),
    );
    for (face, distance) in [("+X", "1"), ("+Y", "2"), ("+Z", "3")] {
        click(&mut h, face);
        number(&mut h, "Move face delta", distance);
        click(&mut h, "Apply face move");
    }
    click(&mut h, "+X");
    number(&mut h, "Face U offset", "0.125");
    number(&mut h, "Face V offset", "0.25");
    click(&mut h, "90 degrees");
    click(&mut h, "Apply face UV");
    click(&mut h, "Checker blue");
    click(&mut h, "Apply face material");
    let package_before = bake_assign(&mut h);
    let before = recipe(&h);
    assert_eq!(before.bounds.max, [1.5, 2.5, 3.5]);
    let before_x = fixed_view(&mut h, &evidence.join("02-before-positive-x.png"), true);
    let before_z = fixed_view(&mut h, &evidence.join("03-before-positive-z.png"), false);
    let authored_checksum = h.state().editor.checksum();
    let host_history = h.state().editor.history().clone();
    let before_bindings = h
        .state()
        .models
        .bindings
        .as_ref()
        .unwrap()
        .document()
        .clone();
    let before_files = tree(root);
    let before_history = history(&h);

    // A complete UV draft is pinned, including uncommitted offset/turn fields.
    text(&mut h, "Model units per repeat", "2");
    click(&mut h, "Calculate UV scale");
    number(&mut h, "Face V offset", "0.375");
    let stale_draft = h.state().models.blockout.uv;
    click(&mut h, "Stage UV scale");
    assert!(
        h.query_by_label("UV density context changed; calculate again before staging")
            .is_some()
    );
    assert_eq!(h.state().models.blockout.uv, stale_draft);
    assert_eq!(recipe(&h), before);
    number(&mut h, "Face V offset", "0.25");
    for invalid in ["0", "-2", "NaN", "inf", "0.00001", "1e30"] {
        let draft = h.state().models.blockout.uv;
        text(&mut h, "Model units per repeat", invalid);
        click(&mut h, "Calculate UV scale");
        assert!(
            h.get_by_label("Stage UV scale")
                .accesskit_node()
                .is_disabled()
        );
        assert_eq!(h.state().models.blockout.uv, draft);
        assert_eq!(recipe(&h), before);
    }
    text(&mut h, "Model units per repeat", "2");
    click(&mut h, "Calculate UV scale");
    assert_eq!(h.state().models.blockout.uv, before.faces[0].uv);
    click(&mut h, "Stage UV scale");
    let draft = h.state().models.blockout.uv;
    assert_eq!(draft.scale, [1.5, 2.0]);
    assert_eq!(draft.offset, before.faces[0].uv.offset);
    assert_eq!(draft.quarter_turn, 1);
    assert_eq!(recipe(&h), before);
    assert_eq!(history(&h), before_history);
    assert_eq!(
        h.state().models.bindings.as_ref().unwrap().document(),
        &before_bindings
    );
    assert_eq!(
        tree(root),
        before_files,
        "calculate/stage never writes files"
    );
    visible_controls(&mut h, &evidence.join("04-full-editor-staged-controls.png"));
    default_density_controls(
        &mut h,
        &evidence.join("04b-full-editor-default-width-density.png"),
    );
    assert_eq!(
        fixed_view(
            &mut h,
            &evidence.join("05-staged-positive-x-unchanged.png"),
            true
        ),
        before_x
    );

    click(&mut h, "Apply face UV");
    let applied = recipe(&h);
    assert_eq!(applied.faces[0].uv, draft);
    assert_eq!(applied.faces[0].material, before.faces[0].material);
    assert_eq!(applied.bounds, before.bounds);
    for i in 1..6 {
        assert_eq!(applied.faces[i], before.faces[i]);
    }
    assert_eq!(history(&h), (before_history.0 + 1, 0));
    visible_controls(&mut h, &evidence.join("06-full-editor-applied-preview.png"));
    assert_eq!(
        fixed_view(
            &mut h,
            &evidence.join("07-applied-immutable-positive-x-unchanged.png"),
            true
        ),
        before_x
    );
    click(&mut h, "Undo blockout");
    assert_eq!(recipe(&h), before);
    click(&mut h, "Redo blockout");
    assert_eq!(recipe(&h), applied);
    click(&mut h, "Save box recipe");
    let applied_bytes = fs::read(root.join(RECIPE)).unwrap();
    click(&mut h, "Open box recipe");
    assert_eq!(recipe(&h), applied);
    assert_eq!(history(&h), (0, 0));
    assert_eq!(fs::read(root.join(RECIPE)).unwrap(), applied_bytes);
    let package_after = bake_assign(&mut h);
    assert_ne!(package_before, package_after);
    let after_x = fixed_view(
        &mut h,
        &evidence.join("08-baked-assigned-positive-x.png"),
        true,
    );
    let after_z = fixed_view(
        &mut h,
        &evidence.join("09-baked-assigned-positive-z.png"),
        false,
    );
    let pixels_changed = changed(&before_x, &after_x);
    assert!(pixels_changed > 1000);
    assert_eq!(before_z, after_z, "other face pixels stay unchanged");
    let loaded_before =
        orr_editor::model_bindings::load_asset(root, &package_before, "box.model.json").unwrap();
    let loaded_after =
        orr_editor::model_bindings::load_asset(root, &package_after, "box.model.json").unwrap();
    let a = loaded_before.static_model().unwrap().source();
    let b = loaded_after.static_model().unwrap().source();
    assert_eq!(
        b.primitives[0]
            .vertices
            .iter()
            .map(|v| v.uv)
            .collect::<Vec<_>>(),
        [[0.125, 0.25], [0.125, 1.75], [-1.875, 1.75], [-1.875, 0.25]],
        "independent literal scale-before-quarter-turn oracle"
    );
    assert_eq!(&a.primitives[1..], &b.primitives[1..]);
    assert_eq!(a.images, b.images);
    assert_eq!(a.materials, b.materials);
    for (old, new) in a.primitives[0]
        .vertices
        .iter()
        .zip(&b.primitives[0].vertices)
    {
        assert_eq!(old.position, new.position);
        assert_eq!(old.normal, new.normal);
    }

    // Resizing never installs a hidden density constraint. The old scale remains
    // until an explicit fresh calculation, stage and existing apply action.
    click(&mut h, "+Y");
    number(&mut h, "Move face delta", "1");
    click(&mut h, "Apply face move");
    click(&mut h, "+X");
    assert_eq!(recipe(&h).faces[0].uv.scale, [1.5, 2.0]);
    assert_eq!(h.state().models.blockout.uv.scale, [1.5, 2.0]);
    click(&mut h, "Calculate UV scale");
    click(&mut h, "Stage UV scale");
    assert_eq!(h.state().models.blockout.uv.scale, [2.0, 2.0]);
    assert_eq!(recipe(&h).faces[0].uv.scale, [1.5, 2.0]);
    click(&mut h, "Apply face UV");
    assert_eq!(recipe(&h).faces[0].uv.scale, [2.0, 2.0]);
    let package_resized = bake_assign(&mut h);
    assert_ne!(package_after, package_resized);
    visible_controls(
        &mut h,
        &evidence.join("10-full-editor-resize-recalculated.png"),
    );
    assert_eq!(h.state().editor.checksum(), authored_checksum);
    assert_eq!(h.state().editor.history(), &host_history);
    assert_eq!(fs::read(&scene).unwrap(), scene_bytes);
    let final_recipe = recipe(&h);
    let final_files = tree(root);
    let before_play = h.state().models.blockout.uv;
    click(&mut h, orr_editor::app::LBL_PLAY);
    for label in [
        "Model units per repeat",
        "Calculate UV scale",
        "Stage UV scale",
    ] {
        assert!(
            h.get_by_label(label).accesskit_node().is_disabled(),
            "{label} must be disabled in Play"
        );
    }
    assert_eq!(h.state().models.blockout.uv, before_play);
    click(&mut h, orr_editor::app::LBL_STOP);
    assert_eq!(tree(root), final_files);
    drop(h);
    let (mut reopened, _) = app(&scene);
    settle(&mut reopened);
    click(&mut reopened, "box_right");
    click(&mut reopened, "Box blockout");
    text(&mut reopened, "Box recipe path", RECIPE);
    click(&mut reopened, "Open box recipe");
    assert_eq!(recipe(&reopened), final_recipe);
    assert_eq!(tree(root), final_files);
    let selected = reopened.state().editor.selected_guid().unwrap().to_string();
    assert_eq!(
        reopened
            .state()
            .models
            .bindings
            .as_ref()
            .unwrap()
            .document()
            .bindings[&selected]
            .package,
        package_resized
    );
    assert_eq!(reopened.state().editor.checksum(), authored_checksum);
    fs::write(evidence.join("acceptance-receipt.json"), serde_json::to_vec_pretty(&serde_json::json!({
        "test": TEST, "source_sha256": digest(include_bytes!("blockout_uv_density_ui.rs")),
        "features": ["models"], "adapter": adapter, "adapter_backend": format!("{:?}", adapter_info.backend), "adapter_device_type": format!("{:?}", adapter_info.device_type), "software_gpu": software_gpu, "physical_os_input": false, "input": "egui_kittest actual widgets", "full_editor_size": SIZE, "fixed_viewport_size": [1010,1383], "fixed_inspector_width": 560, "density_helper_default_width": 340,
        "before_recipe_sha256": before_files[RECIPE], "applied_recipe_sha256": digest(&applied_bytes),
        "final_recipe_sha256": final_files[RECIPE], "positive_x_changed_pixels": pixels_changed,
        "other_face_equal": before_z == after_z, "calculation": {"face":"+X","recipe_edges":[3,4],"model_units_per_repeat":2,"scale":[1.5,2],"quarter_turn":1,"binding_scale_excluded":[1,1.5,0.75]},
        "retained_packages": [package_before,package_after,package_resized],
        "boundary": "Calculate and Stage alter draft scale only; Apply changes the recipe preview/history; the main viewport changes only after existing Save/Bake/Load/Assign",
        "project_files": final_files
    })).unwrap()).unwrap();
    fs::write(
        evidence.join("capture-inventory.json"),
        serde_json::to_vec_pretty(&tree(&evidence)).unwrap(),
    )
    .unwrap();
}
