//! Real Yard3D host, binding widgets, and the production main viewport.
//! ORR_REQUIRE_GPU=1 makes the GPU acceptance test mandatory; optional PPM
//! captures go to ORR_YARD_CAPTURE_DIR. No isolated renderer substitutes here.
#![cfg(feature = "models")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use orr_editor::{game::EditorGame, model_panel::ModelPanel, Editor, EditorApp, Mode};
use orr_fp::{FPVec3, FP};
use orr_package::{Project, Runtime};
use orr_rhi::{Rhi, Wgpu, WgpuOptions};
use serde_json::json;
use std::{
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const PACKAGE: &str = "sample-imported-scene";
const ASSET: &str = "foreground.glb";

fn install_and_scene(root: &Path) -> PathBuf {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/imported_scene_demo")
        .canonicalize()
        .unwrap();
    Project::open_for_install(root, Runtime::content_only().engine_version)
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
    assert_eq!(editor.game(), EditorGame::Yard3D);
    assert_eq!(editor.rows().len(), 3);
    editor
}

fn headless(scene: &Path) -> Harness<'static, EditorApp> {
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
    assert!(
        h.state().editor.down().is_none(),
        "local Yard3D host stays connected"
    );
}

fn click(h: &mut Harness<'_, EditorApp>, label: &str) {
    h.get_by_label(label).scroll_to_me();
    h.run_steps(2);
    h.get_by_label(label).click();
    settle(h);
}

fn configure_asset(app: &mut EditorApp) {
    app.models.package = PACKAGE.into();
    app.models.asset = ASSET.into();
    app.models.transform.scale = [0.75; 3];
}

fn assign_direct(app: &mut EditorApp) {
    app.models.open_for_editor(&app.editor, true).unwrap();
    configure_asset(app);
    app.models.assign(&app.editor).unwrap();
    assert_eq!(app.models.placements(&app.editor).len(), 1);
}

fn count(app: &EditorApp) -> usize {
    app.models
        .bindings
        .as_ref()
        .unwrap()
        .document()
        .bindings
        .len()
}

fn binding_path(app: &EditorApp) -> PathBuf {
    app.models.bindings.as_ref().unwrap().path.clone()
}

#[test]
fn actual_widgets_assign_remove_undo_save_reopen_and_disable_mutation_in_play() {
    let temp = tempfile::tempdir().unwrap();
    let scene = install_and_scene(temp.path());
    let mut h = headless(&scene);
    settle(&mut h);
    click(&mut h, "box_right");
    click(&mut h, "Static model binding");
    click(&mut h, "Create model bindings");
    assert!(
        h.state().models.error().is_none(),
        "{:?}",
        h.state().models.error()
    );
    configure_asset(h.state_mut());
    let host_checksum = h.state().editor.checksum();
    let host_history = h.state().editor.history().entries.len();
    click(&mut h, "Assign static model");
    assert!(
        h.state().models.error().is_none(),
        "{:?}",
        h.state().models.error()
    );
    assert_eq!(count(h.state()), 1);
    let guid = h.state().editor.selected_guid().unwrap();
    let placements = h.state().models.placements(&h.state().editor);
    assert_eq!(placements.len(), 1);
    assert_eq!(placements[0].instance.translation, [2.0, 3.0, 0.0]);
    assert_eq!(placements[0].instance.scale, [0.75; 3]);
    assert_eq!(h.state().editor.checksum(), host_checksum);
    assert_eq!(h.state().editor.history().entries.len(), host_history);

    // Same assignment is a no-op; only one model undo entry exists.
    click(&mut h, "Assign static model");
    click(&mut h, "Undo model");
    assert_eq!(count(h.state()), 0);
    assert!(h.state().models.placements(&h.state().editor).is_empty());
    click(&mut h, "Redo model");
    assert_eq!(count(h.state()), 1);
    click(&mut h, "Remove model");
    assert_eq!(count(h.state()), 0);
    click(&mut h, "Undo model");
    assert_eq!(count(h.state()), 1);

    let sidecar = binding_path(h.state());
    assert!(!sidecar.exists());
    assert!(h.state_mut().editor.save());
    settle(&mut h);
    assert!(!sidecar.exists(), "scene Save does not save model bindings");
    assert!(h.state().models.bindings.as_ref().unwrap().dirty());
    let scene_bytes = std::fs::read(&scene).unwrap();
    click(&mut h, "Save model bindings");
    assert!(sidecar.exists());
    assert!(!h.state().models.bindings.as_ref().unwrap().dirty());
    assert_eq!(std::fs::read(&scene).unwrap(), scene_bytes);
    let saved = h
        .state()
        .models
        .bindings
        .as_ref()
        .unwrap()
        .document()
        .clone();

    assert!(h.state_mut().editor.start_play());
    settle(&mut h);
    assert_eq!(h.state().editor.mode(), Mode::Play);
    for label in [
        "Create model bindings",
        "Open model bindings",
        "Assign static model",
        "Remove model",
        "Undo model",
        "Redo model",
        "Save model bindings",
        "Reload verified models",
        "Discard model bindings",
    ] {
        assert!(
            h.get_by_label(label).accesskit_node().is_disabled(),
            "{label} must be disabled in Play"
        );
    }
    {
        let app = h.state_mut();
        assert!(app.models.assign(&app.editor).is_err());
        assert!(app.models.open_for_editor(&app.editor, false).is_err());
    }
    assert_eq!(
        h.state().models.bindings.as_ref().unwrap().document(),
        &saved
    );
    h.state_mut().editor.step(12);
    settle(&mut h);
    assert_eq!(h.state().editor.timeline().unwrap().tick, 12);
    assert_ne!(
        h.state().models.placements(&h.state().editor)[0]
            .instance
            .translation,
        placements[0].instance.translation
    );
    assert!(h.state_mut().editor.stop().is_some());
    settle(&mut h);
    assert_eq!(h.state().editor.checksum(), host_checksum);
    click(&mut h, "Discard model bindings");
    assert!(h.state().models.bindings.is_none());
    click(&mut h, "Open model bindings");
    assert_eq!(
        h.state().models.bindings.as_ref().unwrap().document(),
        &saved
    );
    assert!(h
        .state()
        .models
        .bindings
        .as_ref()
        .unwrap()
        .document()
        .bindings
        .contains_key(&guid.to_string()));
    assert_eq!(h.state().models.placements(&h.state().editor).len(), 1);
    // A fresh editor on the saved scene independently resolves the same GUID/model.
    let reopened_editor = editor(&scene);
    let mut reopened = ModelPanel::default();
    reopened.open_for_editor(&reopened_editor, false).unwrap();
    assert_eq!(reopened.bindings.as_ref().unwrap().document(), &saved);
    assert_eq!(reopened.placements(&reopened_editor).len(), 1);
}

#[test]
fn failed_assignment_preserves_renderable_state_and_recycled_body_gets_no_binding() {
    let temp = tempfile::tempdir().unwrap();
    let scene = install_and_scene(temp.path());
    let mut h = headless(&scene);
    settle(&mut h);
    click(&mut h, "box_right");
    assign_direct(h.state_mut());
    click(&mut h, "Static model binding");
    let old_guid = h.state().editor.selected_guid().unwrap();
    let old_entity = h
        .state()
        .editor
        .row_of(h.state().editor.selection().unwrap())
        .unwrap()
        .entity;
    let before = h
        .state()
        .models
        .bindings
        .as_ref()
        .unwrap()
        .document()
        .clone();
    let before_model = h.state().models.placements(&h.state().editor)[0]
        .model
        .clone();
    for asset in ["../foreground.glb", "missing.glb"] {
        h.state_mut().models.asset = asset.into();
        click(&mut h, "Assign static model");
        assert!(h.state().models.error().is_some());
        assert_eq!(
            h.state().models.bindings.as_ref().unwrap().document(),
            &before
        );
        let placed = h.state().models.placements(&h.state().editor);
        assert_eq!(placed.len(), 1);
        assert!(std::sync::Arc::ptr_eq(&placed[0].model, &before_model));
    }
    h.state_mut().models.asset = ASSET.into();
    h.state_mut().models.transform.scale = [0.0, 1.0, 1.0];
    click(&mut h, "Assign static model");
    assert!(h.state().models.error().is_some());
    assert_eq!(
        h.state().models.bindings.as_ref().unwrap().document(),
        &before
    );
    assert_eq!(h.state().models.placements(&h.state().editor).len(), 1);
    h.state_mut().models.transform.scale = [0.75; 3];
    h.state_mut().models.transform.rotation = [0.0, 0.0, 0.0, 2.0];
    click(&mut h, "Assign static model");
    assert!(h.state().models.error().is_some());
    assert_eq!(
        h.state().models.bindings.as_ref().unwrap().document(),
        &before
    );

    // Local TRS is valid in isolation, but Body3 x=2 pushes the composed
    // placement beyond the renderer's +/-1e6 bound. Reject before committing.
    h.state_mut().models.transform.rotation = [0.0, 0.0, 0.0, 1.0];
    h.state_mut().models.transform.translation = [1.0e6, 0.0, 0.0];
    h.state().models.transform.validate().unwrap();
    click(&mut h, "Assign static model");
    assert!(h.state().models.error().is_some());
    assert_eq!(
        h.state().models.bindings.as_ref().unwrap().document(),
        &before
    );
    let still_valid = h.state().models.placements(&h.state().editor);
    assert_eq!(still_valid.len(), 1);
    assert!(std::sync::Arc::ptr_eq(&still_valid[0].model, &before_model));
    assert_eq!(still_valid[0].instance.translation, [2.0, 3.0, 0.0]);

    assert!(h.state().editor.yard_rows_coherent());
    assert!(h.state_mut().editor.delete_selected());
    assert!(!h.state().editor.yard_rows_coherent());
    assert!(
        h.state().models.placements(&h.state().editor).is_empty(),
        "do not render a cached GUID-to-handle association before deletion refresh"
    );
    settle(&mut h);
    assert!(h.state().editor.yard_rows_coherent());
    assert!(h.state().models.placements(&h.state().editor).is_empty());
    assert!(h.state_mut().editor.undo());
    assert!(!h.state().editor.yard_rows_coherent());
    assert!(
        h.state().models.placements(&h.state().editor).is_empty(),
        "undo must wait for coherent scene rows before restoring a model placement"
    );
    settle(&mut h);
    assert!(h.state().editor.yard_rows_coherent());
    let restored = h.state().models.placements(&h.state().editor);
    assert_eq!(restored.len(), 1);
    assert!(std::sync::Arc::ptr_eq(&restored[0].model, &before_model));
    assert!(h.state_mut().editor.select_named("box_right"));
    assert!(h.state_mut().editor.delete_selected());
    settle(&mut h);
    assert!(h.state_mut().editor.spawn_yard_body(FPVec3::new(
        FP::from_int(2),
        FP::from_int(3),
        FP::ZERO
    )));
    assert!(!h.state().editor.yard_rows_coherent());
    assert!(h.state().models.placements(&h.state().editor).is_empty());
    settle(&mut h);
    let new_guid = h.state().editor.selected_guid().unwrap();
    let new_entity = h
        .state()
        .editor
        .row_of(h.state().editor.selection().unwrap())
        .unwrap()
        .entity;
    assert_ne!(new_guid, old_guid);
    assert_eq!(
        new_entity.index, old_entity.index,
        "this fixture actually recycles the preview-frame slot"
    );
    assert!(!h
        .state()
        .models
        .bindings
        .as_ref()
        .unwrap()
        .document()
        .bindings
        .contains_key(&new_guid.to_string()));
    assert!(
        h.state().models.placements(&h.state().editor).is_empty(),
        "a recyclable ECS slot cannot inherit the deleted GUID's model"
    );
    assert_eq!(
        h.state().models.bindings.as_ref().unwrap().document(),
        &before,
        "orphaned authoring data is retained for explicit repair"
    );
}

#[test]
fn save_as_keeps_old_dirty_model_sidecar_saveable_without_binding_the_new_scene() {
    let temp = tempfile::tempdir().unwrap();
    let scene = install_and_scene(temp.path());
    let other = temp.path().join("other.scene.yaml");
    let mut h = headless(&scene);
    settle(&mut h);
    click(&mut h, "box_right");
    assign_direct(h.state_mut());
    click(&mut h, "Static model binding");
    let sidecar = binding_path(h.state());
    let document = h
        .state()
        .models
        .bindings
        .as_ref()
        .unwrap()
        .document()
        .clone();
    assert!(!sidecar.exists());
    assert!(h.state_mut().editor.save_as(&other));
    settle(&mut h);
    assert_eq!(h.state().editor.path().unwrap(), other);
    assert!(!h.state().models.scene_matches(&h.state().editor));
    assert!(h.state().models.placements(&h.state().editor).is_empty());
    assert!(h.state().models.bindings.as_ref().unwrap().dirty());
    assert!(!h
        .get_by_label("Save model bindings")
        .accesskit_node()
        .is_disabled());
    click(&mut h, "Save model bindings");
    assert!(sidecar.exists());
    assert!(!h.state().models.bindings.as_ref().unwrap().dirty());
    assert_eq!(
        orr_editor::model_bindings::Bindings::open(sidecar)
            .unwrap()
            .document(),
        &document
    );
    assert!(!temp.path().join("other.scene.yaml.models.json").exists());
    assert!(h.state_mut().editor.open_path(&scene));
    settle(&mut h);
    assert!(h.state().models.scene_matches(&h.state().editor));
    assert_eq!(h.state().models.placements(&h.state().editor).len(), 1);
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
fn replaced_package_revision_never_autoswitches_cached_or_saved_bindings() {
    let temp = tempfile::tempdir().unwrap();
    let scene = install_and_scene(temp.path());
    let source = mutable_package_source(temp.path());
    let mut app = EditorApp::new(editor(&scene), None);
    assert!(app.editor.select_named("box_right"));
    app.editor.sync();
    assign_direct(&mut app);
    app.models.bindings.as_mut().unwrap().save().unwrap();
    let old = app.models.bindings.as_ref().unwrap().document().clone();
    let old_binding = old.bindings.values().next().unwrap().clone();
    let old_model = app.models.placements(&app.editor)[0].model.clone();
    reinstall_revision(temp.path(), &source, 1);

    // A failed refresh/open must retain the last good binding and model resource.
    let error = app.models.reload(&app.editor).unwrap_err();
    assert!(error.contains("package changed"), "{error}");
    assert_eq!(app.models.bindings.as_ref().unwrap().document(), &old);
    assert!(!app.models.bindings.as_ref().unwrap().dirty());
    assert!(std::sync::Arc::ptr_eq(
        &app.models.placements(&app.editor)[0].model,
        &old_model
    ));
    let error = app.models.open_for_editor(&app.editor, false).unwrap_err();
    assert!(error.contains("package changed"), "{error}");
    assert_eq!(app.models.bindings.as_ref().unwrap().document(), &old);
    assert!(std::sync::Arc::ptr_eq(
        &app.models.placements(&app.editor)[0].model,
        &old_model
    ));

    // Only explicit assignment admits the new package identity, even though the
    // source file bytes are identical between the two package revisions.
    app.models.assign(&app.editor).unwrap();
    let reassigned = app.models.bindings.as_ref().unwrap().document().clone();
    let new_binding = reassigned.bindings.values().next().unwrap();
    assert_eq!(new_binding.source_hash, old_binding.source_hash);
    assert_ne!(new_binding.package_digest, old_binding.package_digest);
    let new_model = app.models.placements(&app.editor)[0].model.clone();
    assert!(!std::sync::Arc::ptr_eq(&new_model, &old_model));
    let dirty = app.models.bindings.as_ref().unwrap().dirty();
    let error = app.models.undo(&app.editor).unwrap_err();
    assert!(error.contains("package changed"), "{error}");
    assert_eq!(
        app.models.bindings.as_ref().unwrap().document(),
        &reassigned
    );
    assert_eq!(app.models.bindings.as_ref().unwrap().dirty(), dirty);
    assert!(std::sync::Arc::ptr_eq(
        &app.models.placements(&app.editor)[0].model,
        &new_model
    ));
}

#[test]
fn distinct_package_revisions_count_toward_the_eight_asset_bound() {
    let temp = tempfile::tempdir().unwrap();
    let scene = install_and_scene(temp.path());
    let source = mutable_package_source(temp.path());
    let mut app = EditorApp::new(editor(&scene), None);
    assert!(app.editor.select_named("box_right"));
    app.editor.sync();
    assign_direct(&mut app);
    for revision in 1..8 {
        reinstall_revision(temp.path(), &source, revision);
        assert!(app.editor.spawn_yard_body(FPVec3::new(
            FP::ZERO,
            FP::from_int(3),
            FP::from_int(revision as i32)
        )));
        app.editor.sync();
        app.models.assign(&app.editor).unwrap();
    }
    app.models.bindings.as_mut().unwrap().save().unwrap();
    let before = app.models.bindings.as_ref().unwrap().document().clone();
    let cached = app.models.placements(&app.editor);
    assert_eq!(cached.len(), 8);
    let identities: std::collections::BTreeSet<_> = before
        .bindings
        .values()
        .map(|binding| binding.package_digest.as_str())
        .collect();
    assert_eq!(
        identities.len(),
        8,
        "the fixture retains eight independently verified revisions"
    );
    let source_hashes: std::collections::BTreeSet<_> = before
        .bindings
        .values()
        .map(|binding| binding.source_hash.as_str())
        .collect();
    assert_eq!(
        source_hashes.len(),
        1,
        "the actual model/source bytes did not change"
    );
    assert!(before
        .bindings
        .values()
        .all(|binding| binding.package == PACKAGE && binding.asset == ASSET));

    reinstall_revision(temp.path(), &source, 8);
    assert!(app
        .editor
        .spawn_yard_body(FPVec3::new(FP::ZERO, FP::from_int(3), FP::from_int(8))));
    app.editor.sync();
    let error = app.models.assign(&app.editor).unwrap_err();
    assert!(error.contains("eight"), "{error}");
    assert_eq!(app.models.bindings.as_ref().unwrap().document(), &before);
    assert!(!app.models.bindings.as_ref().unwrap().dirty());
    let after = app.models.placements(&app.editor);
    assert_eq!(after.len(), 8);
    for (previous, current) in cached.iter().zip(&after) {
        assert_eq!(previous.entity, current.entity);
        assert!(std::sync::Arc::ptr_eq(&previous.model, &current.model));
    }
    let error = app.models.reload(&app.editor).unwrap_err();
    assert!(error.contains("package changed"), "{error}");
    assert_eq!(app.models.bindings.as_ref().unwrap().document(), &before);
    for (previous, current) in cached.iter().zip(app.models.placements(&app.editor)) {
        assert!(std::sync::Arc::ptr_eq(&previous.model, &current.model));
    }
}

#[test]
fn candidate_aggregate_draw_budget_rejects_before_changing_bindings_or_cache() {
    let temp = tempfile::tempdir().unwrap();
    let scene = install_and_scene(temp.path());
    let source = mutable_package_source(temp.path());
    let imported = orr_model::import::import_path(&source, ASSET).unwrap();
    let mut model_source = imported.source().clone();
    let mut triangle = model_source.primitives[0].clone();
    triangle.vertices = triangle.indices[..3]
        .iter()
        .map(|index| triangle.vertices[*index as usize])
        .collect();
    triangle.indices = vec![0, 1, 2];
    model_source.primitives = (0..1024)
        .map(|index| {
            let mut primitive = triangle.clone();
            primitive.id = format!("{}#node=0/mesh=0/primitive={index}", model_source.asset_id);
            primitive
        })
        .collect();
    let heavy = orr_model::StaticModel::new(model_source).unwrap();
    let cooked = heavy.to_bytes().unwrap();
    assert_eq!(heavy.source().primitives.len(), 1024);
    let manifest = source.join("orr.package.json");
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
    for name in ["many_a.json", "many_b.json"] {
        std::fs::write(source.join(name), &cooked).unwrap();
        value["files"].as_array_mut().unwrap().push(name.into());
    }
    std::fs::write(manifest, serde_json::to_vec(&value).unwrap()).unwrap();
    let installer =
        Project::open_for_install(temp.path(), Runtime::content_only().engine_version).unwrap();
    installer.remove(PACKAGE).unwrap();
    installer.install(&[source]).unwrap();

    let mut app = EditorApp::new(editor(&scene), None);
    app.models.open_for_editor(&app.editor, true).unwrap();
    configure_asset(&mut app);
    app.models.asset = "many_a.json".into();
    for name in ["box_left", "box_right"] {
        assert!(app.editor.select_named(name));
        app.editor.sync();
        app.models.assign(&app.editor).unwrap();
    }
    assert!(app
        .editor
        .spawn_yard_body(FPVec3::new(FP::ZERO, FP::from_int(3), FP::from_int(2))));
    app.editor.sync();
    app.models.assign(&app.editor).unwrap();
    assert_eq!(count(&app), 3);
    assert!(app
        .editor
        .spawn_yard_body(FPVec3::new(FP::ZERO, FP::from_int(3), FP::from_int(4))));
    app.editor.sync();
    app.models.asset = "many_b.json".into();
    let before = app.models.bindings.as_ref().unwrap().document().clone();
    let cached = app.models.placements(&app.editor);
    let error = app.models.assign(&app.editor).unwrap_err();
    assert!(error.contains("draw"), "{error}");
    assert_eq!(app.models.bindings.as_ref().unwrap().document(), &before);
    let after = app.models.placements(&app.editor);
    assert_eq!(after.len(), 3);
    for (previous, current) in cached.iter().zip(after) {
        assert_eq!(previous.entity, current.entity);
        assert!(std::sync::Arc::ptr_eq(&previous.model, &current.model));
    }
}

/// Read the exact immutable local host frame behind the editor snapshot. FrameView
/// intentionally exposes no byte serializer; the read-only ERP frame subscriber
/// provides the same snapshot as Arc<Frame>, matched by tick and checksum.
fn frame_bytes(editor: &mut Editor) -> (u64, u64, Vec<u8>) {
    editor.sync();
    let snapshot = editor.snapshot().expect("host snapshot");
    let tick = snapshot.predicted().tick();
    let checksum = snapshot.predicted().checksum();
    let client = editor.agent_client("yard-model-byte-probe").unwrap();
    client.local_frames.clear();
    client
        .call(
            "watch.subscribe",
            json!({"topics":["frames"],"source":"view","max_fps":60}),
        )
        .unwrap();
    let end = Instant::now() + Duration::from_secs(3);
    loop {
        client.poll().unwrap();
        while let Some(frame) = client.local_frames.pop_front() {
            if frame.frame.tick() == tick && frame.frame.checksum() == checksum {
                return (tick, checksum, frame.frame.to_bytes());
            }
        }
        assert!(
            Instant::now() < end,
            "matching local snapshot bytes did not arrive"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn trajectory(app: &mut EditorApp) -> Vec<(u64, u64, Vec<u8>)> {
    let mut results = vec![frame_bytes(&mut app.editor)];
    assert!(app.editor.start_play());
    results.push(frame_bytes(&mut app.editor));
    app.editor.step(12);
    results.push(frame_bytes(&mut app.editor));
    let _ = app.models.placements(&app.editor);
    app.editor.step(18);
    results.push(frame_bytes(&mut app.editor));
    app.editor.seek(12);
    results.push(frame_bytes(&mut app.editor));
    assert_eq!(results[2], results[4], "seek restores exact tick-12 bytes");
    app.editor.seek(0);
    results.push(frame_bytes(&mut app.editor));
    assert_eq!(
        results[1], results[5],
        "seek restores exact tick-zero bytes"
    );
    assert!(app.editor.stop().is_some());
    results.push(frame_bytes(&mut app.editor));
    assert_eq!(
        results[0], results[6],
        "Stop restores the authored document frame"
    );
    results
}

#[test]
fn enabling_model_bindings_leaves_same_host_play_step_seek_bytes_and_checksums_identical() {
    let temp = tempfile::tempdir().unwrap();
    let scene = install_and_scene(temp.path());
    let mut app = EditorApp::new(editor(&scene), None);
    assert!(app.editor.select_named("box_right"));
    app.editor.sync();
    let without = trajectory(&mut app);
    assign_direct(&mut app);
    let with = trajectory(&mut app);
    assert_eq!(
        without, with,
        "presentation asset loading and binding must not affect deterministic frames"
    );
    assert_eq!(app.models.placements(&app.editor).len(), 1);
}

fn gpu_available() -> bool {
    match Wgpu::headless(WgpuOptions::default()) {
        Ok(gpu) => {
            eprintln!(
                "Yard3D main-viewport GPU adapter: {} (software: {})",
                gpu.adapter_name(),
                gpu.is_software()
            );
            true
        }
        Err(error) => {
            assert!(
                std::env::var_os("ORR_REQUIRE_GPU").is_none(),
                "ORR_REQUIRE_GPU is set but Yard3D GPU is unavailable: {error}"
            );
            eprintln!("SKIP: Yard3D main-viewport GPU unavailable: {error}");
            false
        }
    }
}

fn capture(name: &str, size: (u32, u32), rgba: &[u8]) {
    let Some(directory) = std::env::var_os("ORR_YARD_CAPTURE_DIR") else {
        return;
    };
    let directory = Path::new(&directory);
    std::fs::create_dir_all(directory).unwrap();
    let mut output = std::fs::File::create(directory.join(format!("{name}.ppm"))).unwrap();
    write!(output, "P6\n{} {}\n255\n", size.0, size.1).unwrap();
    for pixel in rgba.as_chunks::<4>().0.iter() {
        output.write_all(&pixel[..3]).unwrap();
    }
}

fn viewport_pixels(h: &Harness<'_, EditorApp>, name: &str) -> Vec<u8> {
    let viewport = h
        .state()
        .viewport3d_gpu()
        .expect("the real EditorApp main viewport rendered through GpuViewport3d");
    let size = h.state().ui.viewport_px;
    let pixels = viewport.gpu().read_rgba8();
    assert_eq!(pixels.len(), (size.0 * size.1 * 4) as usize);
    assert!(
        pixels
            .as_chunks::<4>().0.iter()
            .any(|pixel| pixel[0] > 40 || pixel[1] > 40 || pixel[2] > 40),
        "main viewport contains scene pixels"
    );
    capture(name, size, &pixels);
    pixels
}

#[test]
fn production_editor_main_gpu_viewport_renders_bound_models_edits_play_seek_and_stop() {
    if !gpu_available() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let scene = install_and_scene(temp.path());
    let mut editor = editor(&scene);
    assert!(editor.select_named("box_right"));
    editor.sync();
    let mut h = Harness::builder()
        .with_size([1200.0, 900.0])
        .with_step_dt(1.0 / 60.0)
        .wgpu()
        .build_eframe(move |cc| EditorApp::new(editor, cc.wgpu_render_state.clone()));
    settle(&mut h);
    assert!(h.state().has_gpu());
    let procedural = viewport_pixels(&h, "yard3d-app-procedural");
    let edit_size = h.state().ui.viewport_px;
    let procedural_ui = h.render().expect("compose the procedural viewport");
    let checksum = h.state().editor.checksum();
    assign_direct(h.state_mut());
    settle(&mut h);
    let bound = viewport_pixels(&h, "yard3d-app-bound");
    assert_eq!(h.state().ui.viewport_px, edit_size);
    assert!(
        procedural != bound,
        "installed static model replaces its procedural collider proxy in the main viewport"
    );
    assert_eq!(h.state().editor.checksum(), checksum);
    let image = h
        .render()
        .expect("egui composes the native main-viewport texture");
    capture(
        "yard3d-app-egui",
        (image.width(), image.height()),
        image.as_raw(),
    );
    let rect = h.state().ui.viewport_rect.unwrap();
    let changed_in_viewport = image
        .enumerate_pixels()
        .filter(|(x, y, pixel)| {
            // Exclude viewport instructions and all neighboring inspector/status UI.
            let point = egui::pos2(*x as f32, *y as f32);
            rect.shrink(4.0).contains(point)
                && point.y > rect.min.y + 40.0
                && *pixel != procedural_ui.get_pixel(*x, *y)
        })
        .count();
    assert!(changed_in_viewport > 20,
        "the native registered texture must change inside the composed main viewport: {changed_in_viewport} pixels");

    assert!(h.state_mut().editor.set_yard_transform(
        FPVec3::new(FP::from_int(1), FP::from_int(5), FP::from_int(-1)),
        [FP::ZERO, FP::ZERO, FP::ZERO, FP::ONE]
    ));
    settle(&mut h);
    let placements = h.state().models.placements(&h.state().editor);
    assert_eq!(placements[0].instance.translation, [1.0, 5.0, -1.0]);
    let edited_pose = placements[0].instance;
    let edited = viewport_pixels(&h, "yard3d-app-edited");
    assert_eq!(h.state().ui.viewport_px, edit_size);
    assert!(
        bound != edited,
        "host Body3 XYZ edits move the attached visible model"
    );
    let edited_checksum = h.state().editor.checksum();

    // Play expands the real timeline UI, changing the viewport's aspect ratio.
    // Compare exact pixels within the same mode and dimensions, while requiring
    // identical host/model poses across the Edit -> paused-Play transition.
    assert!(h.state_mut().editor.start_play());
    settle(&mut h);
    assert_eq!(h.state().editor.timeline().unwrap().tick, 0);
    let paused_play0_pose = h.state().models.placements(&h.state().editor)[0].instance;
    assert_eq!(paused_play0_pose, edited_pose);
    let paused_play0 = viewport_pixels(&h, "yard3d-app-paused-play0");
    let play_size = h.state().ui.viewport_px;

    h.state_mut().editor.step(45);
    settle(&mut h);
    assert_eq!(h.state().editor.timeline().unwrap().tick, 45);
    assert_eq!(h.state().ui.viewport_px, play_size);
    let stepped_pose = h.state().models.placements(&h.state().editor)[0].instance;
    assert_ne!(stepped_pose.translation, paused_play0_pose.translation);
    let stepped = viewport_pixels(&h, "yard3d-app-step45");
    assert!(
        paused_play0 != stepped,
        "Play/Step moves the model using the same host body pose"
    );
    h.state_mut().editor.seek(0);
    settle(&mut h);
    assert_eq!(h.state().editor.timeline().unwrap().tick, 0);
    assert_eq!(h.state().ui.viewport_px, play_size);
    assert_eq!(
        h.state().models.placements(&h.state().editor)[0].instance,
        paused_play0_pose
    );
    let rewound = viewport_pixels(&h, "yard3d-app-seek0");
    assert!(
        paused_play0 == rewound,
        "Seek restores the paused-Play viewport's exact original body/model pixels"
    );
    assert!(h.state_mut().editor.stop().is_some());
    settle(&mut h);
    assert_eq!(h.state().editor.checksum(), edited_checksum);
    assert_eq!(h.state().ui.viewport_px, edit_size);
    assert_eq!(
        h.state().models.placements(&h.state().editor)[0].instance,
        edited_pose
    );
    let restored = viewport_pixels(&h, "yard3d-app-stopped");
    assert!(
        edited == restored,
        "Stop returns the Edit viewport to the authored scene pixels"
    );
}

#[cfg(feature = "terrain")]
#[test]
fn scene_owned_reservation_rejects_model_assignment_before_history_or_cache_changes() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let scene = install_and_scene(&root);
    let mut h = headless(&scene);
    settle(&mut h);
    assert!(h.state_mut().editor.select_named("box_right"));
    assign_direct(h.state_mut());
    let before = h.state().models.bindings.as_ref().unwrap().document().clone();
    let before_model = h.state().models.placements(&h.state().editor)[0].model.clone();
    // Eight reserved scene assets leave no slot for this existing model.
    let app = h.state_mut();
    app.models.reserve_scene_models(8);
    assert!(app.models.assign(&app.editor).is_err());
    assert_eq!(app.models.bindings.as_ref().unwrap().document(), &before);
    assert!(std::sync::Arc::ptr_eq(&app.models.placements(&app.editor)[0].model, &before_model));
    app.models.reserve_scene_models(1);
    app.models.assign(&app.editor).unwrap();
    assert_eq!(app.models.bindings.as_ref().unwrap().document(), &before);
}
