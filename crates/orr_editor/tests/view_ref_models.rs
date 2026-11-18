//! The editor viewport draws the model of an entity's `orr_gfx::ViewRef`: the demo scene
//! `scenes/view_yard3d` (the `ViewYard3D` game) has a pawn with a view asset and one without. Picking
//! a view asset for the second pawn in the inspector puts the id in the document, and the viewport
//! then draws that model. With `ORR_SHOT_DIR` set, the GPU test also saves the editor as a picture.
#![cfg(feature = "models")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

use std::path::{Path, PathBuf};

use egui::accesskit::Role;
use egui_kittest::{kittest::Queryable, Harness};
use orr_editor::{game::EditorGame, Editor, EditorApp};
use orr_reflect::asset_id;
use orr_rhi::{Wgpu, WgpuOptions};

/// The demo scene and its assets, copied to a new folder.
fn demo_scene(root: &Path) -> PathBuf {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scenes/view_yard3d");
    for entry in std::fs::read_dir(&source).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), root.join(entry.file_name())).unwrap();
    }
    root.join("view_yard3d.scene.yaml")
}

fn editor(scene: &Path) -> Editor {
    let mut editor = Editor::open_game(scene, EditorGame::ViewYard3D).unwrap();
    editor.sync();
    editor
}

fn settle(h: &mut Harness<'_, EditorApp>) {
    for _ in 0..3 {
        h.state_mut().editor.sync();
        h.run_steps(2);
    }
    assert!(h.state().editor.down().is_none(), "the local host stays connected");
}

fn placements(h: &mut Harness<'_, EditorApp>) -> Vec<(String, [f32; 3])> {
    let names: Vec<(orr_ecs::Entity, String)> = h.state().editor.rows().iter().map(|r| (r.entity, r.label())).collect();
    let mut found: Vec<(String, [f32; 3])> = h
        .state_mut()
        .view_ref_placements(&[], 0)
        .iter()
        .map(|p| (format!("{} -> {}", names.iter().find(|(e, _)| *e == p.entity).unwrap().1, p.model.source().asset_id), p.instance.translation))
        .collect();
    found.sort_by(|a, b| a.0.cmp(&b.0));
    found
}

/// Selects the red pawn and picks the red view asset in the inspector's `ViewRef` field.
fn pick_red_asset(h: &mut Harness<'_, EditorApp>) {
    h.get_by_label("red_pawn").click();
    settle(h);
    let combo = h.query_all_by_role(Role::ComboBox).find(|c| c.value().as_deref() == Some("(none)")).expect("the ViewRef field shows (none)");
    combo.click();
    h.run_steps(2);
    h.get_by_label("red").click();
    settle(h);
}

#[test]
fn picking_a_view_asset_in_the_inspector_makes_the_viewport_draw_its_model() {
    let temp = tempfile::tempdir().unwrap();
    let scene = demo_scene(temp.path());
    let editor = editor(&scene);
    let mut h = Harness::builder().with_size([1500.0, 1000.0]).with_step_dt(1.0 / 60.0).build_eframe(move |_| EditorApp::new(editor, None));
    settle(&mut h);

    // The blue pawn names a view asset; the red one names none and shows only its marker box.
    assert_eq!(placements(&mut h), vec![("blue_pawn -> blue.glb".to_string(), [-1.5, 0.0, 0.0])]);
    assert_eq!(h.state().editor.yard_frame().view_refs.len(), 2);
    assert_eq!(h.state().editor.yard_frame().items.len(), 3, "the ground and a marker box for each pawn");

    let history = h.state().editor.history().entries.len();
    pick_red_asset(&mut h);

    let id = asset_id("red.orrview");
    let red = h.state().editor.yard_frame().view_refs.iter().find(|p| p.asset == id).expect("the document holds the id of the picked asset");
    assert_eq!(red.translation, [1.5, 0.0, 0.0]);
    assert_eq!(h.state().editor.history().entries.len(), history + 1, "one pick is one edit");
    assert_eq!(
        placements(&mut h),
        vec![("blue_pawn -> blue.glb".to_string(), [-1.5, 0.0, 0.0]), ("red_pawn -> red.glb".to_string(), [1.5, 0.0, 0.0])],
        "the viewport draws the model of the picked asset"
    );
    assert!(h.state().view_models.problem().is_none(), "{:?}", h.state().view_models.problem());
}

#[test]
fn an_id_that_names_no_view_asset_draws_no_model_and_says_so() {
    let temp = tempfile::tempdir().unwrap();
    let scene = demo_scene(temp.path());
    std::fs::remove_file(temp.path().join("blue.orrview")).unwrap();
    let editor = editor(&scene);
    let mut h = Harness::builder().with_size([1500.0, 1000.0]).build_eframe(move |_| EditorApp::new(editor, None));
    settle(&mut h);
    assert!(placements(&mut h).is_empty());
    let problem = h.state().view_models.problem().expect("the missing asset is reported");
    assert!(problem.contains("no .orrview file has the id"), "{problem}");
}

fn gpu_available() -> bool {
    match Wgpu::headless(WgpuOptions::default()) {
        Ok(_) => true,
        Err(error) => {
            assert!(std::env::var_os("ORR_REQUIRE_GPU").is_none(), "ORR_REQUIRE_GPU is set but the GPU is unavailable: {error}");
            eprintln!("SKIP: no GPU: {error}");
            false
        }
    }
}

#[test]
fn the_gpu_viewport_changes_when_a_view_asset_is_picked_and_can_be_saved_as_a_picture() {
    if !gpu_available() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let scene = demo_scene(temp.path());
    let editor = editor(&scene);
    let mut h = Harness::builder()
        .with_size([1500.0, 1000.0])
        .with_step_dt(1.0 / 60.0)
        .wgpu()
        .build_eframe(move |cc| EditorApp::new(editor, cc.wgpu_render_state.clone()));
    settle(&mut h);
    assert!(h.state().has_gpu());
    let size = h.state().ui.viewport_px;
    let before = h.state().viewport3d_gpu().expect("the main viewport rendered").gpu().read_rgba8();
    assert_eq!(before.len(), (size.0 * size.1 * 4) as usize);

    pick_red_asset(&mut h);
    assert_eq!(h.state().ui.viewport_px, size);
    let after = h.state().viewport3d_gpu().unwrap().gpu().read_rgba8();
    let changed = before.as_chunks::<4>().0.iter().zip(after.as_chunks::<4>().0).filter(|(a, b)| a != b).count();
    assert!(changed > 200, "the red pawn model replaces its marker box in the viewport: {changed} pixels changed");

    if let Some(dir) = std::env::var_os("ORR_SHOT_DIR") {
        let image = h.render().expect("the editor is composed on the GPU");
        image.save(PathBuf::from(dir).join("editor_viewref.png")).expect("the picture is saved");
    }
}
