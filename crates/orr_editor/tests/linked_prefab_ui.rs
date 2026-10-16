#![cfg(feature = "linked-prefabs")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
use egui_kittest::{kittest::Queryable, Harness};
use orr_editor::{game::EditorGame, linked_prefab_panel::SourceFiles};
use orr_editor::{Editor, EditorApp};
use serde_json::json;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::{Command, Output},
};

fn project() -> (tempfile::TempDir, Editor) {
    let root = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let path = root.path().join("level.scene.yaml");
    std::fs::write(
        &path,
        include_str!("../../../scenes/collect_dodge_v1.scene.yaml"),
    )
    .unwrap();
    let mut editor = Editor::open_game(&path, EditorGame::CollectDodge).unwrap();
    editor.sync();
    assert!(editor.select_named("first_collectible"));
    (root, editor)
}
fn click(h: &mut Harness<'_, EditorApp>, label: &str) {
    h.get_all_by_label(label).next().unwrap().click();
    h.run_steps(3);
    assert!(
        h.state().linked_prefabs.error().is_none(),
        "{label}: {:?}",
        h.state().linked_prefabs.error()
    );
}

#[test]
fn actual_editor_widgets_capture_twice_override_revert_update_save_reopen() {
    actual_widget_workflow(false);
}

#[test]
#[ignore = "mandatory explicit software-GPU, built runtime/exporter and namespace-isolated export acceptance"]
fn actual_editor_gpu_linked_prefabs_source_hidden_export() {
    actual_widget_workflow(true);
}

fn actual_widget_workflow(gpu: bool) {
    let (root, mut editor) = project();
    // Keep source actors and the explicit override inside the tall test viewport.
    editor.camera = orr_render::Camera::new([5.0, 6.0], 30.0);
    let builder = Harness::builder().with_size([1600.0, 2200.0]);
    let builder = if gpu { builder.wgpu() } else { builder };
    let mut h = builder.build_eframe(|cc| {
        if gpu {
            let state = cc
                .wgpu_render_state
                .as_ref()
                .expect("mandatory real renderer");
            assert_eq!(
                state.adapter.get_info().device_type,
                egui_wgpu::wgpu::DeviceType::Cpu,
                "mandatory software GPU"
            );
        }
        EditorApp::new(
            editor,
            if gpu {
                cc.wgpu_render_state.clone()
            } else {
                None
            },
        )
    });
    if gpu {
        assert!(h.state().has_gpu());
    }
    click(&mut h, "Capture selected to new source");
    let source = root.path().join("actors.prefab.yaml");
    assert!(source.is_file());
    click(&mut h, "Instantiate source");
    click(&mut h, "Instantiate source");
    assert_eq!(
        h.state().linked_prefabs.state().unwrap()["instances"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    h.state_mut().editor.sync();
    assert_eq!(
        h.state().editor.bodies().len(),
        6,
        "authoritative viewport includes both instances"
    );
    // This first profile uses procedural actor presentation, not inherited sprite bindings.
    let before_override = if gpu {
        Some(viewport_capture(&mut h, "linked-before-override"))
    } else {
        None
    };
    h.state_mut().linked_prefabs.position = ["6".into(), "6".into()];
    click(&mut h, "Override position");
    if let Some(before) = before_override {
        let after = viewport_capture(&mut h, "linked-after-override");
        assert_ne!(
            before, after,
            "actual cropped viewport collectible pixels must move after override"
        );
    }
    assert_eq!(
        h.state().linked_prefabs.state().unwrap()["instances"][0]["position_overrides"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    click(&mut h, "Revert position");
    assert_eq!(
        h.state().linked_prefabs.state().unwrap()["instances"][0]["position_overrides"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    let original = std::fs::read_to_string(&source).unwrap();
    let old = h.state().linked_prefabs.state().unwrap().clone();
    std::fs::write(&source, "invalid fragment").unwrap();
    h.get_all_by_label("Apply source update")
        .next()
        .unwrap()
        .click();
    h.run_steps(3);
    assert!(h.state().linked_prefabs.error().is_some());
    assert_eq!(
        h.state().linked_prefabs.state().unwrap(),
        &old,
        "conflict retains previous view"
    );
    click(&mut h, "Override position");
    // Edit the original, unlinked actor using its actual inspector field.
    use egui::accesskit::Role;
    h.state_mut().editor.sync();
    h.run_steps(3);
    h.get_by(|n| n.role() == Role::TextInput && n.value().as_deref() == Some("10"))
        .focus();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(2);
    h.get_by(|n| n.role() == Role::TextInput && n.value().as_deref() == Some("10"))
        .type_text("11");
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
    while !h
        .state()
        .editor
        .bodies()
        .iter()
        .any(|b| b.pos == [11.0, 0.0])
    {
        assert!(
            std::time::Instant::now() < deadline,
            "actual original actor inspector edit did not reach host"
        );
        h.run_steps(1);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let before_replace = h.state().linked_prefabs.state().unwrap()["instances"].clone();
    click(&mut h, "Replace source from selection");
    let changed = std::fs::read_to_string(&source).unwrap();
    assert_ne!(changed, original);
    assert!(changed.contains("position: [11, 0]"));
    assert_eq!(
        h.state().linked_prefabs.state().unwrap()["instances"],
        before_replace,
        "source replacement does not implicitly update linked instances"
    );
    click(&mut h, "Apply source update");
    let updated = h.state().linked_prefabs.state().unwrap();
    assert_ne!(
        updated["instances"][0]["digest"],
        old["instances"][0]["digest"]
    );
    assert_eq!(
        updated["instances"][1]["digest"],
        old["instances"][1]["digest"]
    );
    assert_eq!(
        updated["instances"][0]["position_overrides"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    click(&mut h, "Revert position");
    detach_widget_workflow(&mut h, root.path(), gpu);
    assert!(h.state_mut().editor.save());
    let expected = h
        .state_mut()
        .editor
        .host_call("prefab.list", json!({}))
        .unwrap();
    h.state_mut().editor.sync();
    let edited_checksum = h.state().editor.checksum();
    h.state_mut()
        .editor
        .host_call("sim.start", json!({"run":false}))
        .unwrap();
    let _stepped = h
        .state_mut()
        .editor
        .host_call("sim.step", json!({"n":20}))
        .unwrap();
    let checksum_reply = h
        .state_mut()
        .editor
        .host_call("sim.checksum", json!({}))
        .unwrap();
    assert_eq!(checksum_reply["tick"], 20);
    let idle_checksum = checksum_reply["checksum"].as_str().unwrap().to_owned();
    h.state_mut()
        .editor
        .host_call("sim.stop", json!({}))
        .unwrap();
    assert_eq!(
        h.state_mut()
            .editor
            .host_call("prefab.list", json!({}))
            .unwrap()["instances"],
        expected["instances"]
    );
    drop(h);
    let mut reopened = Editor::open_game(
        &root.path().join("level.scene.yaml"),
        EditorGame::CollectDodge,
    )
    .unwrap();
    let actual = reopened.host_call("prefab.list", json!({})).unwrap();
    assert_eq!(actual["instances"], expected["instances"]);
    reopened.sync();
    assert_eq!(reopened.checksum(), edited_checksum);
    drop(reopened);
    if gpu {
        source_hidden_export(root.path(), edited_checksum, &idle_checksum);
    }
}

/// Extend the same real CPU/GPU editor workflow through metadata-only detach,
/// independent Inspector editing, deleted-source detach, legacy reopen and Undo.
fn detach_widget_workflow(h: &mut Harness<'_, EditorApp>, root: &Path, gpu: bool) {
    use egui::accesskit::Role;
    let source = root.join("actors.prefab.yaml");
    let source_bytes = std::fs::read(&source).unwrap();
    h.state_mut().linked_prefabs.position = ["6".into(), "7".into()];
    click(h, "Override position");
    let linked = editor_scene(h);
    let state = h.state().linked_prefabs.state().unwrap().clone();
    let first = state["instances"][0]["instance"].as_str().unwrap().to_owned();
    let remaining = state["instances"][1].clone();
    let target = state["instances"][0]["guids"]["e_00000002"].as_str().unwrap().to_owned();
    h.state_mut().editor.sync();
    let checksum = h.state().editor.checksum();
    let before_pixels = gpu.then(|| viewport_rgba(h, "linked-before-detach"));
    click(h, "Detach instance");
    let detached = editor_scene(h);
    assert_eq!(detached.entities, linked.entities);
    assert_eq!(detached.singletons, linked.singletons);
    assert_eq!(detached.prefab_links.len(), 1);
    assert_eq!(h.state().linked_prefabs.state().unwrap()["instances"], json!([remaining]));
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(std::fs::read(&source).unwrap(), source_bytes);
    if let Some(before) = before_pixels {
        assert_eq!(viewport_rgba(h, "linked-after-detach"), before, "detach must preserve every viewport RGBA pixel");
    }
    // Actual Edit-menu Undo and Redo restore the entire provenance, including
    // overridden positions. Refresh explicitly fences the next panel mutation.
    click(h, "Edit");
    click(h, "Undo Ctrl+Z");
    click(h, "Refresh linked prefabs");
    assert_eq!(editor_scene(h), linked);
    assert_eq!(h.state().linked_prefabs.state().unwrap()["instances"], state["instances"]);
    click(h, "Edit");
    click(h, "Redo Ctrl+Y");
    click(h, "Refresh linked prefabs");
    assert_eq!(editor_scene(h), detached);
    assert!(h.state().linked_prefabs.state().unwrap()["instances"].as_array().unwrap().iter().all(|v| v["instance"] != first));
    h.state_mut().linked_prefabs.position = ["0".into(), "0".into()];
    h.state_mut().editor.select(Some(orr_editor::Target::Guid(orr_reflect::Guid::parse(&target).unwrap())));
    h.state_mut().editor.sync();
    h.run_steps(3);
    h.get_by(|n| n.role() == Role::TextInput && n.value().as_deref() == Some("6")).focus();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(2);
    h.get_by(|n| n.role() == Role::TextInput && n.value().as_deref() == Some("6")).type_text("8");
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    let guid = orr_reflect::Guid::parse(&target).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
    loop {
        h.state_mut().editor.sync();
        let scene = editor_scene(h);
        if scene.entities[&guid].components[0].1.field("position") == Some(&orr_reflect::Value::Vec2(orr_fp::FPVec2::new(orr_fp::FP::from_int(8), orr_fp::FP::from_int(7)))) {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "detached actor Inspector edit did not reach host");
        h.run_steps(1);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let independently_edited = editor_scene(h).entities[&guid].clone();
    click(h, "Refresh linked prefabs");
    click(h, "Apply source update");
    let updated = editor_scene(h);
    assert_eq!(updated.entities[&guid], independently_edited);
    let other = orr_reflect::Guid::parse(remaining["instance"].as_str().unwrap()).unwrap();
    assert_ne!(updated.prefab_links[&other].digest, detached.prefab_links[&other].digest);
    assert_eq!(std::fs::read(&source).unwrap(), source_bytes);
    std::fs::remove_file(&source).unwrap();
    let before_last = editor_scene(h);
    let before_pixels = gpu.then(|| viewport_rgba(h, "linked-before-last-detach"));
    click(h, "Detach instance");
    let legacy = editor_scene(h);
    assert!(legacy.prefab_links.is_empty());
    assert_eq!(legacy.entities, before_last.entities);
    assert!(legacy.to_yaml().contains("schema: orr.scene/1"));
    assert!(!source.exists(), "detach must not recreate the missing source");
    if let Some(before) = before_pixels {
        assert_eq!(viewport_rgba(h, "linked-after-last-detach"), before);
    }
    assert!(h.state_mut().editor.save());
    let saved = std::fs::read_to_string(root.join("level.scene.yaml")).unwrap();
    assert_eq!(saved, legacy.to_yaml());
    let mut reopened = Editor::open_game(&root.join("level.scene.yaml"), EditorGame::CollectDodge).unwrap();
    assert_eq!(reopened.host_call("prefab.list", json!({})).unwrap()["instances"], json!([]));
    reopened.sync();
    h.state_mut().editor.sync();
    assert_eq!(reopened.checksum(), h.state().editor.checksum());
    drop(reopened);
    click(h, "Edit");
    click(h, "Undo Ctrl+Z");
    click(h, "Refresh linked prefabs");
    assert_eq!(editor_scene(h), before_last);
    assert_eq!(h.state().linked_prefabs.state().unwrap()["instances"].as_array().unwrap().len(), 1);
    assert!(!source.exists());
}

fn editor_scene(h: &mut Harness<'_, EditorApp>) -> orr_reflect::Scene {
    let saved = h.state_mut().editor.host_call("scene.save", json!({})).unwrap();
    orr_reflect::Scene::parse(saved["text"].as_str().unwrap(), &orr_sample::collect_project::types()).unwrap()
}

/// Exact cropped viewport pixels, independent of panel text and row count.
fn viewport_rgba(h: &mut Harness<'_, EditorApp>, name: &str) -> Vec<[u8; 4]> {
    viewport_capture(h, name);
    let image = h.render().expect("mandatory composed editor GPU readback");
    let rect = h.state().ui.viewport_rect.unwrap();
    let mut rgba = Vec::new();
    for y in rect.min.y.ceil() as u32..rect.max.y.floor() as u32 {
        for x in rect.min.x.ceil() as u32..rect.max.x.floor() as u32 {
            rgba.push(image.get_pixel(x, y).0);
        }
    }
    assert!(!rgba.is_empty());
    rgba
}

#[test]
fn source_files_reject_paths_existing_files_and_oversize() {
    let root = tempfile::tempdir().unwrap();
    let files = SourceFiles::new(root.path()).unwrap();
    // An active scene is allowed to have the prefab suffix, but must never be
    // treated as the replacement target by scene-bound source authoring.
    let scene_path = root.path().join("level.prefab.yaml");
    let scene_bytes = include_str!("../../../scenes/collect_dodge_v1.scene.yaml");
    std::fs::write(&scene_path, scene_bytes).unwrap();
    let protected = SourceFiles::new_for_scene(&scene_path).unwrap();
    assert!(protected
        .replace_source("level.prefab.yaml", scene_bytes, "fragment")
        .is_err());
    assert!(protected.read("level.prefab.yaml").is_err());
    assert!(protected.save_new("level.prefab.yaml", "fragment").is_err());
    assert!(protected
        .replace_source("LEVEL.prefab.yaml", scene_bytes, "fragment")
        .is_err());
    assert!(protected.save_new("LEVEL.prefab.yaml", "fragment").is_err());
    assert_eq!(std::fs::read_to_string(&scene_path).unwrap(), scene_bytes);
    #[cfg(unix)]
    {
        std::fs::hard_link(&scene_path, root.path().join("alias.prefab.yaml")).unwrap();
        assert!(protected
            .replace_source("alias.prefab.yaml", scene_bytes, "fragment")
            .is_err());
        assert!(protected.read("alias.prefab.yaml").is_err());
        assert_eq!(
            std::fs::read_to_string(root.path().join("alias.prefab.yaml")).unwrap(),
            scene_bytes
        );
    }
    for name in [
        "",
        "../escape",
        "/tmp/escape",
        "sub/file",
        "sub\\file",
        ".hidden",
        "C:escape",
        "CON.yaml",
        "LPT1.yaml",
    ] {
        assert!(files.save_new(name, "source").is_err(), "{name}");
    }
    files.save_new("actors.yaml", "original").unwrap();
    assert!(files.save_new("actors.yaml", "replacement").is_err());
    assert_eq!(files.read("actors.yaml").unwrap(), "original");
    files.save_new("actors.prefab.yaml", "original").unwrap();
    assert!(files
        .replace_source("actors.yaml", "original", "bad")
        .is_err());
    assert_eq!(files.read("actors.yaml").unwrap(), "original");
    assert!(files
        .replace_source("actors.prefab.yaml", "stale", "bad")
        .is_err());
    assert_eq!(files.read("actors.prefab.yaml").unwrap(), "original");
    assert!(files
        .replace_source("actors.prefab.yaml", "original", &"x".repeat(32769))
        .is_err());
    assert_eq!(files.read("actors.prefab.yaml").unwrap(), "original");
    files
        .replace_source("actors.prefab.yaml", "original", "replacement")
        .unwrap();
    assert_eq!(files.read("actors.prefab.yaml").unwrap(), "replacement");
    let readonly_path = root.path().join("actors.prefab.yaml");
    let original_permissions = std::fs::metadata(&readonly_path).unwrap().permissions();
    let mut readonly = original_permissions.clone();
    readonly.set_readonly(true);
    std::fs::set_permissions(&readonly_path, readonly).unwrap();
    assert!(files
        .replace_source("actors.prefab.yaml", "replacement", "bad")
        .is_err());
    assert_eq!(
        std::fs::read_to_string(&readonly_path).unwrap(),
        "replacement"
    );
    std::fs::set_permissions(&readonly_path, original_permissions).unwrap();
    assert!(files
        .replace_source("missing.prefab.yaml", "", "bad")
        .is_err());
    assert!(!root.path().join("missing.prefab.yaml").exists());
    std::fs::create_dir(root.path().join("directory.prefab.yaml")).unwrap();
    assert!(files
        .replace_source("directory.prefab.yaml", "", "bad")
        .is_err());

    assert!(files.save_new("large.yaml", &"a".repeat(32769)).is_err());
    std::fs::write(root.path().join("large.yaml"), vec![0; 32769]).unwrap();
    assert!(files.read("large.yaml").is_err());
    std::fs::create_dir(root.path().join("directory.yaml")).unwrap();
    assert!(files.read("directory.yaml").is_err());
}

#[cfg(unix)]
#[test]
fn source_files_reject_symlinks_and_replaced_ancestors() {
    use std::os::unix::fs::symlink;
    let outer = tempfile::tempdir().unwrap();
    let root = outer.path().join("project");
    std::fs::create_dir(&root).unwrap();
    let files = SourceFiles::new(&root).unwrap();
    files.save_new("actors.yaml", "source").unwrap();
    symlink(root.join("actors.yaml"), root.join("link.yaml")).unwrap();
    assert!(files.read("link.yaml").is_err());
    assert!(files.save_new("link.yaml", "replacement").is_err());
    symlink(root.join("actors.yaml"), root.join("link.prefab.yaml")).unwrap();
    assert!(files
        .replace_source("link.prefab.yaml", "source", "bad")
        .is_err());
    assert_eq!(files.read("actors.yaml").unwrap(), "source");
    files.save_new("actors.prefab.yaml", "source").unwrap();
    std::fs::rename(&root, outer.path().join("old")).unwrap();
    std::fs::create_dir(&root).unwrap();
    assert!(files.save_new("other.yaml", "source").is_err());
    assert!(files.read("actors.yaml").is_err());
    assert!(files
        .replace_source("actors.prefab.yaml", "source", "bad")
        .is_err());
    assert_eq!(
        std::fs::read_to_string(outer.path().join("old/actors.prefab.yaml")).unwrap(),
        "source"
    );
    symlink(outer.path().join("old"), outer.path().join("alias")).unwrap();
    assert!(SourceFiles::new(&outer.path().join("alias")).is_err());
}

#[test]
fn panel_is_hidden_for_physics() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scenes/physics_demo.scene.yaml");
    let editor = Editor::open(&path).unwrap();
    let h = Harness::builder()
        .with_size([1400.0, 900.0])
        .build_eframe(|_| EditorApp::new(editor, None));
    assert!(h.query_by_label("Linked prefabs").is_none());
    assert!(h.query_by_label("Capture selected to new source").is_none());
}

#[test]
fn panel_is_hidden_for_arena_and_play() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scenes/arena_blank.scene.yaml");
    let arena = Editor::open_game(&path, EditorGame::Arena).unwrap();
    let h = Harness::builder()
        .with_size([1400.0, 900.0])
        .build_eframe(|_| EditorApp::new(arena, None));
    assert!(h.query_by_label("Capture selected to new source").is_none());
    drop(h);
    let (_root, editor) = project();
    let mut h = Harness::builder()
        .with_size([1400.0, 900.0])
        .build_eframe(|_| EditorApp::new(editor, None));
    h.get_by_label(orr_editor::app::LBL_PLAY).click();
    h.run_steps(3);
    h.state_mut().editor.sync();
    h.run_steps(3);
    assert!(h.state().editor.is_playing_mode());
    assert!(h.query_by_label("Capture selected to new source").is_none());
}

/// Returns only yellow actor pixels from the viewport, excluding all inspector
/// labels and controls. Clone presentation is deliberately procedural fallback.
fn viewport_capture(h: &mut Harness<'_, EditorApp>, name: &str) -> Vec<(u32, u32)> {
    h.state_mut().editor.sync();
    h.run_steps(4);
    assert!(h.state().has_gpu());
    let image = h.render().expect("mandatory composed editor GPU readback");
    let rect = h
        .state()
        .ui
        .viewport_rect
        .expect("actual viewport rectangle");
    let mut actors = [0usize; 3];
    let mut yellow = Vec::new();
    for y in rect.min.y.ceil() as u32..rect.max.y.floor() as u32 {
        for x in rect.min.x.ceil() as u32..rect.max.x.floor() as u32 {
            let p = image.get_pixel(x, y).0;
            if p[2] > 180 && p[0] < 160 && p[1] < 200 {
                actors[0] += 1;
            }
            if p[0] > 180 && p[1] > 180 && p[2] < 120 {
                actors[1] += 1;
                yellow.push((x, y));
            }
            if p[0] > 180 && p[1] < 160 && p[2] < 160 {
                actors[2] += 1;
            }
        }
    }
    assert!(
        actors.iter().all(|n| *n > 30),
        "missing actual procedural player/collectible/hazard pixels: {actors:?}"
    );
    if let Some(dest) = std::env::var_os("ORR_LINKED_CAPTURES") {
        let dest = PathBuf::from(dest);
        std::fs::create_dir_all(&dest).unwrap();
        image.save(dest.join(format!("{name}.png"))).unwrap();
    }
    yellow
}
fn good(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}\n{}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}
fn tree_bytes(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let kind = entry.file_type().unwrap();
            assert!(!kind.is_symlink(), "export must not depend on symlinks");
            if kind.is_dir() {
                visit(root, &entry.path(), out);
            } else {
                assert!(kind.is_file());
                out.insert(
                    entry.path().strip_prefix(root).unwrap().to_owned(),
                    std::fs::read(entry.path()).unwrap(),
                );
            }
        }
    }
    let mut result = BTreeMap::new();
    visit(root, root, &mut result);
    result
}

/// Runs trusted prebuilt binaries outside the checkout. Every subprocess hides
/// the entire workspace and original editor project. Export runs additionally
/// hide the staged project/tools and mount the relocated bundle read-only.
fn source_hidden_export(original_project: &Path, expected_checksum: u64, idle_checksum: &str) {
    use orr_sample::collect_project::PreparedProject;
    let scene = std::fs::read(original_project.join("level.scene.yaml")).unwrap();
    assert!(String::from_utf8_lossy(&scene).contains("orr.scene/2"));
    std::fs::write(
        original_project.join("orr.project.json"),
        include_bytes!("../../../assets/collect_dodge_project/orr.project.json"),
    )
    .unwrap();
    assert!(!original_project.join("actors.prefab.yaml").exists(), "detach workflow already removed the source before save/reopen");
    assert_eq!(
        PreparedProject::open(original_project)
            .unwrap()
            .scene()
            .frame()
            .checksum(),
        expected_checksum,
        "source-free prepared initial frame matches editor"
    );

    let work = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let base = work.path();
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .unwrap();
    assert!(!base.starts_with(&workspace));
    assert!(!original_project.starts_with(&workspace));
    let project = base.join("saved project");
    let tools = base.join("trusted tools");
    let captures = base.join("captures");
    let empty = base.join("empty cwd");
    for dir in [&project, &tools, &captures, &empty] {
        std::fs::create_dir(dir).unwrap();
    }
    std::fs::write(project.join("level.scene.yaml"), &scene).unwrap();
    std::fs::copy(
        original_project.join("orr.project.json"),
        project.join("orr.project.json"),
    )
    .unwrap();
    let mut binaries = Vec::new();
    for (key, name) in [
        ("ORR_COLLECT_RUNTIME", "collect_dodge"),
        ("ORR_COLLECT_EXPORTER", "orr_export_collect"),
    ] {
        let original = PathBuf::from(
            std::env::var_os(key).unwrap_or_else(|| panic!("mandatory acceptance requires {key}")),
        )
        .canonicalize()
        .unwrap();
        assert!(original.is_file());
        assert!(
            original.starts_with(&workspace),
            "trusted original binary must be inside the hidden workspace: {}",
            original.display()
        );
        let copy = tools.join(name);
        std::fs::copy(original, &copy).unwrap();
        binaries.push(copy);
    }
    let isolated = |executable: &Path, bundle: Option<&Path>| {
        let mut cmd = Command::new("bwrap");
        cmd.args([
            "--die-with-parent",
            "--ro-bind",
            "/",
            "/",
            "--dev-bind",
            "/dev/null",
            "/dev/null",
            "--bind",
        ])
        .arg(base)
        .arg(base)
        .arg("--tmpfs")
        .arg(&workspace)
        .arg("--tmpfs")
        .arg(original_project);
        if let Some(bundle) = bundle {
            cmd.arg("--tmpfs")
                .arg(&project)
                .arg("--tmpfs")
                .arg(&tools)
                .arg("--ro-bind")
                .arg(bundle)
                .arg(bundle);
        }
        cmd.arg("--")
            .arg(executable)
            .current_dir(&empty)
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY");
        cmd
    };
    let hash = good(
        Command::new("sha256sum")
            .arg(&binaries[0])
            .output()
            .unwrap(),
    );
    let hash = hash.split_whitespace().next().unwrap();
    assert_eq!(hash.len(), 64);
    let built_bundle = base.join("new export");
    good(
        isolated(&binaries[1], None)
            .arg("--project")
            .arg(&project)
            .arg("--runtime")
            .arg(&binaries[0])
            .args(["--runtime-sha256", hash, "--trusted-runtime", "--output"])
            .arg(&built_bundle)
            .output()
            .unwrap(),
    );
    let bundle = base.join("relocated readonly bundle");
    std::fs::rename(&built_bundle, &bundle).unwrap();
    assert_eq!(
        std::fs::read(bundle.join("project/level.scene.yaml")).unwrap(),
        scene,
        "export preserves exact scene/2 metadata bytes"
    );
    assert!(!bundle.join("project/actors.prefab.yaml").exists());
    let before = tree_bytes(&bundle);
    for (name, ticks, held) in [
        ("initial", 0, ""),
        ("editor-idle", 20, ""),
        ("step", 16, "right"),
        ("restart", 17, "right,restart"),
    ] {
        let mut results = Vec::new();
        for exported in [false, true] {
            let capture = captures.join(format!(
                "{name}-{}.png",
                if exported { "export" } else { "runtime" }
            ));
            let executable = if exported {
                bundle.join("run-collect-dodge")
            } else {
                binaries[0].clone()
            };
            let mut cmd = isolated(
                &executable,
                if exported {
                    Some(bundle.as_path())
                } else {
                    None
                },
            );
            if !exported {
                cmd.arg("--project").arg(&project);
            }
            cmd.args(["--headless", "--ticks", &ticks.to_string()]);
            if !held.is_empty() {
                cmd.args(["--hold", held]);
            }
            let stdout = good(cmd.arg("--capture").arg(&capture).output().unwrap());
            assert!(stdout.contains("software: true"));
            assert!(stdout.contains(&format!(
                "collect initial checksum: 0x{expected_checksum:016x}"
            )));
            assert!(stdout.contains(&format!("collect tick: {ticks} checksum:")));
            if name == "editor-idle" {
                assert!(
                    stdout.contains(&format!("collect tick: 20 checksum: {idle_checksum}")),
                    "editor/runtime idle replay diverged: {stdout}"
                );
            }
            let bytes = std::fs::read(&capture).unwrap();
            assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
            if name == "initial" {
                let mut reader = png::Decoder::new(std::io::Cursor::new(&bytes))
                    .read_info()
                    .unwrap();
                let mut pixels = vec![0; reader.output_buffer_size()];
                let info = reader.next_frame(&mut pixels).unwrap();
                assert_eq!(info.color_type, png::ColorType::Rgba);
                let mut actors = [0usize; 3];
                for p in pixels[..info.buffer_size()].as_chunks::<4>().0.iter() {
                    if p[2] > 180 && p[0] < 160 && p[1] < 200 {
                        actors[0] += 1;
                    }
                    if p[0] > 180 && p[1] > 180 && p[2] < 120 {
                        actors[1] += 1;
                    }
                    if p[0] > 180 && p[1] < 160 && p[2] < 160 {
                        actors[2] += 1;
                    }
                }
                assert!(
                    actors.iter().all(|n| *n > 30),
                    "source-free runtime must draw actual actors: {actors:?}"
                );
            }
            if let Some(dest) = std::env::var_os("ORR_LINKED_CAPTURES") {
                let dest = PathBuf::from(dest);
                std::fs::create_dir_all(&dest).unwrap();
                std::fs::copy(&capture, dest.join(capture.file_name().unwrap())).unwrap();
                std::fs::write(
                    dest.join(format!(
                        "{name}-{}.txt",
                        if exported { "export" } else { "runtime" }
                    )),
                    &stdout,
                )
                .unwrap();
            }
            results.push((stdout, bytes));
        }
        assert_eq!(results[0], results[1], "{name}: source-free runtime and readonly relocated export must match stdout and real GPU PNG exactly");
    }
    assert_eq!(
        tree_bytes(&bundle),
        before,
        "runtime must not mutate the readonly bundle"
    );
    assert_eq!(
        std::fs::read(original_project.join("level.scene.yaml")).unwrap(),
        scene
    );
    assert_eq!(
        std::fs::read(project.join("level.scene.yaml")).unwrap(),
        scene
    );
    assert!(!original_project.join("actors.prefab.yaml").exists());
}

#[test]
fn actual_widget_rejects_active_scene_with_prefab_filename() {
    let root = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let scene = root.path().join("level.prefab.yaml");
    let original = include_bytes!("../../../scenes/collect_dodge_v1.scene.yaml");
    std::fs::write(&scene, original).unwrap();
    let mut editor = Editor::open_game(&scene, EditorGame::CollectDodge).unwrap();
    editor.sync();
    assert!(editor.select_named("first_collectible"));
    let mut h = Harness::builder()
        .with_size([1600.0, 2200.0])
        .build_eframe(|_| EditorApp::new(editor, None));
    h.state_mut().linked_prefabs.source = "level.prefab.yaml".into();
    h.run_steps(2);
    h.get_by_label("Replace source from selection").click();
    h.run_steps(3);
    assert!(h
        .state()
        .linked_prefabs
        .error()
        .unwrap()
        .contains("active scene"));
    assert_eq!(std::fs::read(&scene).unwrap(), original);
    assert_eq!(h.state().editor.bodies().len(), 4);
}
