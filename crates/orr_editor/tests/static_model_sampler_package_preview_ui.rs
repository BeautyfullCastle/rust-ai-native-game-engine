//! Mandatory real EditorApp acceptance for installed offline sampler variants.
//! Supply a current-source ORR_STATIC_MODEL_SAMPLER_BIN and a fresh absolute
//! ORR_STATIC_MODEL_SAMPLER_PREVIEW_CAPTURE_DIR, then run with --ignored.
//! Missing software Vulkan, CLI, or capture evidence fails rather than skips.
#![cfg(feature = "package-manager")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
use egui::{accesskit::Role, Event, Modifiers};
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use egui_wgpu::{wgpu, WgpuSetupCreateNew};
use orr_editor::{Editor, EditorApp, HostSpec};
use orr_model_bindings::model_bindings::{self, Binding, Document, LocalTransform};
use orr_sample::{
    room_game::{RoomActor, RoomConfig, RoomEscapeV1, KEY, PLAYER},
    room_project::{self, PreparedProject, PreparedScene, SEED},
};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
fn fixture(root: &Path, source: &Path, authoring: &Path, evidence: &Path) {
    fs::create_dir(root).unwrap();
    let sim = orr_sim::Simulation::<RoomEscapeV1>::new(RoomConfig, 60, SEED);
    let text = orr_reflect::Scene::unbake(&room_project::types(), sim.frame(), None)
        .unwrap()
        .to_yaml();
    fs::write(root.join("room.scene.yaml"), &text).unwrap();
    fs::write(
        root.join("room.camera.json"),
        orr_sample::room_camera::Document::readable_default()
            .to_bytes()
            .unwrap(),
    )
    .unwrap();
    fs::write(root.join("orr.project.json"),serde_json::to_vec(&serde_json::json!({"schema":2,"engine":"*","entry":{"game":"room-escape-v1","scene":"room.scene.yaml","models":"room.models.json","camera":"room.camera.json"}})).unwrap()).unwrap();
    fs::create_dir(source).unwrap();
    fs::create_dir_all(authoring).unwrap();
    let imported = orr_model::import::import_with_resolver(
        "props/original.glb",
        include_bytes!("../../orr_model/tests/fixtures/model.glb"),
        |_| panic!("embedded fixture must not resolve external resources"),
    )
    .unwrap();
    let mut model = imported.source().clone();
    model.materials[0].base_color = [1.0, 1.0, 1.0, 0.25];
    model.materials[0].linear_filter = false;
    model.materials[0].wrap_s = orr_model::Wrap::Clamp;
    model.materials[0].wrap_t = orr_model::Wrap::Clamp;
    let image = model.materials[0].image as usize;
    model.images[image] = orr_model::Image {
        width: 3,
        height: 2,
        rgba8: vec![
            255, 20, 10, 255, 10, 255, 40, 255, 30, 40, 255, 255, 255, 255, 10, 255, 240, 10, 230,
            255, 10, 230, 240, 255,
        ],
    };
    for primitive in &mut model.primitives {
        if primitive.material == 0 {
            for vertex in &mut primitive.vertices {
                vertex.uv = [vertex.uv[0] * 3.0 - 0.75, vertex.uv[1] * 2.5 - 0.5];
            }
        }
    }
    let original = orr_model::StaticModel::new(model).unwrap();
    let original_bytes = original.to_bytes().unwrap();
    fs::write(authoring.join("source.model.json"), &original_bytes).unwrap();
    fs::write(source.join("original.orrmodel"), &original_bytes).unwrap();
    let binary = PathBuf::from(
        std::env::var_os("ORR_STATIC_MODEL_SAMPLER_BIN")
            .expect("current-source static_model_sampler example binary is required"),
    );
    assert!(binary.is_absolute() && binary.is_file());
    for (name, linear_filter) in [("nearest", false), ("linear", true)] {
        let recipe = authoring.join(format!("{name}.json"));
        fs::write(
            &recipe,
            serde_json::to_vec(&serde_json::json!({
                "format": "orr_static_material_sampler", "version": 1,
                "source": "source.model.json", "material": 0, "linear_filter": linear_filter, "wrap_s": "repeat", "wrap_t": "mirror"
            }))
            .unwrap(),
        )
        .unwrap();
        let id = format!("variants/{name}.sampler.json");
        let output = std::process::Command::new(&binary)
            .arg("--recipe")
            .arg(&recipe)
            .arg("--asset-id")
            .arg(&id)
            .arg("--output")
            .arg(authoring.join(format!("{name}.model.json")))
            .output()
            .unwrap();
        fs::write(
            evidence.join(format!("cli-{name}.stdout.log")),
            &output.stdout,
        )
        .unwrap();
        fs::write(
            evidence.join(format!("cli-{name}.stderr.log")),
            &output.stderr,
        )
        .unwrap();
        assert!(
            output.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        // Existing package browser admits .orrmodel. Package exact CLI bytes,
        // without conversion or a second authoring implementation.
        let cooked = fs::read(authoring.join(format!("{name}.model.json"))).unwrap();
        fs::write(source.join(format!("{name}.orrmodel")), &cooked).unwrap();
        assert_eq!(
            fs::read(source.join(format!("{name}.orrmodel"))).unwrap(),
            cooked
        );
        let variant = orr_model::StaticModel::from_bytes(
            &fs::read(authoring.join(format!("{name}.model.json"))).unwrap(),
        )
        .unwrap();
        let actual = variant.source();
        assert_eq!(actual.asset_id, id);
        assert_ne!(actual.dependencies, original.source().dependencies);
        assert_eq!(actual.images, original.source().images);
        assert_eq!(actual.materials.len(), original.source().materials.len());
        for (index, (before, after)) in original
            .source()
            .materials
            .iter()
            .zip(&actual.materials)
            .enumerate()
        {
            let mut expected = before.clone();
            if index == 0 {
                expected.linear_filter = linear_filter;
                expected.wrap_s = orr_model::Wrap::Repeat;
                expected.wrap_t = orr_model::Wrap::Mirror;
            }
            assert_eq!(after, &expected, "only selected sampler fields may change");
        }
        assert_eq!(actual.primitives.len(), original.source().primitives.len());
        for (before, after) in original.source().primitives.iter().zip(&actual.primitives) {
            let mut expected = before.clone();
            expected.id = format!(
                "{id}{}",
                before.id.strip_prefix(&original.source().asset_id).unwrap()
            );
            assert_eq!(
                after, &expected,
                "geometry, UV, normal, transform and material slot remain exact"
            );
        }
    }
    let manifest = orr_package::Manifest {
        schema: 1,
        name: PACKAGE.into(),
        version: "1.0.0".into(),
        engine: "*".into(),
        capabilities: ["models".into()].into_iter().collect(),
        dependencies: BTreeMap::new(),
        files: ["original.orrmodel", "nearest.orrmodel", "linear.orrmodel"]
            .into_iter()
            .map(str::to_owned)
            .collect(),
    };
    fs::write(
        source.join("orr.package.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    orr_package::Project::open_for_install(
        root,
        orr_package::Runtime::content_only().engine_version,
    )
    .unwrap()
    .install(&[source.to_path_buf()])
    .unwrap();
    let loaded =
        model_bindings::load_asset(root, "static-sampler-preview", "original.orrmodel").unwrap();
    let binding = Binding::from_asset(
        "static-sampler-preview".into(),
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
}
fn harness(root: &Path) -> Harness<'static, EditorApp> {
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
    let mut setup = WgpuSetupCreateNew::without_display_handle();
    setup.instance_descriptor.backends = wgpu::Backends::VULKAN;
    setup.native_adapter_selector = Some(Arc::new(|adapters, _| {
        adapters
            .iter()
            .find(|adapter| {
                adapter.get_info().backend == wgpu::Backend::Vulkan
                    && adapter.get_info().device_type == wgpu::DeviceType::Cpu
            })
            .cloned()
            .ok_or_else(|| "software Vulkan required".into())
    }));
    Harness::builder()
        .with_size([1800.0, 1200.0])
        .wgpu_setup(setup.into())
        .build_eframe(move |cc| {
            let mut app = EditorApp::new(editor, cc.wgpu_render_state.clone());
            app.models.install_room(models).unwrap();
            app.room_camera = Some(orr_editor::room_camera_panel::Panel::new(camera));
            app
        })
}

const CANVAS: &str = "Static package model preview canvas";
const PACKAGE: &str = "static-sampler-preview";

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
    h.state_mut().editor.sync();
    h.run_steps(5);
}

fn click(h: &mut Harness<'_, EditorApp>, label: &str) {
    h.get_by_role_and_label(Role::Button, label).scroll_to_me();
    h.run_steps(8);
    assert!(
        !h.get_by_role_and_label(Role::Button, label)
            .accesskit_node()
            .is_disabled(),
        "disabled {label}"
    );
    h.get_by_role_and_label(Role::Button, label)
        .click_accesskit();
    settle(h);
}

fn wait(h: &mut Harness<'_, EditorApp>, condition: impl Fn(&Harness<'_, EditorApp>) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !condition(h) {
        settle(h);
        assert!(Instant::now() < deadline, "package preview timed out");
        std::thread::sleep(Duration::from_millis(10));
    }
    settle(h);
}

fn verify(h: &mut Harness<'_, EditorApp>) {
    click(h, "Verify installed packages");
    wait(h, |h| !h.state().package_manager.busy());
    assert!(h
        .state()
        .package_manager
        .status
        .starts_with("Installed package bytes match"));
}

fn preview(h: &mut Harness<'_, EditorApp>, asset: &str) {
    click(h, &format!("Preview static model {PACKAGE}/{asset}"));
    wait(h, |h| h.query_by_label(CANVAS).is_some());
    h.event(Event::PointerMoved(egui::pos2(1790.0, 1190.0)));
    h.run_steps(10);
}

fn shot(h: &mut Harness<'_, EditorApp>, evidence: &Path, name: &str) {
    let frame = h.render().expect("production EditorApp GPU frame");
    orr_editor::app::write_png(
        &evidence.join(name),
        &egui::ColorImage::from_rgba_unmultiplied(
            [frame.width() as usize, frame.height() as usize],
            frame.as_raw(),
        ),
    )
    .unwrap();
}

fn crop(h: &mut Harness<'_, EditorApp>, evidence: &Path, name: &str) -> Vec<u8> {
    let rect = h.get_by_label(CANVAS).rect();
    let scale = h.ctx.pixels_per_point();
    let frame = h.render().expect("production EditorApp preview canvas");
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

fn viewport_pixels(h: &mut Harness<'_, EditorApp>, evidence: &Path, name: &str) -> Vec<u8> {
    // Normalize actual render targets; never resample or crop the comparison.
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
    let bytes = gpu.read_rgba8();
    assert_eq!(bytes.len(), (target.0 * target.1 * 4) as usize);
    assert!(
        bytes
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|pixel| *pixel != &bytes[..4])
            .count()
            > 1000,
        "nonblank production viewport required"
    );
    orr_editor::app::write_png(
        &evidence.join(name),
        &egui::ColorImage::from_rgba_unmultiplied([target.0 as usize, target.1 as usize], &bytes),
    )
    .unwrap();
    bytes
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

fn orbit(h: &mut Harness<'_, EditorApp>) {
    let start = h.get_by_label(CANVAS).rect().center();
    h.event(Event::PointerMoved(start));
    h.step();
    h.event(Event::PointerButton {
        pos: start,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.step();
    for step in 1..=5 {
        h.event(Event::PointerMoved(
            start + egui::vec2(15.0 * step as f32, 7.0 * step as f32),
        ));
        h.step();
    }
    h.event(Event::PointerButton {
        pos: start + egui::vec2(75.0, 35.0),
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    h.event(Event::PointerMoved(egui::pos2(1790.0, 1190.0)));
    h.run_steps(10);
}

#[test]
#[ignore = "requires software Vulkan and fresh ORR_STATIC_MODEL_SAMPLER_PREVIEW_CAPTURE_DIR"]
fn installed_sampler_variants_preview_read_only_and_reopen_identity() {
    let evidence = PathBuf::from(
        std::env::var_os("ORR_STATIC_MODEL_SAMPLER_PREVIEW_CAPTURE_DIR")
            .expect("fresh evidence directory"),
    );
    assert!(evidence.is_absolute());
    fs::create_dir(&evidence).unwrap();
    let root = evidence.join("room");
    let source = evidence.join("owned-package-copy");
    let authoring = evidence.join("authoring");
    fixture(&root, &source, &authoring, &evidence);
    let original = files(&root);
    let source_bytes = files(&source);
    let authoring_bytes = files(&authoring);
    let mut h = harness(&root);
    settle(&mut h);
    let adapter = h.state().viewport3d_gpu().unwrap().gpu().adapter_name();
    assert!(
        adapter.to_lowercase().contains("llvmpipe"),
        "mandatory llvmpipe: {adapter}"
    );
    let checksum = h.state().editor.checksum();
    let history = h.state().editor.history().clone();
    let camera = h.state().editor.camera3d;
    let bindings = h
        .state()
        .models
        .bindings
        .as_ref()
        .unwrap()
        .document()
        .clone();
    assert!(h.state().editor.selection().is_none());
    h.get_by_label("File").click();
    h.run_steps(2);
    h.get_by_label("Content packages…").click_accesskit();
    settle(&mut h);
    assert!(
        h.query_by_label(&format!("Preview static model {PACKAGE}/original.orrmodel"))
            .is_none(),
        "unverified inventory cannot preview"
    );
    verify(&mut h);
    shot(&mut h, &evidence, "01-verified.png");
    let hidden_source = evidence.join("hidden-package-source");
    let hidden_authoring = evidence.join("hidden-authoring");
    fs::rename(&source, &hidden_source).unwrap();
    fs::rename(&authoring, &hidden_authoring).unwrap();
    assert!(!source.exists() && !authoring.exists());
    let original_loaded = model_bindings::load_asset(&root, PACKAGE, "original.orrmodel").unwrap();
    let nearest_loaded = model_bindings::load_asset(&root, PACKAGE, "nearest.orrmodel").unwrap();
    let linear_loaded = model_bindings::load_asset(&root, PACKAGE, "linear.orrmodel").unwrap();
    assert_eq!(
        nearest_loaded.static_model().unwrap().source().asset_id,
        "variants/nearest.sampler.json"
    );
    assert_eq!(
        linear_loaded.static_model().unwrap().source().asset_id,
        "variants/linear.sampler.json"
    );
    assert_ne!(original_loaded.source_hash(), nearest_loaded.source_hash());
    assert_ne!(nearest_loaded.source_hash(), linear_loaded.source_hash());
    assert_ne!(
        original_loaded.static_model().unwrap().source().asset_id,
        nearest_loaded.static_model().unwrap().source().asset_id
    );
    assert_ne!(
        nearest_loaded.static_model().unwrap().source().asset_id,
        linear_loaded.static_model().unwrap().source().asset_id
    );

    preview(&mut h, "original.orrmodel");
    shot(&mut h, &evidence, "02-original.png");
    let foreground = crop(&mut h, &evidence, "02-original-canvas.png");
    preview(&mut h, "nearest.orrmodel");
    shot(&mut h, &evidence, "03-nearest.png");
    let background = crop(&mut h, &evidence, "03-nearest-canvas.png");
    let different_assets = differences(&foreground, &background);
    assert!(
        different_assets > 1000,
        "same geometry with repeat/mirror must visibly differ from clamped sampling, got {different_assets}"
    );
    preview(&mut h, "linear.orrmodel");
    shot(&mut h, &evidence, "03-linear.png");
    let linear = crop(&mut h, &evidence, "03-linear-canvas.png");
    let variant_pixels = differences(&background, &linear);
    assert!(
        variant_pixels > 1000,
        "nearest and linear filtering must visibly differ: {variant_pixels}"
    );
    preview(&mut h, "nearest.orrmodel");
    orbit(&mut h);
    shot(&mut h, &evidence, "04-orbit.png");
    let orbited = crop(&mut h, &evidence, "04-orbit-canvas.png");
    let orbit_pixels = differences(&background, &orbited);
    assert!(orbit_pixels > 1000, "orbit must visibly change the preview");
    let center = h.get_by_label(CANVAS).rect().center();
    h.event(Event::PointerMoved(center));
    h.step();
    h.event(Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, 100.0),
        modifiers: Modifiers::NONE,
        phase: egui::TouchPhase::Move,
    });
    h.run_steps(40);
    h.event(Event::PointerMoved(egui::pos2(1790.0, 1190.0)));
    h.run_steps(10);
    let zoomed = crop(&mut h, &evidence, "05-zoom-canvas.png");
    let zoom_pixels = differences(&orbited, &zoomed);
    assert!(zoom_pixels > 1000, "zoom must visibly change the preview");
    click(&mut h, "Reset preview view");
    h.run_steps(10);
    let reset = crop(&mut h, &evidence, "06-reset-canvas.png");
    assert_eq!(
        reset, background,
        "Reset restores the exact default asset camera"
    );
    shot(&mut h, &evidence, "06-reset.png");
    click(&mut h, "Close static preview");
    assert!(h.query_by_label(CANVAS).is_none());
    assert!(h.query_by_label("Static package model preview").is_none());
    shot(&mut h, &evidence, "07-closed.png");
    assert!(h.state().editor.selection().is_none());
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history(), &history);
    assert_eq!(h.state().editor.camera3d, camera);
    assert_eq!(
        h.state().models.bindings.as_ref().unwrap().document(),
        &bindings
    );
    assert!(!h.state().models.bindings.as_ref().unwrap().dirty());
    assert_eq!(files(&root), original);
    assert_eq!(files(&hidden_source), source_bytes);

    // Tamper only with this test's own installed copy after Verify. The button
    // still exists, but the asynchronous loader must refuse the changed bytes.
    let lock = orr_editor::package_manager::inspect(&root).unwrap();
    let cached = root
        .join(".orr/packages/objects")
        .join(&lock.packages[PACKAGE].digest)
        .join("nearest.orrmodel");
    let good_bytes = fs::read(&cached).unwrap();
    fs::write(&cached, b"corrupt isolated acceptance fixture").unwrap();
    click(
        &mut h,
        &format!("Preview static model {PACKAGE}/nearest.orrmodel"),
    );
    wait(&mut h, |h| {
        h.query_by_label_contains("Static preview unavailable:")
            .is_some()
    });
    assert!(h.query_by_label(CANVAS).is_none());
    shot(&mut h, &evidence, "08-corrupt-refused.png");
    click(&mut h, "Close static preview");
    fs::write(&cached, &good_bytes).unwrap();
    verify(&mut h);
    fs::remove_file(&cached).unwrap();
    click(
        &mut h,
        &format!("Preview static model {PACKAGE}/nearest.orrmodel"),
    );
    wait(&mut h, |h| {
        h.query_by_label_contains("Static preview unavailable:")
            .is_some()
    });
    assert!(h.query_by_label(CANVAS).is_none());
    shot(&mut h, &evidence, "08-removed-refused.png");
    click(&mut h, "Close static preview");
    fs::write(&cached, good_bytes).unwrap();
    verify(&mut h);
    assert_eq!(files(&root), original);

    // Assignment remains a separate existing action and changes only the model
    // sidecar after its explicit Save. It does not mutate scene history/Frame.
    let prepared = PreparedProject::open(&root).unwrap();
    let scene = prepared.scene();
    let player = scene
        .frame()
        .entities()
        .find(|entity| {
            scene
                .frame()
                .get::<RoomActor>(*entity)
                .is_some_and(|actor| actor.kind == PLAYER)
        })
        .unwrap();
    let guid = scene.index().guid(player).unwrap().clone();
    h.state_mut()
        .editor
        .select(Some(orr_editor::Target::Guid(guid.clone())));
    settle(&mut h);
    click(&mut h, &format!("Use model {PACKAGE}/nearest.orrmodel"));
    assert!(h.state().models.bindings.as_ref().unwrap().dirty());
    assert_eq!(
        h.state()
            .models
            .bindings
            .as_ref()
            .unwrap()
            .document()
            .bindings[&guid.to_string()]
            .asset,
        "nearest.orrmodel"
    );
    let assigned = h
        .state()
        .models
        .bindings
        .as_ref()
        .unwrap()
        .document()
        .bindings[&guid.to_string()]
        .clone();
    let mut expected_bindings = bindings.clone();
    expected_bindings
        .bindings
        .insert(guid.to_string(), assigned.clone());
    assert_eq!(
        h.state().models.bindings.as_ref().unwrap().document(),
        &expected_bindings,
        "only selected Room GUID binding changes"
    );
    assert_eq!(assigned.transform, LocalTransform::default());
    assert_eq!(assigned.package, PACKAGE);
    assert_eq!(assigned.source_hash, nearest_loaded.source_hash());
    assigned.validate(&nearest_loaded).unwrap();
    h.state_mut().package_manager.open = false;
    settle(&mut h);
    if h.query_by_label("Save model bindings").is_none() {
        h.get_by_label("Static model binding").click_accesskit();
        settle(&mut h);
    }
    click(&mut h, "Save model bindings");
    assert!(!h.state().models.bindings.as_ref().unwrap().dirty());
    shot(&mut h, &evidence, "09-assigned-saved.png");
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history(), &history);
    for name in [
        "room.scene.yaml",
        "room.camera.json",
        "orr.project.json",
        "orr.packages.lock.json",
    ] {
        if let Some(bytes) = original.get(Path::new(name)) {
            assert_eq!(&fs::read(root.join(name)).unwrap(), bytes);
        }
    }
    h.state_mut().editor.select(None);
    settle(&mut h);
    let saved_pixels = viewport_pixels(&mut h, &evidence, "09-saved-viewport.png");
    let mut expected_files = original.clone();
    expected_files.insert(
        PathBuf::from("room.models.json"),
        fs::read(root.join("room.models.json")).unwrap(),
    );
    assert_eq!(
        files(&root),
        expected_files,
        "only explicit model sidecar Save may write"
    );
    drop(h);
    let mut h = harness(&root);
    settle(&mut h);
    assert_eq!(
        h.state()
            .models
            .bindings
            .as_ref()
            .unwrap()
            .document()
            .bindings[&guid.to_string()]
            .asset,
        "nearest.orrmodel"
    );
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history(), &history);
    assert_eq!(
        h.state()
            .models
            .bindings
            .as_ref()
            .unwrap()
            .document()
            .bindings[&guid.to_string()],
        assigned
    );
    assert!(!h.state().models.bindings.as_ref().unwrap().dirty());
    let reopened = model_bindings::load_binding(&root, &assigned).unwrap();
    assert_eq!(
        reopened.static_model().unwrap().source(),
        nearest_loaded.static_model().unwrap().source()
    );
    assert_eq!(reopened.source_hash(), nearest_loaded.source_hash());
    let reopened_pixels = viewport_pixels(&mut h, &evidence, "10-reopened-viewport.png");
    assert_eq!(
        saved_pixels, reopened_pixels,
        "fresh app preserves exact normalized viewport pixels"
    );
    shot(&mut h, &evidence, "10-reopened.png");
    assert_eq!(
        files(&root),
        expected_files,
        "fresh reopen remains read-only"
    );
    assert_eq!(files(&hidden_authoring), authoring_bytes);
    assert_eq!(files(&hidden_source), source_bytes);
    fs::write(
        evidence.join("receipt.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "test": "installed_sampler_variants_preview_read_only_and_reopen_identity",
            "mandatory_test_count": 1,
            "assigned_guid": guid.to_string(),
            "assigned_binding": assigned,
            "installed_package_digest": lock.packages[PACKAGE].digest,
            "variant_asset_id": reopened.static_model().unwrap().source().asset_id,
            "variant_source_hash": reopened.source_hash(),
            "variant_sampler": reopened.static_model().unwrap().source().materials[0],
            "adapter": h.state().viewport3d_gpu().unwrap().gpu().adapter_name(),
            "wrap_changed_pixels": different_assets,
            "filter_changed_pixels": variant_pixels,
            "authoring_and_package_source_hidden": true,
            "real_cli_logs": ["cli-nearest.stdout.log", "cli-nearest.stderr.log", "cli-linear.stdout.log", "cli-linear.stderr.log"],
            "geometry_uv_images_color_unselected_materials_preserved": true,
            "removed_cached_asset_refused": true,
            "default_transform_and_full_binding_identity_reopened": true,
            "orbit_changed_pixels": orbit_pixels,
            "zoom_changed_pixels": zoom_pixels,
            "reset_exact": true,
            "preview_without_selection": true,
            "preview_preserves_project_source_files_frame_history_bindings_and_editor_camera": true,
            "corrupt_cached_asset_refused": true,
            "use_model_save_reopen_preserved": true,
            "normalized_viewport_size": [900, 850],
            "fresh_reopen_viewport_pixels_exact": true
        }))
        .unwrap(),
    )
    .unwrap();
}
