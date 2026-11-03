//! Actual Content import/add/verify and model Use/Undo/Redo/Save acceptance.
//! Run with --ignored, ORR_REQUIRE_GPU=1 and a fresh absolute
//! ORR_COOKED_STATIC_IMPORT_CAPTURE_DIR. Missing GPU devices fail, not skip.
#![cfg(feature = "package-manager")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

// Reuse the shipped offline authoring implementations, not a test-built model.
#[allow(dead_code)]
#[path = "../../orr_model/examples/static_model_cylinder/authoring.rs"]
mod cylinder;
#[allow(dead_code)]
#[path = "../../orr_model/examples/static_model_recolor/authoring.rs"]
mod recolor;

use egui::{accesskit::Role, Event};
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use orr_editor::{Editor, EditorApp, HostSpec, Target};
use orr_model::{Dependency, StaticModel};
use orr_model_bindings::model_bindings::{self, Binding, Document, LocalTransform, ModelKind};
use orr_sample::{
    room_game::{RoomActor, RoomConfig, RoomEscapeV1, KEY, PLAYER},
    room_project::{self, PreparedProject, PreparedScene, SEED},
};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const PACKAGE: &str = "cooked-static-import";
const ORIGINAL_PACKAGE: &str = "original-cylinder";
const ASSET: &str = "model.orrmodel";
const CANVAS: &str = "Static package model preview canvas";
const CYLINDER_ID: &str = "props/original.cylinder.json";
const VARIANT_ID: &str = "variants/blue.recolor.json";
const HISTORY_PATH: &str = "inputs/original.model.json";

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn fixture(root: &Path, original_package: &Path, authoring: &Path) -> StaticModel {
    for dir in [root, original_package, authoring] {
        fs::create_dir(dir).unwrap();
    }
    fs::create_dir(authoring.join("inputs")).unwrap();
    let cylinder_recipe = br#"{"format":"orr_static_model_cylinder","version":1,"radius":0.75,"height":2.0,"segments":16}"#;
    let original = cylinder::author_cylinder(cylinder_recipe, CYLINDER_ID).unwrap();
    let original_bytes = original.to_bytes().unwrap();
    fs::write(authoring.join("original.cylinder.json"), cylinder_recipe).unwrap();
    fs::write(authoring.join(HISTORY_PATH), &original_bytes).unwrap();
    let color_recipe = br#"{"format":"orr_static_material_color","version":1,"source":"inputs/original.model.json","material":0,"linear_rgb":[0.0625,0.125,1.0]}"#;
    fs::write(authoring.join("blue.recolor.json"), color_recipe).unwrap();
    let variant = recolor::recolor_model(color_recipe, VARIANT_ID, |path| {
        assert_eq!(path, HISTORY_PATH);
        Ok(original_bytes.clone())
    })
    .unwrap();
    let source = variant.source();
    assert_eq!(source.asset_id, VARIANT_ID);
    assert_eq!(
        source.dependencies,
        vec![
            Dependency {
                uri: "$source".into(),
                sha256: digest(color_recipe)
            },
            Dependency {
                uri: HISTORY_PATH.into(),
                sha256: digest(&original_bytes)
            },
        ],
        "recipe and historical cooked-input digests must remain inside the cooked file"
    );
    // Literal geometry/material controls, independent of package admission.
    assert_eq!(source.primitives.len(), 3);
    assert_eq!(
        source
            .primitives
            .iter()
            .map(|p| p.vertices.len())
            .collect::<Vec<_>>(),
        [34, 17, 17]
    );
    assert_eq!(
        source
            .primitives
            .iter()
            .map(|p| p.indices.len())
            .collect::<Vec<_>>(),
        [96, 48, 48]
    );
    assert_eq!(source.primitives[0].vertices[0].position, [0.75, -1.0, 0.0]);
    assert_eq!(source.primitives[0].vertices[1].position, [0.75, 1.0, 0.0]);
    assert_eq!(source.primitives[1].vertices[0].position, [0.0, 1.0, 0.0]);
    assert_eq!(source.primitives[2].vertices[0].position, [0.0, -1.0, 0.0]);
    assert_eq!(source.materials[0].base_color, [0.0625, 0.125, 1.0, 1.0]);
    assert_eq!(source.materials[1].base_color, [1.0; 4]);
    assert_eq!(source.materials[2].base_color, [1.0; 4]);
    assert_eq!(source.images, original.source().images);
    for (before, after) in original.source().primitives.iter().zip(&source.primitives) {
        let mut expected = before.clone();
        expected.id = format!(
            "{VARIANT_ID}{}",
            before.id.strip_prefix(CYLINDER_ID).unwrap()
        );
        assert_eq!(
            after, &expected,
            "recolor preserves every geometry and transform field"
        );
    }
    fs::write(
        authoring.join("blue.model.json"),
        variant.to_bytes().unwrap(),
    )
    .unwrap();
    fs::write(
        authoring.join("LICENSE.txt"),
        b"Test-owned generated cylinder and color recipe. CC0-1.0.\n",
    )
    .unwrap();
    assert!(!authoring.join("orr.package.json").exists());

    fs::write(original_package.join("original.orrmodel"), original_bytes).unwrap();
    fs::copy(
        authoring.join("LICENSE.txt"),
        original_package.join("LICENSE.txt"),
    )
    .unwrap();
    let manifest = orr_package::Manifest {
        schema: 1,
        name: ORIGINAL_PACKAGE.into(),
        version: "1.0.0".into(),
        engine: "*".into(),
        capabilities: ["models".into()].into_iter().collect(),
        dependencies: BTreeMap::new(),
        files: ["original.orrmodel".into(), "LICENSE.txt".into()]
            .into_iter()
            .collect(),
    };
    fs::write(
        original_package.join("orr.package.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    let sim = orr_sim::Simulation::<RoomEscapeV1>::new(RoomConfig, 60, SEED);
    let text = orr_reflect::Scene::unbake(&room_project::types(), sim.frame(), None)
        .unwrap()
        .to_yaml();
    fs::write(root.join("room.scene.yaml"), &text).unwrap();
    let mut camera = orr_sample::room_camera::Document::readable_default();
    camera.target = [1.0, 0.5, 0.0];
    camera.projection = orr_sample::room_camera::Projection::Orthographic { half_height: 4.0 };
    fs::write(root.join("room.camera.json"), camera.to_bytes().unwrap()).unwrap();
    fs::write(root.join("orr.project.json"), serde_json::to_vec(&serde_json::json!({
        "schema":2,"engine":"*","entry":{"game":"room-escape-v1","scene":"room.scene.yaml","models":"room.models.json","camera":"room.camera.json"}
    })).unwrap()).unwrap();
    orr_package::Project::open_for_install(
        root,
        orr_package::Runtime::content_only().engine_version,
    )
    .unwrap()
    .install(&[original_package.to_path_buf()])
    .unwrap();
    // Only the pre-existing baseline is fixture-authored. The imported variant's
    // binding must be created by the production Use model action below.
    let loaded = model_bindings::load_asset(root, ORIGINAL_PACKAGE, "original.orrmodel").unwrap();
    let binding = Binding::from_asset(
        ORIGINAL_PACKAGE.into(),
        "original.orrmodel".into(),
        &loaded,
        LocalTransform::default(),
    )
    .unwrap();
    let scene = PreparedScene::parse(&text).unwrap();
    let bindings = scene
        .frame()
        .entities()
        .filter(|entity| {
            scene
                .frame()
                .get::<RoomActor>(*entity)
                .is_some_and(|actor| [PLAYER, KEY].contains(&actor.kind))
        })
        .map(|entity| {
            (
                scene.index().guid(entity).unwrap().to_string(),
                binding.clone(),
            )
        })
        .collect();
    fs::write(
        root.join("room.models.json"),
        serde_json::to_vec(&Document {
            version: 2,
            scene: "room.scene.yaml".into(),
            project: ".".into(),
            bindings,
        })
        .unwrap(),
    )
    .unwrap();
    variant
}

fn harness(root: &Path) -> (Harness<'static, EditorApp>, serde_json::Value) {
    let mut prepared = PreparedProject::open(root).unwrap();
    let camera = prepared.take_camera().unwrap();
    let (_, path, scene, models) = prepared.into_parts();
    let mut editor = Editor::start(&HostSpec::PreparedRoom {
        scene: path,
        text: scene.text().into(),
        listen: None,
        debug_hooks: false,
    })
    .unwrap();
    editor.sync();
    editor.select(None);
    editor.install_room_camera(camera.document.clone()).unwrap();
    // Capture the egui device through its real public creation context. The
    // separate production viewport exposes its adapter name, recorded below.
    let adapter = Arc::new(Mutex::new(None));
    let captured_adapter = Arc::clone(&adapter);
    let harness = Harness::builder()
        .with_size([1800.0, 1200.0])
        .wgpu()
        .build_eframe(move |cc| {
            let info = cc
                .wgpu_render_state
                .as_ref()
                .expect("required actual egui renderer")
                .adapter
                .get_info();
            *captured_adapter.lock().unwrap() = Some(serde_json::json!({
                "name": info.name, "backend": format!("{:?}", info.backend),
                "device_type": format!("{:?}", info.device_type),
                "driver": info.driver, "driver_info": info.driver_info,
                "source":"egui creation-context render-state adapter",
            }));
            let mut app = EditorApp::new(editor, cc.wgpu_render_state.clone());
            app.models.install_room(models).unwrap();
            app.room_camera = Some(orr_editor::room_camera_panel::Panel::new(camera));
            app
        });
    let metadata = adapter
        .lock()
        .unwrap()
        .take()
        .expect("actual egui adapter metadata");
    (harness, metadata)
}

fn files(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, dir: &Path, result: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(root, &path, result);
            } else {
                result.insert(
                    path.strip_prefix(root).unwrap().into(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut result = BTreeMap::new();
    visit(root, root, &mut result);
    result
}

fn settle(h: &mut Harness<'_, EditorApp>) {
    for _ in 0..3 {
        h.state_mut().editor.sync();
        h.run_steps(3);
    }
}

fn click_role(h: &mut Harness<'_, EditorApp>, role: Role, label: &str) {
    h.get_by_role_and_label(role, label).scroll_to_me();
    h.run_steps(3);
    assert!(
        !h.get_by_role_and_label(role, label)
            .accesskit_node()
            .is_disabled(),
        "disabled {label}"
    );
    h.get_by_role_and_label(role, label).click_accesskit();
    settle(h);
}

fn click(h: &mut Harness<'_, EditorApp>, label: &str) {
    click_role(h, Role::Button, label);
}

fn toggle(h: &mut Harness<'_, EditorApp>, label: &str) {
    h.get_by_label(label).scroll_to_me();
    h.run_steps(3);
    h.get_by_label(label).click_accesskit();
    settle(h);
}

fn field(h: &mut Harness<'_, EditorApp>, label: &str, value: &str) {
    h.get_by_role_and_label(Role::TextInput, label)
        .scroll_to_me();
    h.run_steps(3);
    h.get_by_role_and_label(Role::TextInput, label).focus();
    h.run_steps(3);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(3);
    h.get_by_role_and_label(Role::TextInput, label)
        .type_text(value);
    h.run_steps(3);
}

fn wait(
    h: &mut Harness<'_, EditorApp>,
    reason: &str,
    ready: impl Fn(&Harness<'_, EditorApp>) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !ready(h) {
        settle(h);
        assert!(
            Instant::now() < deadline,
            "{reason}: {}",
            h.state().package_manager.status
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    settle(h);
}

fn shot(h: &mut Harness<'_, EditorApp>, evidence: &Path, name: &str) {
    let frame = h.render().expect("actual EditorApp GPU frame");
    orr_editor::app::write_png(
        &evidence.join(name),
        &egui::ColorImage::from_rgba_unmultiplied(
            [frame.width() as usize, frame.height() as usize],
            frame.as_raw(),
        ),
    )
    .unwrap();
}

fn preview(
    h: &mut Harness<'_, EditorApp>,
    package: &str,
    asset: &str,
    evidence: &Path,
    name: &str,
) -> Vec<u8> {
    click(h, &format!("Preview static model {package}/{asset}"));
    wait(h, "model preview", |h| h.query_by_label(CANVAS).is_some());
    h.event(Event::PointerMoved(egui::pos2(1790.0, 1190.0)));
    h.run_steps(10);
    let rect = h.get_by_label(CANVAS).rect();
    let scale = h.ctx.pixels_per_point();
    let frame = h.render().expect("actual model preview canvas");
    let x = (rect.left() * scale).round() as u32;
    let y = (rect.top() * scale).round() as u32;
    let width = (rect.width() * scale).round() as u32;
    let height = (rect.height() * scale).round() as u32;
    assert!(width > 0 && height > 0 && width <= 1024 && height <= 1024);
    assert!(x + width <= frame.width() && y + height <= frame.height());
    let mut rgba = Vec::new();
    for row in y..y + height {
        let start = ((row * frame.width() + x) * 4) as usize;
        rgba.extend_from_slice(&frame.as_raw()[start..start + width as usize * 4]);
    }
    orr_editor::app::write_png(
        &evidence.join(name),
        &egui::ColorImage::from_rgba_unmultiplied([width as usize, height as usize], &rgba),
    )
    .unwrap();
    rgba
}

fn viewport(h: &mut Harness<'_, EditorApp>, evidence: &Path, name: &str) -> Vec<u8> {
    let target = (900, 850);
    for _ in 0..8 {
        settle(h);
        let actual = h.state().viewport3d_gpu().unwrap().gpu().target().size();
        if actual == target {
            break;
        }
        let window = h.ctx.viewport_rect().size();
        let ratio = h.ctx.pixels_per_point();
        h.set_size(egui::vec2(
            window.x + (target.0 as f32 - actual.0 as f32) / ratio,
            window.y + (target.1 as f32 - actual.1 as f32) / ratio,
        ));
    }
    settle(h);
    let gpu = h.state().viewport3d_gpu().unwrap().gpu();
    assert_eq!(gpu.target().size(), target);
    let pixels = gpu.read_rgba8();
    assert_eq!(pixels.len(), (target.0 * target.1 * 4) as usize);
    assert!(
        pixels
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|pixel| *pixel != &pixels[..4])
            .count()
            > 1000,
        "nonblank viewport required"
    );
    orr_editor::app::write_png(
        &evidence.join(name),
        &egui::ColorImage::from_rgba_unmultiplied([target.0 as usize, target.1 as usize], &pixels),
    )
    .unwrap();
    pixels
}

fn differences(a: &[u8], b: &[u8]) -> usize {
    assert_eq!(a.len(), b.len());
    a.as_chunks::<4>()
        .0
        .iter()
        .zip(b.as_chunks::<4>().0.iter())
        .filter(|(a, b)| a != b)
        .count()
}

#[test]
#[ignore = "requires a GPU device and fresh ORR_COOKED_STATIC_IMPORT_CAPTURE_DIR"]
fn cooked_authored_variant_import_assign_undo_save_and_installed_only_reopen() {
    assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
    let evidence = PathBuf::from(
        std::env::var_os("ORR_COOKED_STATIC_IMPORT_CAPTURE_DIR").expect("fresh evidence directory"),
    );
    assert!(evidence.is_absolute());
    fs::create_dir(&evidence).expect("capture directory must be new");
    let root = evidence.join("room");
    let original_package = evidence.join("original-package-source");
    let authoring = evidence.join("authoring");
    let package_source = evidence.join("imported-package-source");
    let variant = fixture(&root, &original_package, &authoring);
    let authoring_before = files(&authoring);
    let original_package_before = files(&original_package);
    let project_before = files(&root);
    let lock_before = orr_editor::package_manager::inspect(&root).unwrap();
    let prepared = PreparedProject::open(&root).unwrap();
    let scene_bytes = prepared.scene().frame().to_bytes();
    let checksum = prepared.scene().frame().checksum();
    let key = prepared
        .scene()
        .frame()
        .entities()
        .find(|entity| {
            prepared
                .scene()
                .frame()
                .get::<RoomActor>(*entity)
                .is_some_and(|actor| actor.kind == KEY)
        })
        .unwrap();
    let guid = prepared.scene().index().guid(key).unwrap().clone();
    let bindings_before = prepared.models().document.clone();
    drop(prepared);

    let (mut h, harness_adapter) = harness(&root);
    settle(&mut h);
    assert!(h.state().has_gpu());
    let adapter = h.state().viewport3d_gpu().unwrap().gpu().adapter_name();
    eprintln!("actual egui adapter={harness_adapter}; production viewport adapter={adapter}");
    let history = h.state().editor.history().clone();
    let camera = h.state().editor.camera3d;
    h.get_by_label("File").click();
    h.run_steps(2);
    h.get_by_label("Content packages…").click_accesskit();
    settle(&mut h);
    toggle(&mut h, "Import a model package");
    assert!(
        h.query_by_label("Raw GLB file").is_some(),
        "raw remains the default"
    );
    click_role(&mut h, Role::RadioButton, "Cooked static model");
    field(
        &mut h,
        "Cooked static model file (.model.json)",
        &authoring.join("blue.model.json").display().to_string(),
    );
    field(
        &mut h,
        "License file",
        &authoring.join("LICENSE.txt").display().to_string(),
    );
    field(
        &mut h,
        "Package destination directory",
        &package_source.display().to_string(),
    );
    field(&mut h, "Package name", PACKAGE);
    field(&mut h, "Package version", "1.0.0");
    assert!(h
        .get_by_label("Import cooked static package")
        .accesskit_node()
        .is_disabled());
    assert_eq!(files(&root), project_before);
    assert!(!package_source.exists());
    click_role(
        &mut h,
        Role::CheckBox,
        "I have reviewed rights to distribute this content",
    );
    shot(&mut h, &evidence, "01-explicit-cooked-import.png");
    click(&mut h, "Import cooked static package");
    wait(&mut h, "cooked import worker", |h| {
        !h.state().package_manager.busy()
    });
    let status = h.state().package_manager.status.clone();
    assert!(status.contains(&format!("{PACKAGE}/{ASSET}")), "{status}");
    assert!(status.contains("not installed or assigned"), "{status}");
    let cooked_bytes = &authoring_before[&PathBuf::from("blue.model.json")];
    let license_bytes = &authoring_before[&PathBuf::from("LICENSE.txt")];
    assert!(status.contains(&digest(cooked_bytes)) && status.contains(&digest(license_bytes)));
    assert_eq!(
        Path::new(&h.state().package_manager.source)
            .canonicalize()
            .unwrap(),
        package_source.canonicalize().unwrap()
    );
    assert_eq!(fs::read(package_source.join(ASSET)).unwrap(), *cooked_bytes);
    assert_eq!(
        fs::read(package_source.join("LICENSE.txt")).unwrap(),
        *license_bytes
    );
    let manifest: orr_package::Manifest =
        serde_json::from_slice(&fs::read(package_source.join("orr.package.json")).unwrap())
            .unwrap();
    assert_eq!(manifest.name, PACKAGE);
    assert_eq!(manifest.version, "1.0.0");
    assert_eq!(
        manifest.capabilities,
        ["models".into()].into_iter().collect()
    );
    assert!(manifest.dependencies.is_empty());
    assert_eq!(
        manifest.files,
        [ASSET.into(), "LICENSE.txt".into()].into_iter().collect()
    );
    assert!(
        !package_source.join(HISTORY_PATH).exists(),
        "historical source dependencies are metadata, not package files"
    );
    assert_eq!(
        StaticModel::from_bytes(&fs::read(package_source.join(ASSET)).unwrap())
            .unwrap()
            .source(),
        variant.source()
    );
    let package_before = files(&package_source);
    assert_eq!(
        files(&root),
        project_before,
        "import must not install, assign or save"
    );
    assert_eq!(
        h.state().models.bindings.as_ref().unwrap().document(),
        &bindings_before
    );
    assert_eq!(h.state().editor.checksum(), checksum);
    shot(&mut h, &evidence, "02-imported-not-installed.png");
    toggle(&mut h, "Import a model package");

    click(&mut h, "Preview package addition");
    wait(&mut h, "package preview", |h| {
        !h.state().package_manager.busy()
    });
    assert!(h
        .state()
        .package_manager
        .status
        .starts_with("Preview ready. No package was added"));
    assert_eq!(files(&root), project_before, "source preview is read-only");
    shot(&mut h, &evidence, "03-package-preview-read-only.png");
    click(&mut h, "Add previewed package");
    wait(&mut h, "package addition", |h| {
        !h.state().package_manager.busy()
    });
    assert!(h
        .state()
        .package_manager
        .status
        .starts_with("Package added."));
    let lock_added = orr_editor::package_manager::inspect(&root).unwrap();
    assert_eq!(lock_added.packages.len(), lock_before.packages.len() + 1);
    assert_eq!(
        lock_added.packages[ORIGINAL_PACKAGE],
        lock_before.packages[ORIGINAL_PACKAGE]
    );
    for (path, bytes) in &project_before {
        if path != Path::new("orr.packages.lock.json") {
            assert_eq!(
                fs::read(root.join(path)).unwrap(),
                *bytes,
                "existing file preserved: {}",
                path.display()
            );
        }
    }
    assert!(
        h.query_by_label(&format!("Preview static model {PACKAGE}/{ASSET}"))
            .is_none(),
        "Add must not bypass explicit verification"
    );
    click(&mut h, "Verify installed packages");
    wait(&mut h, "installed package verification", |h| {
        !h.state().package_manager.busy()
    });
    assert!(h
        .state()
        .package_manager
        .status
        .starts_with("Installed package bytes match"));
    let project_added = files(&root);
    shot(&mut h, &evidence, "04-installed-verified.png");
    let original_pixels = preview(
        &mut h,
        ORIGINAL_PACKAGE,
        "original.orrmodel",
        &evidence,
        "05-original-preview-canvas.png",
    );
    let variant_pixels = preview(
        &mut h,
        PACKAGE,
        ASSET,
        &evidence,
        "06-authored-preview-canvas.png",
    );
    let preview_changed = differences(&original_pixels, &variant_pixels);
    assert!(
        preview_changed > 1000,
        "authored recolor must visibly change real preview: {preview_changed}"
    );
    shot(&mut h, &evidence, "06-authored-preview.png");
    click(&mut h, "Close static preview");
    assert_eq!(files(&root), project_added);
    assert_eq!(
        h.state().models.bindings.as_ref().unwrap().document(),
        &bindings_before
    );
    assert!(!h.state().models.bindings.as_ref().unwrap().dirty());
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history(), &history);
    assert_eq!(h.state().editor.camera3d, camera);

    // Actual hierarchy selection and production package Use action. Do not call
    // assign(), commit_assignment(), or synthesize the imported binding in test code.
    let label = h
        .state()
        .editor
        .rows()
        .iter()
        .find(|row| row.guid.as_ref() == Some(&guid))
        .unwrap()
        .label();
    toggle(&mut h, &label);
    assert_eq!(
        h.state().editor.selection(),
        Some(&Target::Guid(guid.clone()))
    );
    click(&mut h, &format!("Use model {PACKAGE}/{ASSET}"));
    assert!(h.state().package_manager.status.starts_with(&format!(
        "Assigned verified static model {PACKAGE}/{ASSET}."
    )));
    let assigned = h
        .state()
        .models
        .bindings
        .as_ref()
        .unwrap()
        .document()
        .bindings[&guid.to_string()]
        .clone();
    assert_eq!(assigned.package, PACKAGE);
    assert_eq!(assigned.asset, ASSET);
    assert_eq!(assigned.kind, ModelKind::Static);
    assert!(assigned.animation.is_none());
    assert_eq!(assigned.transform, LocalTransform::default());
    let loaded = model_bindings::load_asset(&root, PACKAGE, ASSET).unwrap();
    assigned.validate(&loaded).unwrap();
    assert_eq!(loaded.static_model().unwrap().source(), variant.source());
    assert_eq!(assigned.source_hash, loaded.source_hash());
    let mut expected = bindings_before.clone();
    expected.bindings.insert(guid.to_string(), assigned.clone());
    assert_eq!(
        h.state().models.bindings.as_ref().unwrap().document(),
        &expected
    );
    assert!(h.state().models.bindings.as_ref().unwrap().dirty());
    assert_eq!(
        files(&root),
        project_added,
        "Use changes only unsaved bindings"
    );
    h.state_mut().package_manager.open = false;
    settle(&mut h);
    if h.query_by_label("Save model bindings").is_none() {
        toggle(&mut h, "Static model binding");
    }
    shot(&mut h, &evidence, "07-production-assignment.png");
    click(&mut h, "Undo model");
    assert_eq!(
        h.state().models.bindings.as_ref().unwrap().document(),
        &bindings_before
    );
    assert_eq!(files(&root), project_added);
    let undo_pixels = viewport(&mut h, &evidence, "08-undo-viewport.png");
    click(&mut h, "Redo model");
    assert_eq!(
        h.state().models.bindings.as_ref().unwrap().document(),
        &expected
    );
    assert_eq!(files(&root), project_added);
    let redo_pixels = viewport(&mut h, &evidence, "09-redo-viewport.png");
    let assigned_changed = differences(&undo_pixels, &redo_pixels);
    assert!(
        assigned_changed > 1000,
        "actual assignment must visibly recolor KEY: {assigned_changed}"
    );
    click(&mut h, "Save model bindings");
    assert!(!h.state().models.bindings.as_ref().unwrap().dirty());
    let saved_document: Document =
        serde_json::from_slice(&fs::read(root.join("room.models.json")).unwrap()).unwrap();
    assert_eq!(saved_document, expected);
    let mut project_saved = project_added.clone();
    project_saved.insert(
        PathBuf::from("room.models.json"),
        fs::read(root.join("room.models.json")).unwrap(),
    );
    assert_eq!(
        files(&root),
        project_saved,
        "Save may change only model bindings"
    );
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history(), &history);
    h.state_mut().editor.select(None);
    settle(&mut h);
    let saved_pixels = viewport(&mut h, &evidence, "10-saved-viewport.png");
    shot(&mut h, &evidence, "10-saved.png");
    drop(h);

    let hidden_authoring = evidence.join("hidden-authoring");
    let hidden_package = evidence.join("hidden-imported-package");
    let hidden_original = evidence.join("hidden-original-package");
    fs::rename(&authoring, &hidden_authoring).unwrap();
    fs::rename(&package_source, &hidden_package).unwrap();
    fs::rename(&original_package, &hidden_original).unwrap();
    assert!(!authoring.exists() && !package_source.exists() && !original_package.exists());
    assert_eq!(files(&hidden_authoring), authoring_before);
    assert_eq!(files(&hidden_package), package_before);
    assert_eq!(files(&hidden_original), original_package_before);
    let prepared = PreparedProject::open(&root).unwrap();
    assert_eq!(prepared.scene().frame().to_bytes(), scene_bytes);
    assert_eq!(prepared.models().document, expected);
    let reopened_model = model_bindings::load_binding(&root, &assigned).unwrap();
    assert_eq!(
        reopened_model.static_model().unwrap().source(),
        variant.source()
    );
    assert_eq!(reopened_model.source_hash(), loaded.source_hash());
    drop(prepared);
    let (mut reopened, reopened_harness_adapter) = harness(&root);
    settle(&mut reopened);
    assert_eq!(
        reopened
            .state()
            .models
            .bindings
            .as_ref()
            .unwrap()
            .document(),
        &expected
    );
    assert!(!reopened.state().models.bindings.as_ref().unwrap().dirty());
    assert_eq!(reopened.state().editor.checksum(), checksum);
    assert_eq!(reopened.state().editor.history(), &history);
    assert_eq!(
        reopened
            .state()
            .viewport3d_gpu()
            .unwrap()
            .gpu()
            .adapter_name(),
        adapter
    );
    let reopened_pixels = viewport(&mut reopened, &evidence, "11-reopened-viewport.png");
    assert_eq!(
        reopened_pixels, saved_pixels,
        "fresh host/app/caches must reopen exactly from installed bytes"
    );
    shot(&mut reopened, &evidence, "11-reopened.png");
    assert_eq!(files(&root), project_saved);
    assert_eq!(
        orr_editor::package_manager::inspect(&root).unwrap(),
        lock_added
    );
    fs::write(evidence.join("receipt.json"), serde_json::to_vec_pretty(&serde_json::json!({
        "schema":1, "status":"passed", "test":"cooked_authored_variant_import_assign_undo_save_and_installed_only_reopen",
        "mandatory_test_count":1, "test_source_sha256":digest(include_bytes!("cooked_static_import_ui.rs")),
        "cylinder_helper_sha256":digest(include_bytes!("../../orr_model/examples/static_model_cylinder/authoring.rs")),
        "recolor_helper_sha256":digest(include_bytes!("../../orr_model/examples/static_model_recolor/authoring.rs")),
        "viewport_adapter_name":adapter, "viewport_adapter_source":"production viewport3d_gpu adapter_name()",
        "harness_adapter":harness_adapter, "reopened_harness_adapter":reopened_harness_adapter,
        "harness_setup":"normal Harness::builder().wgpu()",
        "scope":"offscreen EditorApp; no native-window, hardware-coverage or export claim",
        "source_sha256":digest(cooked_bytes), "license_sha256":digest(license_bytes),
        "variant_asset_id":VARIANT_ID, "historical_dependencies":variant.source().dependencies,
        "package_digest":lock_added.packages[PACKAGE].digest, "assigned_guid":guid.to_string(),
        "assigned_role":"KEY", "actual_use_assignment":assigned,
        "literal_vertex_counts":[34,17,17], "literal_index_counts":[96,48,48],
        "preview_changed_pixels":preview_changed, "assigned_changed_pixels":assigned_changed,
        "source_import_only":true, "package_preview_read_only":true, "explicit_add_and_verify":true,
        "model_preview_read_only":true, "undo_redo_save":true,
        "source_license_original_package_unchanged":true, "scene_frame_bytes_unchanged":true,
        "all_source_directories_hidden":true, "fresh_prepared_project_exact":true,
        "fresh_reopen_viewport_pixels_exact":true, "viewport_size":[900,850],
        "saved_viewport_sha256":digest(&saved_pixels), "reopened_viewport_sha256":digest(&reopened_pixels)
    })).unwrap()).unwrap();
}
