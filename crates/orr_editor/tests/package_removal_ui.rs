#![cfg(all(feature = "package-manager", feature = "room-animated-models"))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
use egui::accesskit::Role;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use egui_wgpu::{wgpu, WgpuSetupCreateNew};
use orr_editor::{
    app::{LBL_PLAY, LBL_STOP},
    Editor, EditorApp, HostSpec, Mode,
};
use orr_model_bindings::model_bindings::{
    self, AnimationDescriptor, Binding, Document, LocalTransform, ModelKind, PlaybackMode,
};
use orr_package::{Project, Runtime};
use orr_reflect::Scene;
use orr_sample::{
    room_game::{RoomActor, RoomConfig, RoomEscapeV1, KEY, PLAYER},
    room_project::{self, PreparedProject, PreparedScene, SEED},
};
use orr_sim::Simulation;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

fn files(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for item in fs::read_dir(dir).unwrap() {
            let path = item.unwrap().path();
            if path.is_dir() {
                visit(root, &path, out);
            } else {
                out.insert(
                    path.strip_prefix(root).unwrap().into(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    visit(root, root, &mut out);
    out
}
fn source(root: &Path, name: &str, dependencies: &[&str]) -> PathBuf {
    let path = root.join(name);
    fs::create_dir(&path).unwrap();
    fs::write(path.join("LICENSE.txt"), "Owned QA fixture license").unwrap();
    fs::write(path.join("orr.package.json"),serde_json::to_vec(&serde_json::json!({"schema":1,"name":name,"version":"1.0.0","engine":"*","capabilities":[],"dependencies":dependencies.iter().map(|name|((*name).to_owned(),"1.0.0")).collect::<BTreeMap<_,_>>(),"files":["LICENSE.txt"]})).unwrap()).unwrap();
    path
}
fn fixture(root: &Path, sources: &Path) {
    fs::create_dir(root).unwrap();
    fs::create_dir(sources).unwrap();
    let sim = Simulation::<RoomEscapeV1>::new(RoomConfig, 60, SEED);
    let text = Scene::unbake(&room_project::types(), sim.frame(), None)
        .unwrap()
        .to_yaml();
    fs::write(root.join("room.scene.yaml"), &text).unwrap();
    fs::write(
        root.join("room.camera.json"),
        serde_json::to_vec(&orr_sample::room_camera::Document {
            schema: 1,
            target: [0.0; 3],
            yaw: 0.6,
            pitch: 0.6,
            distance: 12.0,
            projection: orr_sample::room_camera::Projection::Perspective {
                fov_y_degrees: 45.0,
            },
        })
        .unwrap(),
    )
    .unwrap();
    fs::write(root.join("orr.project.json"),serde_json::to_vec(&serde_json::json!({"schema":2,"engine":"*","entry":{"game":"room-escape-v1","scene":"room.scene.yaml","models":"room.models.json","camera":"room.camera.json"}})).unwrap()).unwrap();
    let assets = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets")
        .canonicalize()
        .unwrap();
    let shared = source(sources, "shared", &[]);
    let unused = source(sources, "unused", &["shared"]);
    let keeper = source(sources, "keeper", &["shared"]);
    Project::open_for_install(root, Runtime::content_only().engine_version)
        .unwrap()
        .install(&[
            assets.join("imported_scene_demo"),
            assets.join("animation_demo"),
            shared,
            unused,
            keeper,
        ])
        .unwrap();
    let loaded =
        model_bindings::load_asset(root, "sample-imported-scene", "foreground.glb").unwrap();
    let static_binding = Binding::from_asset(
        "sample-imported-scene".into(),
        "foreground.glb".into(),
        &loaded,
        LocalTransform::default(),
    )
    .unwrap();
    let loaded = model_bindings::load_asset_for_kind(
        root,
        "sample-animation",
        "animated.glb",
        ModelKind::Animated,
    )
    .unwrap();
    let animated_binding = Binding::from_animated_asset(
        "sample-animation".into(),
        "animated.glb".into(),
        &loaded,
        AnimationDescriptor {
            clip_index: 1,
            playback: PlaybackMode::Loop,
        },
        LocalTransform::default(),
    )
    .unwrap();
    let scene = PreparedScene::parse(&text).unwrap();
    let mut bindings = BTreeMap::new();
    for entity in scene.frame().entities() {
        let kind = scene.frame().get::<RoomActor>(entity).unwrap().kind;
        if kind == PLAYER || kind == KEY {
            bindings.insert(
                scene.index().guid(entity).unwrap().to_string(),
                if kind == PLAYER {
                    static_binding.clone()
                } else {
                    animated_binding.clone()
                },
            );
        }
    }
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
    editor.install_room_camera(camera.document.clone()).unwrap();
    let mut setup = WgpuSetupCreateNew::without_display_handle();
    setup.instance_descriptor.backends = wgpu::Backends::VULKAN;
    setup.native_adapter_selector = Some(Arc::new(|adapters, _| {
        adapters
            .iter()
            .find(|a| a.get_info().backend == wgpu::Backend::Vulkan)
            .cloned()
            .ok_or_else(|| "Vulkan required".into())
    }));
    Harness::builder()
        .with_size([1600.0, 1100.0])
        .wgpu_setup(setup.into())
        .build_eframe(move |cc| {
            let mut app = EditorApp::new(editor, cc.wgpu_render_state.clone());
            app.models.install_room(models).unwrap();
            app.room_camera = Some(orr_editor::room_camera_panel::Panel::new(camera));
            app
        })
}
fn settle(h: &mut Harness<'_, EditorApp>) {
    for _ in 0..3 {
        h.state_mut().editor.sync();
        h.run_steps(2);
    }
}
fn click(h: &mut Harness<'_, EditorApp>, label: &str) {
    h.get_by_role_and_label(Role::Button, label).scroll_to_me();
    h.run_steps(20);
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
fn wait(h: &mut Harness<'_, EditorApp>) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while h.state().package_manager.busy() {
        settle(h);
        assert!(Instant::now() < deadline, "package worker timeout");
        std::thread::sleep(Duration::from_millis(10));
    }
    settle(h);
}
fn shot(h: &mut Harness<'_, EditorApp>, root: &Path, name: &str) {
    settle(h);
    let image = h.render().expect("production EditorApp composite");
    let rgba = egui::ColorImage::from_rgba_unmultiplied(
        [image.width() as usize, image.height() as usize],
        image.as_raw(),
    );
    orr_editor::app::write_png(&root.join(name), &rgba).unwrap();
}
fn preview(h: &mut Harness<'_, EditorApp>, name: &str) {
    click(h, &format!("Select removal {name}"));
    click(h, "Preview package removal");
    wait(h);
}

#[test]
#[ignore = "requires software Vulkan and fresh ORR_PACKAGE_REMOVAL_CAPTURE_DIR"]
fn verified_room_package_removal_preserves_content_and_reopens_for_play() {
    let evidence = PathBuf::from(
        std::env::var_os("ORR_PACKAGE_REMOVAL_CAPTURE_DIR").expect("fresh evidence directory"),
    );
    assert!(evidence.is_absolute());
    fs::create_dir(&evidence).unwrap();
    let root = evidence.join("room");
    let sources = evidence.join("sources");
    fixture(&root, &sources);
    let original = files(&root);
    let source_bytes = files(&sources);
    let mut h = harness(&root);
    settle(&mut h);
    let checksum = h.state().editor.checksum();
    let history = h.state().editor.history().clone();
    h.get_by_label("File").click();
    h.run_steps(2);
    h.get_by_label("Content packages…").click_accesskit();
    settle(&mut h);
    click(&mut h, "Verify installed packages");
    wait(&mut h);
    shot(&mut h, &evidence, "01-verified.png");
    for name in ["sample-imported-scene", "sample-animation"] {
        preview(&mut h, name);
        assert!(h.state().package_manager.status.contains("failed"));
        assert!(h
            .get_by_role_and_label(Role::Button, "Remove previewed package")
            .accesskit_node()
            .is_disabled());
        assert_eq!(files(&root), original);
    }
    shot(&mut h, &evidence, "02-used-refused.png");
    preview(&mut h, "unused");
    assert!(h.query_by_label("Deactivate package: unused").is_some());
    assert!(h.query_by_label("Retained dependency: shared").is_none());
    // A lock change after preview must fail without replacing that newer lock.
    let extra = source(&sources, "extra", &[]);
    let extra_plan = orr_editor::package_manager::plan_add(&root, &extra).unwrap();
    orr_editor::package_manager::apply_add(&extra_plan).unwrap();
    let newer = files(&root);
    click(&mut h, "Remove previewed package");
    wait(&mut h);
    assert!(h.state().package_manager.status.contains("failed"));
    assert_eq!(files(&root), newer);
    click(&mut h, "Verify installed packages");
    wait(&mut h);
    // Removing shared's direct selection keeps it active for keeper/unused.
    preview(&mut h, "shared");
    assert!(h
        .query_by_label("Remains active as a dependency: shared")
        .is_some());
    click(&mut h, "Remove previewed package");
    wait(&mut h);
    assert!(h
        .state()
        .package_manager
        .status
        .contains("removal complete"));
    preview(&mut h, "unused");
    assert!(h.query_by_label("Deactivate package: unused").is_some());
    assert!(h.query_by_label("Retained dependency: shared").is_some());
    shot(&mut h, &evidence, "03-removal-preview.png");
    click(&mut h, "Remove previewed package");
    wait(&mut h);
    assert!(h
        .state()
        .package_manager
        .status
        .contains("removal complete"));
    assert!(h.query_by_label("Select removal unused").is_none());
    assert!(h.query_by_label("Select removal shared").is_none());
    shot(&mut h, &evidence, "04-inventory-refreshed.png");
    let lock = orr_editor::package_manager::inspect(&root).unwrap();
    assert!(!lock.packages.contains_key("unused"));
    assert!(lock.packages.contains_key("shared"));
    assert!(!lock.direct.contains_key("shared"));
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history(), &history);
    for (path, bytes) in &original {
        if path != Path::new("orr.packages.lock.json") {
            assert_eq!(
                fs::read(root.join(path)).unwrap(),
                *bytes,
                "preserved {}",
                path.display()
            );
        }
    }
    for (path, bytes) in &source_bytes {
        assert_eq!(fs::read(sources.join(path)).unwrap(), *bytes);
    }
    drop(h);
    let mut h = harness(&root);
    settle(&mut h);
    assert_eq!(h.state().editor.checksum(), checksum);
    shot(&mut h, &evidence, "05-reopened.png");
    click(&mut h, LBL_PLAY);
    assert_eq!(h.state().editor.mode(), Mode::Play);
    shot(&mut h, &evidence, "06-playing.png");
    click(&mut h, LBL_STOP);
    assert_eq!(h.state().editor.checksum(), checksum);
    let adapter = h
        .state()
        .viewport3d_gpu()
        .expect("production Room viewport")
        .gpu()
        .adapter_name()
        .to_owned();
    assert!(
        adapter.to_lowercase().contains("llvmpipe"),
        "software Vulkan acceptance"
    );
    fs::write(evidence.join("acceptance.json"),serde_json::to_vec_pretty(&serde_json::json!({"schema":1,"production_editor_app":true,"software_gpu":true,"adapter":adapter,"physical_os_input":false,"static_animated_used_refused":true,"stale_lock_refused":true,"direct_to_retained_dependency":true,"explicit_preview_remove":true,"cache_sources_scene_models_camera_preserved":true,"checksum_history_unchanged":true,"inventory_refreshed":true,"reopen_play_stop":true})).unwrap()).unwrap();
}
