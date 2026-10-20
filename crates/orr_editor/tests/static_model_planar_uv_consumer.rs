//! Actual CLI -> installed package -> EditorApp egui widgets -> Room Play.
//! This is GPU/widget integration, NOT an OS-native mouse/keyboard acceptance.
//! Scene, camera, selection and asset text are public-API setup; Load, Assign,
//! Undo, Redo, Save, Play and Pause use the existing real egui widgets.
#![cfg(feature = "package-manager")]
#![allow(clippy::float_arithmetic, clippy::disallowed_types)]
#[path = "../../orr_render/tests/common/static_model_planar_uv_oracle.rs"]
mod oracle;
use egui_kittest::{kittest::Queryable, Harness};
use egui_wgpu::{wgpu, WgpuSetupCreateNew};
use orr_editor::{
    app::{LBL_PAUSE, LBL_PLAY},
    model_bindings::{self, Binding, Document, LocalTransform, ModelKind},
    Editor, EditorApp, HostSpec, Target,
};
use orr_package::{Project, Runtime};
use orr_reflect::{Guid, Scene};
use orr_sample::{
    room_game::{RoomActor, RoomConfig, RoomEscapeV1, PLAYER},
    room_project::{self, PreparedProject, PreparedScene, SEED},
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
    time::{Duration, Instant},
};
const PACKAGE: &str = "planar-uv-acceptance";
// The existing catalog discovers cooked static models by .json + format header.
// Keep the exact CLI output bytes under that supported extension.
const ASSET: &str = "projected.model.json";
const ID: &str = "variants/projected.planar-uv.json";

fn fingerprint(bytes: &[u8]) -> serde_json::Value {
    let sha256: String = Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    json!({"sha256": sha256, "length": bytes.len()})
}

fn write(root: &Path, name: &str, bytes: &[u8]) {
    use std::io::Write;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(root.join(name))
        .unwrap();
    file.write_all(bytes).unwrap();
    file.sync_all().unwrap();
}

// Every simulated UI step is submitted to the actual renderer. In particular,
// sim.step acceptance must not accumulate an unbounded queue of GPU work.
fn pump(h: &mut Harness<'_, EditorApp>) {
    h.state_mut().editor.sync();
    h.run_steps(1);
    let _ = h.render().expect("real egui GPU composition / queue drain");
}
fn settle(h: &mut Harness<'_, EditorApp>) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        pump(h);
        if h.state().editor.down().is_none()
            && h.state().editor.yard_rows_coherent()
            && h.state().editor.snapshot().is_some_and(|s| {
                s.timeline().is_some() == (h.state().editor.mode() == orr_editor::Mode::Play)
                    && s.tick() == s.predicted().tick()
            })
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "coherent Room host frame deadline"
        );
        std::thread::yield_now();
    }
}
fn click(h: &mut Harness<'_, EditorApp>, label: &str) {
    h.get_by_label(label).scroll_to_me();
    pump(h);
    h.get_by_label(label).click_accesskit();
    settle(h);
}
fn capture(h: &mut Harness<'_, EditorApp>, root: &Path, name: &str) -> Vec<u8> {
    settle(h);
    let ui = h.render().expect("actual EditorApp full UI composition");
    write(root, &format!("{name}-ui.rgba"), ui.as_raw());
    // Encode the retained GPU bytes directly. ColorImage/Color32 conversion
    // premultiplies translucent UI pixels and would change the evidence bytes.
    let mut encoded_ui = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut encoded_ui, ui.width(), ui.height());
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(ui.as_raw()).unwrap();
        writer.finish().unwrap();
    }
    write(root, &format!("{name}-ui.png"), &encoded_ui);
    let gpu = h
        .state()
        .viewport3d_gpu()
        .expect("actual main viewport")
        .gpu();
    let (w, height) = gpu.target().size();
    let pixels = gpu.read_rgba8();
    write(root, &format!("{name}-viewport.rgba"), &pixels);
    orr_editor::app::write_png(
        &root.join(format!("{name}-viewport.png")),
        &egui::ColorImage::from_rgba_unmultiplied([w as usize, height as usize], &pixels),
    )
    .unwrap();
    write(
        root,
        &format!("{name}-dimensions.json"),
        &serde_json::to_vec(&json!({
            "ui": [ui.width(), ui.height()], "viewport": [w, height],
            "adapter": gpu.adapter_name()
        }))
        .unwrap(),
    );
    pixels
}
fn harness(root: &Path, evidence: &Path) -> Harness<'static, EditorApp> {
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
    let evidence = evidence.to_path_buf();
    let mut setup = WgpuSetupCreateNew::without_display_handle();
    setup.instance_descriptor.backends = wgpu::Backends::VULKAN;
    setup.native_adapter_selector = Some(Arc::new(move |adapters, _| {
        let adapter = adapters
            .iter()
            .find(|adapter| {
                let info = adapter.get_info();
                info.backend == wgpu::Backend::Vulkan && info.device_type == wgpu::DeviceType::Cpu
            })
            .cloned()
            .ok_or_else(|| "mandatory software Vulkan CPU adapter unavailable".to_owned())?;
        let info = adapter.get_info();
        if !evidence.join("adapter.json").exists() {
            write(
                &evidence,
                "adapter.json",
                &serde_json::to_vec_pretty(&json!({
                    "name": info.name, "backend": format!("{:?}",info.backend),
                    "device_type": format!("{:?}",info.device_type)
                }))
                .unwrap(),
            );
        }
        Ok(adapter)
    }));
    Harness::builder()
        .with_size([1600.0, 1100.0])
        .with_step_dt(1.0 / 60.0)
        .wgpu_setup(setup.into())
        .build_eframe(move |cc| {
            let mut app = EditorApp::new(editor, cc.wgpu_render_state.clone());
            app.models.install_room(models).unwrap();
            app.room_camera = Some(orr_editor::room_camera_panel::Panel::new(camera));
            app
        })
}
fn room(root: &Path) -> Guid {
    let sim = orr_sim::Simulation::<RoomEscapeV1>::new(RoomConfig, 60, SEED);
    let text = Scene::unbake(&room_project::types(), sim.frame(), None)
        .unwrap()
        .to_yaml();
    write(root, "room.scene.yaml", text.as_bytes());
    write(
        root,
        "room.camera.json",
        &orr_sample::room_camera::Document::readable_default()
            .to_bytes()
            .unwrap(),
    );
    write(
        root,
        "orr.project.json",
        &serde_json::to_vec(&json!({"schema":2,"engine":"*",
        "entry":{"game":"room-escape-v1","scene":"room.scene.yaml",
            "models":"room.models.json","camera":"room.camera.json"}}))
        .unwrap(),
    );
    let scene = PreparedScene::parse(&text).unwrap();
    let player = scene
        .frame()
        .entities()
        .find(|e| {
            scene
                .frame()
                .get::<RoomActor>(*e)
                .is_some_and(|a| a.kind == PLAYER)
        })
        .unwrap();
    scene.index().guid(player).unwrap().clone()
}
// Match an actual authoritative streamed Frame, not reconstructed predicted data.
fn authoritative_frame(h: &mut Harness<'_, EditorApp>) -> (u64, u64, Vec<u8>) {
    settle(h);
    let snapshot = h.state().editor.snapshot().unwrap();
    let tick = snapshot.predicted().tick();
    let checksum = snapshot.predicted().checksum();
    let client = h
        .state_mut()
        .editor
        .agent_client("planar-uv-authority-probe")
        .unwrap();
    client.local_frames.clear();
    client
        .call(
            "watch.subscribe",
            json!({"topics":["frames"],"source":"view","max_fps":60}),
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        client.poll().unwrap();
        while let Some(f) = client.local_frames.pop_front() {
            if f.frame.tick() == tick && f.frame.checksum() == checksum {
                return (tick, checksum, f.frame.to_bytes());
            }
        }
        assert!(
            Instant::now() < deadline,
            "matching authoritative Frame deadline"
        );
        std::thread::yield_now();
    }
}
fn checkpoint(h: &mut Harness<'_, EditorApp>, tick: u64) -> (u64, u64, Vec<u8>) {
    // Play/Pause are real widgets. Seek/individual tick stepping is explicitly
    // public-API test transport setup, never claimed to be mouse interaction.
    click(h, LBL_PLAY);
    click(h, LBL_PAUSE);
    h.state_mut().editor.seek(0);
    settle(h);
    assert_eq!(h.state().editor.snapshot().unwrap().predicted().tick(), 0);
    for expected in 1..=tick {
        h.state_mut().editor.step(1);
        settle(h); // includes a real render drain after every sim.step
        assert_eq!(
            h.state().editor.snapshot().unwrap().predicted().tick(),
            expected
        );
    }
    authoritative_frame(h)
}

#[test]
#[ignore = "requires current-source absolute CLI, fresh evidence directory, software Vulkan CPU GPU"]
fn actual_cli_installed_room_assign_undo_redo_save_reopen_hidden_sources_and_frame_parity() {
    let cli = PathBuf::from(
        std::env::var_os("ORR_STATIC_MODEL_PLANAR_UV_BIN").expect("built current-source CLI"),
    );
    assert!(cli.is_absolute() && cli.is_file());
    let evidence = PathBuf::from(
        std::env::var_os("ORR_PLANAR_UV_EDITOR_EVIDENCE")
            .expect("fresh absolute evidence directory"),
    );
    assert!(evidence.is_absolute());
    fs::create_dir(&evidence).unwrap();
    write(&evidence, "started.json", &serde_json::to_vec_pretty(&json!({
        "status":"partial", "cli":cli, "actual_widgets":["Load model asset","Assign static model","Undo model","Redo model","Save model bindings",LBL_PLAY,LBL_PAUSE],
        "public_api_setup":["Room fixture", "camera", "selection", "asset text", "package installation", "seek and tick transport", "presentation-off baseline"],
        "scope":"real egui/GPU, not OS-native; independent texture pixel oracle is in renderer suite"
    })).unwrap());
    let authoring = evidence.join("authoring");
    let package = evidence.join("package-source");
    let root = evidence.join("room");
    for path in [&authoring, &package, &root] {
        fs::create_dir(path).unwrap();
    }
    fs::create_dir_all(authoring.join(Path::new(oracle::SOURCE_PATH).parent().unwrap())).unwrap();
    write(
        &authoring,
        oracle::SOURCE_PATH,
        &oracle::fixture().to_bytes().unwrap(),
    );
    write(
        &authoring,
        "project.planar-uv.json",
        &serde_json::to_vec(&oracle::recipe()).unwrap(),
    );
    // Exact executable and compiled-in test/creator source identities, recorded
    // before executing the external creator. No machine-local paths are needed.
    let mut provenance = json!({
        "algorithm": "sha256",
        "cli": fingerprint(&fs::read(&cli).unwrap()),
        "recipe": fingerprint(&fs::read(authoring.join("project.planar-uv.json")).unwrap()),
        "source": fingerprint(&fs::read(authoring.join(oracle::SOURCE_PATH)).unwrap()),
        "source_files": {
            "test": fingerprint(include_bytes!("static_model_planar_uv_consumer.rs")),
            "oracle": fingerprint(include_bytes!("../../orr_render/tests/common/static_model_planar_uv_oracle.rs")),
            "creator": fingerprint(include_bytes!("../../orr_model/examples/static_model_planar_uv.rs")),
            "helper": fingerprint(include_bytes!("../../orr_model/examples/static_model_planar_uv/authoring.rs"))
        }
    });
    write(
        &evidence,
        "provenance-inputs.json",
        &serde_json::to_vec_pretty(&provenance).unwrap(),
    );
    let output = Command::new(&cli)
        .arg("--recipe")
        .arg(authoring.join("project.planar-uv.json"))
        .args(["--asset-id", ID, "--output"])
        .arg(authoring.join("projected.model.json"))
        .output()
        .unwrap();
    write(&evidence, "cli.stdout", &output.stdout);
    write(&evidence, "cli.stderr", &output.stderr);
    assert!(
        output.status.success(),
        "actual CLI failed: {:?}",
        output.status
    );
    let cooked = fs::read(authoring.join("projected.model.json")).unwrap();
    let hash: String = Sha256::digest(&cooked)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    write(&package, ASSET, &cooked);
    // Explicit unrelated admission seed. This is NOT the planar source model;
    // it remains installed when the recipe/source/original output are hidden.
    write(
        &package,
        "foreground.glb",
        include_bytes!("../../../assets/imported_scene_demo/foreground.glb"),
    );
    write(
        &package,
        "orr.package.json",
        &serde_json::to_vec(&json!({"schema":1,
        "name":PACKAGE,"version":"1.0.0","engine":"*","capabilities":["models"],
        "dependencies":{},"files":[ASSET,"foreground.glb"]}))
        .unwrap(),
    );
    let guid = room(&root);
    Project::open_for_install(&root, Runtime::content_only().engine_version)
        .unwrap()
        .install(std::slice::from_ref(&package))
        .unwrap();
    let starter = model_bindings::load_asset(&root, PACKAGE, "foreground.glb").unwrap();
    let original_document = Document {
        version: 2,
        scene: "room.scene.yaml".into(),
        project: ".".into(),
        bindings: [(
            guid.to_string(),
            Binding::from_asset(
                PACKAGE.into(),
                "foreground.glb".into(),
                &starter,
                LocalTransform::default(),
            )
            .unwrap(),
        )]
        .into_iter()
        .collect(),
    };
    write(
        &root,
        "room.models.json",
        &serde_json::to_vec(&original_document).unwrap(),
    );
    // Room inspection admits its entry sidecar, so create the real unrelated
    // starter binding first. Inspection remains strict; no admission is skipped.
    let lock = orr_editor::package_manager::inspect(&root).unwrap();
    let installed = root
        .join(".orr/packages/objects")
        .join(&lock.packages[PACKAGE].digest)
        .join(ASSET);
    provenance["output"] = fingerprint(&cooked);
    provenance["installed"] = fingerprint(&fs::read(installed).unwrap());
    write(
        &evidence,
        "provenance.json",
        &serde_json::to_vec_pretty(&provenance).unwrap(),
    );
    assert_eq!(
        provenance["output"], provenance["installed"],
        "install preserves exact CLI output bytes"
    );
    let initial_scene = fs::read(root.join("room.scene.yaml")).unwrap();
    let initial_bindings = fs::read(root.join("room.models.json")).unwrap();
    let mut h = harness(&root, &evidence);
    settle(&mut h);
    h.state_mut()
        .editor
        .select(Some(Target::Guid(guid.clone())));
    settle(&mut h);
    click(&mut h, "Static model binding");
    h.state_mut().models.package = PACKAGE.into();
    h.state_mut().models.asset = ASSET.into();
    click(&mut h, "Load model asset");
    click(&mut h, "Assign static model");
    capture(&mut h, &evidence, "01-assigned");
    assert!(
        h.state().models.error().is_none(),
        "{:?}",
        h.state().models.error()
    );
    let assigned_document = h
        .state()
        .models
        .bindings
        .as_ref()
        .unwrap()
        .document()
        .clone();
    let assigned = assigned_document.bindings[&guid.to_string()].clone();
    assert_eq!(assigned.package, PACKAGE);
    assert_eq!(assigned.asset, ASSET);
    assert_eq!(assigned.kind, ModelKind::Static);
    assert_eq!(assigned.source_hash, hash);
    assert_eq!(assigned.transform, LocalTransform::default());
    assert_eq!(assigned_document.bindings.len(), 1);
    assert!(h.state().models.bindings.as_ref().unwrap().dirty());
    assert_eq!(
        fs::read(root.join("room.models.json")).unwrap(),
        initial_bindings
    );
    click(&mut h, "Undo model");
    capture(&mut h, &evidence, "02-undo");
    assert_eq!(
        h.state().models.bindings.as_ref().unwrap().document(),
        &original_document
    );
    click(&mut h, "Redo model");
    capture(&mut h, &evidence, "03-redo");
    assert_eq!(
        h.state().models.bindings.as_ref().unwrap().document(),
        &assigned_document
    );
    click(&mut h, "Save model bindings");
    capture(&mut h, &evidence, "04-saved");
    assert!(!h.state().models.bindings.as_ref().unwrap().dirty());
    assert_eq!(
        fs::read(root.join("room.scene.yaml")).unwrap(),
        initial_scene
    );
    let saved = fs::read(root.join("room.models.json")).unwrap();
    drop(h);

    fs::rename(&authoring, evidence.join("authoring-hidden")).unwrap();
    fs::rename(&package, evidence.join("package-source-hidden")).unwrap();
    assert!(!authoring.exists() && !package.exists());
    let mut h = harness(&root, &evidence); // genuinely fresh project and app admission
    settle(&mut h);
    capture(&mut h, &evidence, "05-fresh-reopen");
    assert_eq!(
        h.state().models.bindings.as_ref().unwrap().document(),
        &assigned_document
    );
    let loaded = model_bindings::load_binding(&root, &assigned).unwrap();
    assert_eq!(loaded.source_hash(), hash);
    assert_eq!(loaded.static_model().unwrap().to_bytes().unwrap(), cooked);
    assert_eq!(loaded.static_model().unwrap().source().asset_id, ID);
    assert_eq!(
        loaded.static_model().unwrap().source().dependencies.len(),
        2
    );
    assert!(model_bindings::list_assets(&root)
        .unwrap()
        .iter()
        .any(|a| a.package == PACKAGE && a.asset == ASSET && a.kind == ModelKind::Static));
    // Baseline uses the same scene and session admission with only presentation
    // bindings detached. No alternative simulation implementation is used.
    let presentation = std::mem::take(&mut h.state_mut().models);
    let off = checkpoint(&mut h, 12);
    let off_pixels = capture(&mut h, &evidence, "06-source-hidden-play-off");
    write(&evidence, "presentation-off.frame", &off.2);
    drop(h);
    let mut h = harness(&root, &evidence);
    settle(&mut h);
    let on = checkpoint(&mut h, 12);
    let on_pixels = capture(&mut h, &evidence, "07-source-hidden-play-on");
    write(&evidence, "presentation-on.frame", &on.2);
    write(
        &evidence,
        "parity-observed.json",
        &serde_json::to_vec_pretty(&json!({
            "off_tick":off.0,"on_tick":on.0,"off_checksum":off.1,"on_checksum":on.1,
            "off_frame_bytes":off.2.len(),"on_frame_bytes":on.2.len(),
            "off_viewport_bytes":off_pixels.len(),"on_viewport_bytes":on_pixels.len()
        }))
        .unwrap(),
    );
    // All GPU/UI and authoritative bytes above survive any following assertion.
    assert_eq!(on.0, 12);
    assert_eq!(
        on, off,
        "same-tick presentation-neutral authoritative Frame bytes/checksum"
    );
    assert!(!h.state().models.placements(&h.state().editor).is_empty());
    assert_eq!(
        on_pixels.len(),
        off_pixels.len(),
        "same actual viewport dimensions"
    );
    let changed = on_pixels
        .as_chunks::<4>()
        .0
        .iter()
        .zip(off_pixels.as_chunks::<4>().0.iter())
        .filter(|(a, b)| a != b)
        .count();
    assert!(
        changed > 20,
        "installed presentation must visibly affect actual Room viewport: {changed}"
    );
    assert_eq!(fs::read(root.join("room.models.json")).unwrap(), saved);
    assert_eq!(
        fs::read(root.join("room.scene.yaml")).unwrap(),
        initial_scene
    );
    drop(presentation);
    write(&evidence, "acceptance.json", &serde_json::to_vec_pretty(&json!({
        "status":"passed","actual_cli":true,"actual_egui_widgets_and_gpu":true,"os_native_input":false,
        "installed_asset":ASSET,"recipe_identity":ID,"source_hash":hash,"assigned_guid":guid.to_string(),
        "undo_redo_save_fresh_reopen":true,"authoring_and_package_source_hidden":true,
        "hidden_inputs":["recipe","source cooked model","original CLI output","package source"],
        "retained_installed_files":[ASSET,"foreground.glb"],
        "unrelated_admission_seed":"foreground.glb; not planar fixture source",
        "authoritative_frame_byte_parity":true,"tick":on.0,"checksum":on.1,
        "actual_viewport_changed_pixels":changed,
        "pixel_oracle_scope":"independent UV/texture math is tested by renderer suite; this test proves real Room/editor consumer integration"
    })).unwrap());
}
