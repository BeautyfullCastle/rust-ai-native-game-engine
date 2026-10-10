#![cfg(feature = "models")]
#![allow(clippy::float_arithmetic, clippy::disallowed_types)]

use egui::{Key, Modifiers, accesskit::Role};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use orr_editor::{Editor, EditorApp, Mode, game::EditorGame};
use orr_fp::{FP, FPVec3};
use orr_package::{Project, Runtime};
use orr_rhi::{Rhi, Wgpu, WgpuOptions};
use std::path::{Path, PathBuf};

const PACKAGE: &str = "sample-imported-scene";
const ASSET: &str = "foreground.glb";

fn prepare(root: &Path) -> PathBuf {
    // Resolve the test-owned temporary directory alias (for example macOS /var).
    let root = root.canonicalize().unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/imported_scene_demo")
        .canonicalize()
        .unwrap();
    Project::open_for_install(&root, Runtime::content_only().engine_version)
        .unwrap()
        .install(&[source])
        .unwrap();
    let scene = root.join("yard.scene.yaml");
    std::fs::write(
        &scene,
        include_str!("../../../scenes/yard3d_authoring.scene.yaml"),
    )
    .unwrap();
    scene
}

fn editor(scene: &Path) -> Editor {
    let mut editor = Editor::open_game(scene, EditorGame::Yard3D).unwrap();
    editor.sync();
    editor
}

fn harness(scene: &Path) -> Harness<'static, EditorApp> {
    let editor = editor(scene);
    Harness::builder()
        .with_size([1500.0, 1200.0])
        .with_step_dt(1.0 / 60.0)
        .build_eframe(move |_| EditorApp::new(editor, None))
}

fn settle(h: &mut Harness<'_, EditorApp>) {
    for _ in 0..2 {
        h.state_mut().editor.sync();
        h.run_steps(2);
    }
}

fn wait_for(h: &mut Harness<'_, EditorApp>, description: &str, ready: impl Fn(&EditorApp) -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        settle(h);
        if ready(h.state()) {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for {description}"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

fn host_frame_bytes(h: &mut Harness<'_, EditorApp>) -> Vec<u8> {
    settle(h);
    let snapshot = h.state().editor.snapshot().unwrap();
    let (tick, checksum) = (snapshot.tick(), snapshot.predicted().checksum());
    let client = h
        .state_mut()
        .editor
        .agent_client("material-byte-probe")
        .unwrap();
    client.local_frames.clear();
    client
        .call(
            "watch.subscribe",
            serde_json::json!({"topics":["frames"],"source":"view","max_fps":60}),
        )
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        client.poll().unwrap();
        while let Some(frame) = client.local_frames.pop_front() {
            if frame.frame.tick() == tick && frame.frame.checksum() == checksum {
                return frame.frame.to_bytes();
            }
        }
        assert!(
            std::time::Instant::now() < deadline,
            "matching material host frame missing"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

fn click(h: &mut Harness<'_, EditorApp>, label: &str) {
    h.get_by_label(label).scroll_to_me();
    h.run_steps(2);
    h.get_by_label(label).click();
    settle(h);
}

fn type_number(h: &mut Harness<'_, EditorApp>, label: &str, value: &str) {
    h.get_by_role_and_label(Role::SpinButton, label)
        .scroll_to_me();
    h.run_steps(2);
    h.get_by_role_and_label(Role::SpinButton, label).focus();
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run_steps(2);
    h.get_by_role_and_label(Role::SpinButton, label)
        .type_text(value);
    h.run_steps(2);
    h.key_press(Key::Enter);
    settle(h);
}

fn gpu_available() -> bool {
    match Wgpu::headless(WgpuOptions::default()) {
        Ok(gpu) => {
            eprintln!(
                "static material editor GPU adapter: {} (software: {})",
                gpu.adapter_name(),
                gpu.is_software()
            );
            true
        }
        Err(error) => {
            assert!(
                std::env::var_os("ORR_REQUIRE_GPU").is_none(),
                "ORR_REQUIRE_GPU is set but the editor GPU is unavailable: {error}"
            );
            eprintln!("SKIP: static material editor GPU unavailable: {error}");
            false
        }
    }
}

fn viewport_pixels(h: &Harness<'_, EditorApp>) -> Vec<u8> {
    let viewport = h
        .state()
        .viewport3d_gpu()
        .expect("the production main viewport is GPU-backed");
    let size = h.state().ui.viewport_px;
    let pixels = viewport.gpu().read_rgba8();
    assert_eq!(pixels.len(), (size.0 * size.1 * 4) as usize);
    pixels
}

fn capture_viewport(name: &str, size: (u32, u32), rgba: &[u8]) {
    let Some(directory) = std::env::var_os("ORR_MATERIAL_CAPTURE_DIR") else {
        return;
    };
    let directory = Path::new(&directory);
    std::fs::create_dir_all(directory).unwrap();
    let mut output = std::fs::File::create(directory.join(format!("{name}.ppm"))).unwrap();
    use std::io::Write;
    write!(output, "P6\n{} {}\n255\n", size.0, size.1).unwrap();
    for pixel in rgba.as_chunks::<4>().0 {
        output.write_all(&pixel[..3]).unwrap();
    }
}

#[test]
fn one_static_slot_override_is_saved_undoable_and_play_fenced() {
    let temp = tempfile::tempdir().unwrap();
    let scene = prepare(temp.path());
    let mut h = harness(&scene);
    settle(&mut h);
    click(&mut h, "box_right");
    click(&mut h, "Static model binding");
    click(&mut h, "Create model bindings");
    h.state_mut().models.package = PACKAGE.into();
    h.state_mut().models.asset = ASSET.into();
    click(&mut h, "Assign static model");

    let first_guid = h.state().editor.selected_guid().unwrap();
    click(&mut h, "box_left");
    click(&mut h, "Assign static model");
    let second_guid = h.state().editor.selected_guid().unwrap();
    assert_ne!(first_guid, second_guid);
    click(&mut h, "box_right");

    let guid = first_guid.to_string();
    let binding = h
        .state()
        .models
        .bindings
        .as_ref()
        .unwrap()
        .document()
        .bindings[&guid]
        .clone();
    assert!(binding.material_override.is_none());
    let placements = h.state().models.placements(&h.state().editor);
    assert_eq!(placements.len(), 2);
    assert!(std::sync::Arc::ptr_eq(
        &placements[0].model,
        &placements[1].model
    ));
    assert!(
        placements
            .iter()
            .all(|p| p.instance.material_override.is_none())
    );
    let model_before = placements[0].model.clone();
    let slot = model_before.source().primitives[0].material;
    let imported = model_before.source().materials[slot as usize].base_color;
    let scene_bytes = std::fs::read(&scene).unwrap();
    let host_checksum = h.state().editor.checksum();
    let host_bytes = host_frame_bytes(&mut h);

    // Change R through the production egui numeric field; G/B keep their imported
    // linear values. Applying must affect only this GUID's placement.
    type_number(&mut h, "R", "0.23");
    click(&mut h, "Apply material override");
    let bindings = h.state().models.bindings.as_ref().unwrap();
    let override_value = bindings.document().bindings[&guid]
        .material_override
        .unwrap();
    assert_eq!(override_value.material_slot, slot);
    assert_eq!(
        override_value.base_color_factor,
        [0.23, imported[1], imported[2]]
    );
    let placements = h.state().models.placements(&h.state().editor);
    assert!(
        placements
            .iter()
            .all(|p| std::sync::Arc::ptr_eq(&p.model, &model_before))
    );
    assert_eq!(
        placements
            .iter()
            .find(|p| p.entity
                == h.state()
                    .editor
                    .row_of(&orr_editor::Target::Guid(first_guid.clone()))
                    .unwrap()
                    .entity)
            .unwrap()
            .instance
            .material_override,
        Some(override_value)
    );
    assert_eq!(
        placements
            .iter()
            .find(|p| p.entity
                == h.state()
                    .editor
                    .row_of(&orr_editor::Target::Guid(second_guid.clone()))
                    .unwrap()
                    .entity)
            .unwrap()
            .instance
            .material_override,
        None
    );
    assert_eq!(bindings.document().version, 3);

    let model_doc = bindings.document().clone();
    click(&mut h, "Save model bindings");
    let sidecar = h.state().models.bindings.as_ref().unwrap().path.clone();
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&sidecar).unwrap()).unwrap();
    assert_eq!(saved["version"], 3);
    assert_eq!(
        saved["bindings"][&guid]["material_override"]["material_slot"],
        override_value.material_slot
    );
    let sidecar_bytes = std::fs::read(&sidecar).unwrap();
    assert_eq!(std::fs::read(&scene).unwrap(), scene_bytes);

    click(&mut h, "Undo model");
    assert!(
        h.state()
            .models
            .bindings
            .as_ref()
            .unwrap()
            .document()
            .bindings[&guid]
            .material_override
            .is_none()
    );
    assert_eq!(
        h.state()
            .models
            .bindings
            .as_ref()
            .unwrap()
            .document()
            .version,
        2,
        "undo restores the pre-override sidecar version"
    );
    click(&mut h, "Redo model");
    assert_eq!(
        h.state().models.bindings.as_ref().unwrap().document(),
        &model_doc
    );
    click(&mut h, "Reload material draft");
    click(&mut h, "Reset material override");
    assert!(
        h.state()
            .models
            .bindings
            .as_ref()
            .unwrap()
            .document()
            .bindings[&guid]
            .material_override
            .is_none()
    );
    assert_eq!(
        h.state()
            .models
            .bindings
            .as_ref()
            .unwrap()
            .document()
            .version,
        3
    );
    assert!(
        h.state()
            .models
            .placements(&h.state().editor)
            .iter()
            .all(|p| std::sync::Arc::ptr_eq(&p.model, &model_before))
    );
    click(&mut h, "Undo model");
    assert_eq!(
        h.state().models.bindings.as_ref().unwrap().document(),
        &model_doc
    );

    assert!(h.state_mut().editor.start_play());
    settle(&mut h);
    assert_eq!(h.state().editor.mode(), Mode::Play);
    for label in [
        "Apply material override",
        "Reset material override",
        "Undo model",
        "Redo model",
    ] {
        assert!(
            h.get_by_label(label).accesskit_node().is_disabled(),
            "{label} must be disabled in Play"
        );
    }
    h.state_mut().editor.step(12);
    wait_for(&mut h, "material Play step 12", |app| {
        app.editor.yard_rows_coherent()
            && app
                .editor
                .timeline()
                .is_some_and(|timeline| timeline.tick == 12)
    });
    assert_eq!(
        h.state().models.bindings.as_ref().unwrap().document(),
        &model_doc
    );
    h.state_mut().editor.seek(0);
    wait_for(&mut h, "material Play seek zero", |app| {
        app.editor.yard_rows_coherent()
            && app
                .editor
                .timeline()
                .is_some_and(|timeline| timeline.tick == 0)
    });
    assert_eq!(
        h.state().models.bindings.as_ref().unwrap().document(),
        &model_doc
    );
    assert!(h.state_mut().editor.stop().is_some());
    settle(&mut h);
    assert_eq!(h.state().editor.checksum(), host_checksum);
    assert_eq!(
        host_frame_bytes(&mut h),
        host_bytes,
        "material authoring and Play/Seek/Stop preserve authoritative bytes"
    );
    assert_eq!(std::fs::read(&sidecar).unwrap(), sidecar_bytes);
    assert_eq!(std::fs::read(&scene).unwrap(), scene_bytes);

    let reopened_editor = editor(&scene);
    let mut reopened = orr_editor::model_panel::ModelPanel::default();
    reopened.open_for_editor(&reopened_editor, false).unwrap();
    assert_eq!(reopened.bindings.as_ref().unwrap().document(), &model_doc);
    let reopened_placement = reopened.placements(&reopened_editor);
    assert_eq!(reopened_placement.len(), 2);
    assert!(
        reopened_placement
            .iter()
            .any(|p| p.instance.material_override == Some(override_value))
    );
    assert!(
        reopened_placement
            .iter()
            .any(|p| p.instance.material_override.is_none())
    );
}

#[test]
fn sidecar_revision_change_fences_the_open_material_draft_until_reload() {
    let temp = tempfile::tempdir().unwrap();
    let scene = prepare(temp.path());
    let mut h = harness(&scene);
    settle(&mut h);
    click(&mut h, "box_right");
    click(&mut h, "Static model binding");
    click(&mut h, "Create model bindings");
    h.state_mut().models.package = PACKAGE.into();
    h.state_mut().models.asset = ASSET.into();
    click(&mut h, "Assign static model");

    let guid = h.state().editor.selected_guid().unwrap();
    let bindings = h.state().models.bindings.as_ref().unwrap();
    let root = bindings.project_root().unwrap();
    let loaded = orr_editor::model_bindings::load_asset_for_kind(
        &root,
        PACKAGE,
        ASSET,
        orr_editor::model_bindings::ModelKind::Static,
    )
    .unwrap();
    let slot = loaded.static_model().unwrap().source().primitives[0].material;
    let external = orr_model::MaterialOverride {
        material_slot: slot,
        base_color_factor: [0.13, 0.27, 0.41],
    };
    h.state_mut()
        .models
        .bindings
        .as_mut()
        .unwrap()
        .set_material_override(&guid, Some(external), &loaded)
        .unwrap();
    settle(&mut h);
    assert!(
        h.get_by_label("Apply material override")
            .accesskit_node()
            .is_disabled(),
        "a changed sidecar generation must fence the old draft"
    );
    click(&mut h, "Reload material draft");
    assert!(
        !h.get_by_label("Apply material override")
            .accesskit_node()
            .is_disabled()
    );
    click(&mut h, "Apply material override");
    assert_eq!(
        h.state()
            .models
            .bindings
            .as_ref()
            .unwrap()
            .document()
            .bindings[&guid.to_string()]
            .material_override,
        Some(external)
    );
}

#[test]
fn same_path_scene_replacement_fences_draft_until_reload() {
    let temp = tempfile::tempdir().unwrap();
    let scene = prepare(temp.path());
    let mut h = harness(&scene);
    settle(&mut h);
    click(&mut h, "box_right");
    click(&mut h, "Static model binding");
    click(&mut h, "Create model bindings");
    h.state_mut().models.package = PACKAGE.into();
    h.state_mut().models.asset = ASSET.into();
    click(&mut h, "Assign static model");
    let guid = h.state().editor.selected_guid().unwrap();
    let original_checksum = h.state().editor.checksum();
    let old_scene_path = h.state().editor.path().unwrap().to_path_buf();

    // Save a scene revision to the same path, then reopen it as a replacement
    // host session while the material draft is still resident in the panel.
    assert!(h.state_mut().editor.set_yard_transform(
        FPVec3::new(FP::from_int(5), FP::from_int(7), FP::from_int(-2)),
        [FP::ZERO, FP::ZERO, FP::ZERO, FP::ONE],
    ));
    wait_for(&mut h, "edited coherent scene", |app| {
        app.editor.yard_rows_coherent() && app.editor.checksum() != original_checksum
    });
    assert!(h.state_mut().editor.save());
    wait_for(&mut h, "saved scene rows", |app| {
        app.editor.yard_rows_coherent()
            && app.editor.path().as_deref() == Some(old_scene_path.as_path())
    });
    assert!(h.state_mut().editor.open_path(&scene));
    wait_for(&mut h, "same-path replacement rows", |app| {
        app.editor.yard_rows_coherent()
            && app.editor.path().as_deref() == Some(old_scene_path.as_path())
    });
    h.state_mut()
        .editor
        .select(Some(orr_editor::Target::Guid(guid.clone())));
    wait_for(&mut h, "reselected replacement GUID", |app| {
        app.editor.selected_guid().as_ref() == Some(&guid)
    });
    assert!(h.state().models.scene_matches(&h.state().editor));
    assert_ne!(h.state().editor.checksum(), original_checksum);
    assert!(
        h.get_by_label("Apply material override")
            .accesskit_node()
            .is_disabled(),
        "same-path scene replacement must stale the original draft"
    );
    click(&mut h, "Reload material draft");
    assert!(
        !h.get_by_label("Apply material override")
            .accesskit_node()
            .is_disabled()
    );
    click(&mut h, "Apply material override");
    assert!(
        h.state()
            .models
            .bindings
            .as_ref()
            .unwrap()
            .document()
            .bindings[&guid.to_string()]
            .material_override
            .is_some()
    );
}

#[derive(Clone, Copy)]
enum MaterialDraftAba {
    Selection,
    SceneReopen,
    ExternalSceneLoad,
}

fn assert_material_draft_rejects_aba(operation: MaterialDraftAba) {
    let temp = tempfile::tempdir().unwrap();
    let scene = prepare(temp.path());
    let mut h = harness(&scene);
    settle(&mut h);
    click(&mut h, "box_right");
    click(&mut h, "Static model binding");
    click(&mut h, "Create model bindings");
    h.state_mut().models.package = PACKAGE.into();
    h.state_mut().models.asset = ASSET.into();
    click(&mut h, "Assign static model");
    type_number(&mut h, "R", "0.23");
    let guid = h.state().editor.selected_guid().unwrap();
    let entity = h
        .state()
        .editor
        .rows()
        .iter()
        .find(|row| row.guid.as_ref() == Some(&guid))
        .unwrap()
        .entity;
    let checksum = h.state().editor.checksum();
    let document = h
        .state()
        .models
        .bindings
        .as_ref()
        .unwrap()
        .document()
        .clone();
    let generation = h
        .state()
        .models
        .bindings
        .as_ref()
        .unwrap()
        .generation_token();
    let scene_bytes = std::fs::read(&scene).unwrap();
    // A failed load must not retire a still-valid draft or alter sidecar state.
    assert!(
        !h.state_mut()
            .editor
            .open_path(&temp.path().join("missing.scene.yaml"))
    );
    settle(&mut h);
    assert!(
        !h.get_by_label("Apply material override")
            .accesskit_node()
            .is_disabled()
    );
    assert!(
        h.state_mut()
            .editor
            .agent_client("material-aba-source")
            .unwrap()
            .call(
                "scene.load",
                serde_json::json!({"text": "not a scene: [", "path": scene.display().to_string()})
            )
            .is_err()
    );
    settle(&mut h);
    assert!(
        !h.get_by_label("Apply material override")
            .accesskit_node()
            .is_disabled()
    );
    match operation {
        MaterialDraftAba::SceneReopen => {
            assert!(h.state_mut().editor.open_path(&scene));
            wait_for(&mut h, "identical reopened scene", |app| {
                app.editor.yard_rows_coherent() && app.editor.selected_guid().is_none()
            });
        }
        MaterialDraftAba::ExternalSceneLoad => {
            h.state_mut().editor.agent_client("material-aba-source").unwrap().call(
                "scene.load", serde_json::json!({"text": String::from_utf8(scene_bytes.clone()).unwrap(), "path": scene.display().to_string()})
            ).unwrap();
            wait_for(&mut h, "external identical scene load", |app| {
                app.editor.yard_rows_coherent() && app.editor.selected_guid().is_none()
            });
        }
        MaterialDraftAba::Selection => {
            // Both selection changes happen before another UI frame. Comparing
            // only the final GUID/Entity would revive the old unsaved draft.
            assert!(h.state_mut().editor.select_named("box_left"));
        }
    }
    h.state_mut()
        .editor
        .select(Some(orr_editor::Target::Guid(guid.clone())));
    settle(&mut h);
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(
        h.state()
            .editor
            .rows()
            .iter()
            .find(|row| row.guid.as_ref() == Some(&guid))
            .unwrap()
            .entity,
        entity
    );
    assert_eq!(std::fs::read(&scene).unwrap(), scene_bytes);
    assert!(
        h.get_by_label("Apply material override")
            .accesskit_node()
            .is_disabled(),
        "an identical selection/scene value must not revive an old material draft"
    );
    assert_eq!(
        h.state().models.bindings.as_ref().unwrap().document(),
        &document
    );
    assert!(std::sync::Arc::ptr_eq(
        &h.state()
            .models
            .bindings
            .as_ref()
            .unwrap()
            .generation_token(),
        &generation
    ));
    click(&mut h, "Reload material draft");
    assert!(
        !h.get_by_label("Apply material override")
            .accesskit_node()
            .is_disabled()
    );
    click(&mut h, "Apply material override");
    let factor = h
        .state()
        .models
        .bindings
        .as_ref()
        .unwrap()
        .document()
        .bindings[&guid.to_string()]
        .material_override
        .unwrap();
    assert_ne!(
        factor.base_color_factor[0], 0.23,
        "reload must discard the old draft value"
    );
}

#[test]
fn material_draft_rejects_selection_aba_between_widget_frames() {
    assert_material_draft_rejects_aba(MaterialDraftAba::Selection);
}

#[test]
fn material_draft_rejects_identical_scene_reopen_aba() {
    assert_material_draft_rejects_aba(MaterialDraftAba::SceneReopen);
}

#[test]
fn material_draft_rejects_external_identical_scene_load_aba() {
    assert_material_draft_rejects_aba(MaterialDraftAba::ExternalSceneLoad);
}

fn mutable_package_source(root: &Path) -> PathBuf {
    let source = root.join("package-source");
    std::fs::create_dir(&source).unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/imported_scene_demo");
    for entry in std::fs::read_dir(fixture).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), source.join(entry.file_name())).unwrap();
    }
    source
}

fn reinstall_revision(root: &Path, source: &Path, revision: u32) {
    let manifest = source.join("orr.package.json");
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
    value["version"] = format!("1.0.{revision}").into();
    std::fs::write(manifest, serde_json::to_vec(&value).unwrap()).unwrap();
    let project = Project::open_for_install(root, Runtime::content_only().engine_version).unwrap();
    project.remove(PACKAGE).unwrap();
    project.install(&[source.to_path_buf()]).unwrap();
}

#[test]
fn package_replacement_stales_material_draft_without_autoswitching_binding() {
    let temp = tempfile::tempdir().unwrap();
    let scene = prepare(temp.path());
    let source = mutable_package_source(scene.parent().unwrap());
    let mut h = harness(&scene);
    settle(&mut h);
    click(&mut h, "box_right");
    click(&mut h, "Static model binding");
    click(&mut h, "Create model bindings");
    h.state_mut().models.package = PACKAGE.into();
    h.state_mut().models.asset = ASSET.into();
    click(&mut h, "Assign static model");
    let guid = h.state().editor.selected_guid().unwrap();
    let old_document = h
        .state()
        .models
        .bindings
        .as_ref()
        .unwrap()
        .document()
        .clone();
    let old_model = h.state().models.placements(&h.state().editor)[0]
        .model
        .clone();
    let project_root = h
        .state()
        .models
        .bindings
        .as_ref()
        .unwrap()
        .project_root()
        .unwrap();
    reinstall_revision(&project_root, &source, 1);

    // The first click discovers the changed package while verifying a fresh
    // source identity; it must not rewrite the old binding or replace its cache.
    click(&mut h, "Apply material override");
    assert!(h.state().models.error().is_some());
    assert_eq!(
        h.state().models.bindings.as_ref().unwrap().document(),
        &old_document
    );
    assert!(std::sync::Arc::ptr_eq(
        &h.state().models.placements(&h.state().editor)[0].model,
        &old_model
    ));
    assert!(
        h.get_by_label("Apply material override")
            .accesskit_node()
            .is_disabled()
    );
    let reload_error = {
        let app = h.state_mut();
        app.models.reload(&app.editor).unwrap_err()
    };
    assert!(reload_error.contains("package changed"), "{reload_error}");
    assert_eq!(
        h.state().models.bindings.as_ref().unwrap().document(),
        &old_document
    );

    // Explicit reassignment admits the new package lock, clearing the old
    // one-slot override rather than silently carrying it across identities.
    {
        let app = h.state_mut();
        app.models.assign(&app.editor).unwrap();
    }
    let new_binding = &h
        .state()
        .models
        .bindings
        .as_ref()
        .unwrap()
        .document()
        .bindings[&guid.to_string()];
    assert_ne!(
        new_binding.package_digest,
        old_document.bindings[&guid.to_string()].package_digest
    );
    assert!(new_binding.material_override.is_none());
}

#[test]
#[ignore = "mandatory production-editor GPU acceptance; run with ORR_REQUIRE_GPU=1"]
fn production_main_viewport_changes_after_material_override_edit() {
    if !gpu_available() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let scene = prepare(temp.path());
    let mut editor = editor(&scene);
    assert!(editor.select_named("box_right"));
    editor.sync();
    let mut h = Harness::builder()
        .with_size([1200.0, 900.0])
        .with_step_dt(1.0 / 60.0)
        .wgpu()
        .build_eframe(move |cc| EditorApp::new(editor, cc.wgpu_render_state.clone()));
    settle(&mut h);
    {
        let app = h.state_mut();
        app.models.open_for_editor(&app.editor, true).unwrap();
        app.models.package = PACKAGE.into();
        app.models.asset = ASSET.into();
        app.models.assign(&app.editor).unwrap();
    }
    settle(&mut h);
    click(&mut h, "Static model binding");
    let before = viewport_pixels(&h);
    let size = h.state().ui.viewport_px;
    capture_viewport("material-before", size, &before);
    let checksum = h.state().editor.checksum();
    type_number(&mut h, "R", "1.0");
    type_number(&mut h, "G", "0.0");
    type_number(&mut h, "B", "1.0");
    click(&mut h, "Apply material override");
    settle(&mut h);
    let after = viewport_pixels(&h);
    capture_viewport("material-applied", size, &after);
    assert_ne!(
        before, after,
        "the production viewport must render the material factor override"
    );
    assert!(
        h.state().models.placements(&h.state().editor)[0]
            .instance
            .material_override
            .is_some()
    );
    assert_eq!(
        h.state().editor.checksum(),
        checksum,
        "presentation-only edits cannot affect the host document"
    );
    click(&mut h, "Reset material override");
    settle(&mut h);
    let reset = viewport_pixels(&h);
    capture_viewport("material-reset", size, &reset);
    assert_eq!(
        before, reset,
        "Reset must restore the imported factor in the production viewport"
    );
}

#[cfg(all(
    feature = "room-character",
    feature = "project-create",
    target_os = "linux"
))]
#[test]
fn room_material_override_save_preserves_animated_character_document() {
    use orr_editor::{HostSpec, Target};
    use orr_sample::{
        project_create::{CreateOptions, create},
        room_game::{EXIT, KEY, PLAYER, RoomActor},
        room_project::{CheckpointSupport, PreparedProject, PreparedScene},
    };
    let temp = tempfile::tempdir().unwrap();
    let root = temp
        .path()
        .canonicalize()
        .unwrap()
        .join("room material project");
    create(&CreateOptions {
        output: root.clone(),
        template: "room-escape-character-3d-v1".into(),
        seed: "material-room-regression".into(),
    })
    .unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/imported_scene_demo")
        .canonicalize()
        .unwrap();
    Project::open_for_install(&root, Runtime::content_only().engine_version)
        .unwrap()
        .install(&[source])
        .unwrap();

    let mut prepared =
        PreparedProject::open_with_capabilities(&root, false, CheckpointSupport::Disabled, true)
            .unwrap();
    let camera = prepared.take_camera().unwrap();
    let character = prepared.take_character().unwrap();
    let character_before = character.document.clone();
    let (_, path, scene, models) = prepared.into_parts();
    let source_scene = PreparedScene::parse(scene.text()).unwrap();
    let mut guids = std::collections::BTreeMap::new();
    for entity in source_scene.frame().entities() {
        let kind = source_scene.frame().get::<RoomActor>(entity).unwrap().kind;
        if [PLAYER, KEY, EXIT].contains(&kind) {
            guids.insert(kind, source_scene.index().guid(entity).unwrap().clone());
        }
    }
    let mut editor = Editor::start(&HostSpec::PreparedRoom {
        scene: path,
        text: scene.text().into(),
        listen: None,
        debug_hooks: false,
    })
    .unwrap();
    editor
        .install_room_character(character_before.clone())
        .unwrap();
    editor.install_room_camera(camera.document.clone()).unwrap();
    editor.sync();
    let mut app = EditorApp::new(editor, None);
    app.models.install_room(models).unwrap();
    app.room_character = Some(orr_editor::room_character_panel::Panel::new(character));
    app.room_camera = Some(orr_editor::room_camera_panel::Panel::new(camera));
    for _ in 0..1600 {
        app.editor.sync();
        if app.editor.yard_rows_coherent() && app.models.scene_matches(&app.editor) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(app.editor.yard_rows_coherent());
    app.editor.select(Some(Target::Guid(guids[&KEY].clone())));
    app.editor.sync();
    app.models.package = PACKAGE.into();
    app.models.asset = ASSET.into();
    app.models.assign(&app.editor).unwrap();
    let loaded = orr_editor::model_bindings::load_asset_for_kind(
        &root,
        PACKAGE,
        ASSET,
        orr_editor::model_bindings::ModelKind::Static,
    )
    .unwrap();
    let slot = loaded.static_model().unwrap().source().primitives[0].material;
    let override_value = orr_model::MaterialOverride {
        material_slot: slot,
        base_color_factor: [0.18, 0.36, 0.72],
    };
    app.models
        .bindings
        .as_mut()
        .unwrap()
        .set_material_override(&guids[&KEY], Some(override_value), &loaded)
        .unwrap();
    let material_document = app.models.bindings.as_ref().unwrap().document().clone();
    assert_eq!(
        material_document.bindings[&guids[&PLAYER].to_string()].kind,
        orr_editor::model_bindings::ModelKind::Animated
    );
    assert_eq!(
        material_document.bindings[&guids[&PLAYER].to_string()].material_override,
        None
    );
    app.models.save(&app.editor).unwrap();
    let saved_material =
        PreparedProject::open_with_capabilities(&root, false, CheckpointSupport::Disabled, true)
            .unwrap();
    assert_eq!(
        saved_material.character().unwrap().document,
        character_before
    );
    assert_eq!(saved_material.models().document, material_document);
    // Then change character clips and persist the pair. The static material
    // override must survive the other author's transaction and Save.
    let mut changed_character = character_before.clone();
    std::mem::swap(
        &mut changed_character.carrying,
        &mut changed_character.escaped,
    );
    app.room_character
        .as_mut()
        .unwrap()
        .apply_document(changed_character.clone(), &mut app.editor, &mut app.models)
        .unwrap();
    app.room_character
        .as_mut()
        .unwrap()
        .save_all(&app.editor, &mut app.models)
        .unwrap();

    let reopened =
        PreparedProject::open_with_capabilities(&root, false, CheckpointSupport::Disabled, true)
            .unwrap();
    assert_eq!(reopened.character().unwrap().document, changed_character);
    assert_eq!(reopened.models().document, material_document);
    assert_eq!(reopened.models().document.version, 3);
    assert_eq!(
        reopened.models().document.bindings[&guids[&PLAYER].to_string()].kind,
        orr_editor::model_bindings::ModelKind::Animated
    );
    assert_eq!(
        reopened.models().document.bindings[&guids[&KEY].to_string()].material_override,
        Some(override_value)
    );
}
