//! Mandatory real EditorApp cone KEY workflow, with production renderer defaults.
//! Current-source CLI, installed-only reload, exact fresh reopen pixels, actual
//! keyboard collect/Restart, and presentation-free authoritative replay parity.
#![cfg(feature = "package-manager")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
use egui::{accesskit::Role, Event, Modifiers};
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use egui_wgpu::{wgpu, WgpuSetupCreateNew};
use orr_bridge::PlayerSlot;
use orr_editor::{editor::input::Phase, Editor, EditorApp, HostSpec, Mode};
use orr_model_bindings::model_bindings::{self, Binding, Document, LocalTransform};
use orr_sample::{
    room_game::{RoomActor, RoomConfig, RoomEscapeV1, RoomRun, INTERACT, KEY, PLAYER},
    room_project::{self, PreparedProject, PreparedScene, SEED},
};
use orr_sim::TickInputs;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
fn hash(bytes: &[u8]) -> String {
    format!(
        "{:016x}",
        bytes
            .iter()
            .fold(0xcbf29ce484222325_u64, |h, b| (h ^ u64::from(*b))
                .wrapping_mul(0x100000001b3))
    )
}
// Independent f64 trigonometry, not producer tables, indices, bounds or cameras.
// The oblique view has a distinct apex and base, so inverted cones, cylinders,
// missing caps and wrong height/radius cannot pass by rotational symmetry.
fn oracle_triangles(radius: f64, height: f64, segments: usize) -> Vec<[[f64; 3]; 3]> {
    let ring = |i: usize| {
        let angle = std::f64::consts::TAU * (i % segments) as f64 / segments as f64;
        [radius * angle.cos(), -height / 2.0, radius * angle.sin()]
    };
    let mut result = Vec::new();
    for i in 0..segments {
        result.push([ring(i), [0.0, height / 2.0, 0.0], ring(i + 1)]);
    }
    for i in 0..segments {
        result.push([[0.0, -height / 2.0, 0.0], ring(i), ring(i + 1)]);
    }
    result
}

fn triangle_oracle(model: &orr_model::StaticModel, radius: f64, height: f64, segments: usize) {
    let expected = oracle_triangles(radius, height, segments);
    let primitives = &model.source().primitives;
    assert_eq!(primitives.len(), 2);
    let mut triangle = 0;
    for (slot, primitive) in primitives.iter().enumerate() {
        assert_eq!(primitive.transform, orr_model::IDENTITY);
        assert_eq!(primitive.indices.len(), segments * 3);
        assert_eq!(primitive.material, slot as u32);
        assert_eq!(
            primitive.vertices.len(),
            if slot == 0 {
                segments * 3
            } else {
                segments + 1
            }
        );
        for indices in primitive.indices.chunks_exact(3) {
            let want = expected[triangle];
            let a: [f64; 3] = std::array::from_fn(|axis| want[1][axis] - want[0][axis]);
            let b: [f64; 3] = std::array::from_fn(|axis| want[2][axis] - want[0][axis]);
            let normal = [
                a[1] * b[2] - a[2] * b[1],
                a[2] * b[0] - a[0] * b[2],
                a[0] * b[1] - a[1] * b[0],
            ];
            let length = normal.iter().map(|v| v * v).sum::<f64>().sqrt();
            assert!(length > 0.0);
            for (corner, index) in indices.iter().enumerate() {
                let vertex = &primitive.vertices[*index as usize];
                for (axis, normal_component) in normal.iter().enumerate() {
                    assert!(
                        (f64::from(vertex.position[axis]) - want[corner][axis]).abs() < 0.000002,
                        "independent position triangle {triangle} corner {corner} axis {axis}"
                    );
                    assert!(
                        (f64::from(vertex.normal[axis]) - normal_component / length).abs()
                            < 0.00001,
                        "faceted outward normal triangle {triangle} corner {corner} axis {axis}"
                    );
                }
            }
            triangle += 1;
        }
    }
    assert_eq!(triangle, segments * 2);
}

fn silhouette_oracle(rgba: &[u8], radius: f64, height: f64, segments: usize) -> (usize, usize) {
    // Public preview contract: fixed 448x336 canvas at one pixel per point,
    // default oblique orthographic view. No production projection helper used.
    const WIDTH: usize = 448;
    const HEIGHT: usize = 336;
    assert_eq!(rgba.len(), WIDTH * HEIGHT * 4);
    let (sy, cy) = 0.45_f64.sin_cos();
    let (sp, cp) = 0.25_f64.sin_cos();
    let half_height = (2.0 * radius * radius + height * height / 4.0).sqrt() * 1.2;
    let scale = HEIGHT as f64 / (2.0 * half_height);
    let project = |p: [f64; 3]| {
        [
            WIDTH as f64 / 2.0 + (cy * p[0] - sy * p[2]) * scale,
            HEIGHT as f64 / 2.0 - (-sy * sp * p[0] + cp * p[1] - cy * sp * p[2]) * scale,
        ]
    };
    let triangles: Vec<_> = oracle_triangles(radius, height, segments)
        .into_iter()
        .map(|tri| tri.map(project))
        .collect();
    let edge = |a: [f64; 2], b: [f64; 2], p: [f64; 2]| {
        (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
    };
    let mut expected = vec![false; WIDTH * HEIGHT];
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let p = [x as f64 + 0.5, y as f64 + 0.5];
            expected[y * WIDTH + x] = triangles.iter().any(|tri| {
                let edges = [
                    edge(tri[0], tri[1], p),
                    edge(tri[1], tri[2], p),
                    edge(tri[2], tri[0], p),
                ];
                edges.iter().all(|v| *v >= 0.0) || edges.iter().all(|v| *v <= 0.0)
            });
        }
    }
    // Ignore only a fixed three-pixel silhouette boundary for raster sampling
    // and GUI texture filtering. Interior and exterior remain independently
    // asserted. Dither cannot exceed this background classification threshold.
    let background = &rgba[..3];
    let mut checked = 0;
    let mut mismatches = 0;
    let mut interior = 0;
    for y in 3..HEIGHT - 3 {
        for x in 3..WIDTH - 3 {
            let want = expected[y * WIDTH + x];
            if !(y - 3..=y + 3)
                .all(|yy| (x - 3..=x + 3).all(|xx| expected[yy * WIDTH + xx] == want))
            {
                continue;
            }
            let pixel = &rgba[(y * WIDTH + x) * 4..][..3];
            let foreground = pixel
                .iter()
                .zip(background)
                .any(|(a, b)| a.abs_diff(*b) > 12);
            checked += 1;
            interior += usize::from(want);
            mismatches += usize::from(foreground != want);
        }
    }
    assert!(interior > 4000, "non-vacuous cone foreground coverage");
    (checked, mismatches)
}

fn fixture(root: &Path, source: &Path, authoring: &Path, evidence: &Path) {
    for dir in [root, source, authoring] {
        fs::create_dir(dir).unwrap();
    }
    let sim = orr_sim::Simulation::<RoomEscapeV1>::new(RoomConfig, 60, SEED);
    let text = orr_reflect::Scene::unbake(&room_project::types(), sim.frame(), None)
        .unwrap()
        .to_yaml();
    fs::write(root.join("room.scene.yaml"), &text).unwrap();
    let mut camera = orr_sample::room_camera::Document::readable_default();
    camera.target = [1.0, 0.5, 0.0];
    camera.projection = orr_sample::room_camera::Projection::Orthographic { half_height: 4.0 };
    fs::write(root.join("room.camera.json"), camera.to_bytes().unwrap()).unwrap();
    fs::write(root.join("orr.project.json"),serde_json::to_vec(&serde_json::json!({"schema":2,"engine":"*","entry":{"game":"room-escape-v1","scene":"room.scene.yaml","models":"room.models.json","camera":"room.camera.json"}})).unwrap()).unwrap();
    let original = orr_model::import::import_with_resolver(
        "props/original.glb",
        include_bytes!("../../orr_model/tests/fixtures/model.glb"),
        |_| panic!("embedded source"),
    )
    .unwrap();
    fs::write(
        source.join("original.orrmodel"),
        original.to_bytes().unwrap(),
    )
    .unwrap();
    let binary = PathBuf::from(
        std::env::var_os("ORR_STATIC_MODEL_CONE_BIN").expect("current-source CLI required"),
    );
    assert!(binary.is_absolute() && binary.is_file());
    for (name, radius, height, segments) in [("tall", 0.55, 2.0, 32), ("wide", 1.0, 0.65, 16)] {
        let recipe = authoring.join(format!("{name}.json"));
        fs::write(&recipe,serde_json::to_vec_pretty(&serde_json::json!({"format":"orr_static_model_cone","version":1,"radius":radius,"height":height,"segments":segments})).unwrap()).unwrap();
        let id = format!("props/{name}.cone.json");
        let output = authoring.join(format!("{name}.orrmodel"));
        let result = std::process::Command::new(&binary)
            .arg("--recipe")
            .arg(&recipe)
            .arg("--asset-id")
            .arg(&id)
            .arg("--output")
            .arg(&output)
            .output()
            .unwrap();
        fs::write(
            evidence.join(format!("cli-{name}.stdout.log")),
            &result.stdout,
        )
        .unwrap();
        fs::write(
            evidence.join(format!("cli-{name}.stderr.log")),
            &result.stderr,
        )
        .unwrap();
        assert!(
            result.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let bytes = fs::read(output).unwrap();
        let model = orr_model::StaticModel::from_bytes(&bytes).unwrap();
        assert_eq!(model.source().asset_id, id);
        assert_eq!(model.source().primitives.len(), 2);
        triangle_oracle(&model, radius, height, segments);
        fs::write(source.join(format!("{name}.orrmodel")), &bytes).unwrap();
        assert_eq!(
            fs::read(source.join(format!("{name}.orrmodel"))).unwrap(),
            bytes
        );
    }
    let cylinder_binary = PathBuf::from(
        std::env::var_os("ORR_STATIC_MODEL_CYLINDER_BIN")
            .expect("current-source cylinder CLI required for negative control"),
    );
    assert!(cylinder_binary.is_absolute() && cylinder_binary.is_file());
    let recipe = authoring.join("cylinder-control.json");
    fs::write(
        &recipe,
        serde_json::to_vec_pretty(&serde_json::json!({
            "format":"orr_static_model_cylinder", "version":1,
            "radius":0.55, "height":2.0, "segments":32
        }))
        .unwrap(),
    )
    .unwrap();
    let output = authoring.join("cylinder-control.model.json");
    let result = std::process::Command::new(&cylinder_binary)
        .arg("--recipe")
        .arg(&recipe)
        .arg("--asset-id")
        .arg("props/cylinder-control.cylinder.json")
        .arg("--output")
        .arg(&output)
        .output()
        .unwrap();
    fs::write(
        evidence.join("cli-cylinder-control.stdout.log"),
        &result.stdout,
    )
    .unwrap();
    fs::write(
        evidence.join("cli-cylinder-control.stderr.log"),
        &result.stderr,
    )
    .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let bytes = fs::read(&output).unwrap();
    let cylinder = orr_model::StaticModel::from_bytes(&bytes).unwrap();
    assert_eq!(cylinder.source().primitives.len(), 3);
    fs::write(source.join("cylinder-control.orrmodel"), bytes).unwrap();
    let manifest = orr_package::Manifest {
        schema: 1,
        name: PACKAGE.into(),
        version: "1.0.0".into(),
        engine: "*".into(),
        capabilities: ["models".into()].into_iter().collect(),
        dependencies: BTreeMap::new(),
        files: [
            "original.orrmodel",
            "tall.orrmodel",
            "wide.orrmodel",
            "cylinder-control.orrmodel",
        ]
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
    let loaded = model_bindings::load_asset(root, PACKAGE, "original.orrmodel").unwrap();
    let binding = Binding::from_asset(
        PACKAGE.into(),
        "original.orrmodel".into(),
        &loaded,
        LocalTransform::default(),
    )
    .unwrap();
    let scene = PreparedScene::parse(&text).unwrap();
    let bindings = scene
        .frame()
        .entities()
        .filter(|e| {
            scene
                .frame()
                .get::<RoomActor>(*e)
                .is_some_and(|a| [PLAYER, KEY].contains(&a.kind))
        })
        .map(|e| (scene.index().guid(e).unwrap().to_string(), binding.clone()))
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
        .with_pixels_per_point(1.0)
        .renderer(egui_kittest::wgpu::WgpuTestRenderer::from_render_state(
            egui_kittest::wgpu::create_render_state(
                setup.into(),
                egui_wgpu::RendererOptions::default(),
            ),
        ))
        .build_eframe(move |cc| {
            let mut app = EditorApp::new(editor, cc.wgpu_render_state.clone());
            app.models.install_room(models).unwrap();
            app.room_camera = Some(orr_editor::room_camera_panel::Panel::new(camera));
            app
        })
}

const CANVAS: &str = "Static package model preview canvas";
const PACKAGE: &str = "static-cone-preview";

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
    fs::write(evidence.join(name).with_extension("rgba"), &rgba).unwrap();
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

fn run(app: &EditorApp) -> RoomRun {
    *app.editor
        .snapshot()
        .unwrap()
        .predicted()
        .singleton::<RoomRun>()
}
fn key(h: &mut Harness<'_, EditorApp>, pressed: bool) {
    h.event(Event::Key {
        key: egui::Key::E,
        physical_key: Some(egui::Key::E),
        pressed,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
}
fn frame_bytes(editor: &mut Editor) -> Vec<u8> {
    editor.sync();
    let snapshot = editor.snapshot().unwrap();
    let tick = snapshot.predicted().tick();
    let checksum = snapshot.predicted().checksum();
    let client = editor.agent_client("cone-room-frame-probe").unwrap();
    client.local_frames.clear();
    client
        .call(
            "watch.subscribe",
            serde_json::json!({"topics":["frames"],"source":"view","max_fps":60}),
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        client.poll().unwrap();
        while let Some(frame) = client.local_frames.pop_front() {
            if frame.frame.tick() == tick && frame.frame.checksum() == checksum {
                return frame.frame.to_bytes();
            }
        }
        assert!(
            Instant::now() < deadline,
            "matching authoritative frame bytes missing"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}
#[test]
#[ignore = "requires current-source CLI, software Vulkan and fresh absolute capture directory"]
fn installed_cone_key_preview_undo_reopen_collect_restart() {
    assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
    let evidence = PathBuf::from(
        std::env::var_os("ORR_STATIC_MODEL_CONE_PREVIEW_CAPTURE_DIR")
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
    let authored_frame = frame_bytes(&mut h.state_mut().editor);
    assert!(h.state().editor.selection().is_none());
    h.get_by_label("File").click();
    h.run_steps(2);
    h.get_by_label("Content packages…").click_accesskit();
    settle(&mut h);
    assert!(h
        .query_by_label(&format!("Preview static model {PACKAGE}/tall.orrmodel"))
        .is_none());
    verify(&mut h);
    shot(&mut h, &evidence, "01-verified.png");
    let hidden_source = evidence.join("hidden-package-source");
    let hidden_authoring = evidence.join("hidden-authoring");
    fs::rename(&source, &hidden_source).unwrap();
    fs::rename(&authoring, &hidden_authoring).unwrap();
    assert!(!source.exists() && !authoring.exists());
    let tall = model_bindings::load_asset(&root, PACKAGE, "tall.orrmodel").unwrap();
    let wide = model_bindings::load_asset(&root, PACKAGE, "wide.orrmodel").unwrap();
    triangle_oracle(tall.static_model().unwrap(), 0.55, 2.0, 32);
    triangle_oracle(wide.static_model().unwrap(), 1.0, 0.65, 16);
    assert_ne!(tall.source_hash(), wide.source_hash());
    assert_ne!(
        tall.static_model().unwrap().source().asset_id,
        wide.static_model().unwrap().source().asset_id
    );
    preview(&mut h, "original.orrmodel");
    let original_preview = crop(&mut h, &evidence, "02-original-canvas.png");
    preview(&mut h, "tall.orrmodel");
    let tall_preview = crop(&mut h, &evidence, "03-tall-canvas.png");
    shot(&mut h, &evidence, "03-tall.png");
    let tall_oracle = silhouette_oracle(&tall_preview, 0.55, 2.0, 32);
    assert!(
        tall_oracle.0 > 4000 && tall_oracle.1 == 0,
        "independent tall cone silhouette mismatch: {tall_oracle:?}"
    );
    preview(&mut h, "cylinder-control.orrmodel");
    let cylinder_preview = crop(&mut h, &evidence, "03-cylinder-negative-control.png");
    let cylinder_oracle = silhouette_oracle(&cylinder_preview, 0.55, 2.0, 32);
    assert!(
        cylinder_oracle.1 > 1000,
        "independent cone oracle must reject cylinder substitution: {cylinder_oracle:?}"
    );
    preview(&mut h, "wide.orrmodel");
    let wide_preview = crop(&mut h, &evidence, "04-wide-canvas.png");
    let wide_oracle = silhouette_oracle(&wide_preview, 1.0, 0.65, 16);
    assert!(
        wide_oracle.0 > 4000 && wide_oracle.1 == 0,
        "independent wide cone silhouette mismatch: {wide_oracle:?}"
    );
    let original_changed = differences(&original_preview, &tall_preview);
    let proportions_changed = differences(&tall_preview, &wide_preview);
    assert!(original_changed > 1000 && proportions_changed > 1000);
    preview(&mut h, "tall.orrmodel");
    orbit(&mut h);
    let orbited = crop(&mut h, &evidence, "05-orbit-canvas.png");
    let orbit_changed = differences(&tall_preview, &orbited);
    assert!(orbit_changed > 1000);
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
    let zoomed = crop(&mut h, &evidence, "06-zoom-canvas.png");
    let zoom_changed = differences(&orbited, &zoomed);
    assert!(zoom_changed > 1000);
    click(&mut h, "Reset preview view");
    h.run_steps(10);
    let reset = crop(&mut h, &evidence, "07-reset-canvas.png");
    assert_eq!(reset, tall_preview);
    click(&mut h, "Close static preview");
    assert!(h.query_by_label(CANVAS).is_none());
    assert!(h.state().editor.selection().is_none());
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history(), &history);
    assert_eq!(h.state().editor.camera3d, camera);
    assert_eq!(
        h.state().models.bindings.as_ref().unwrap().document(),
        &bindings
    );
    assert!(!h.state().models.bindings.as_ref().unwrap().dirty());
    assert_eq!(frame_bytes(&mut h.state_mut().editor), authored_frame);
    assert_eq!(files(&root), original);
    let prepared = PreparedProject::open(&root).unwrap();
    let scene = prepared.scene();
    let entity = scene
        .frame()
        .entities()
        .find(|e| {
            scene
                .frame()
                .get::<RoomActor>(*e)
                .is_some_and(|a| a.kind == KEY)
        })
        .unwrap();
    let guid = scene.index().guid(entity).unwrap().clone();
    h.state_mut()
        .editor
        .select(Some(orr_editor::Target::Guid(guid.clone())));
    settle(&mut h);
    click(&mut h, &format!("Use model {PACKAGE}/tall.orrmodel"));
    let assigned = h
        .state()
        .models
        .bindings
        .as_ref()
        .unwrap()
        .document()
        .bindings[&guid.to_string()]
        .clone();
    let mut expected = bindings.clone();
    expected.bindings.insert(guid.to_string(), assigned.clone());
    assert_eq!(
        h.state().models.bindings.as_ref().unwrap().document(),
        &expected
    );
    assert_eq!(assigned.transform, LocalTransform::default());
    assert_eq!(assigned.asset, "tall.orrmodel");
    assert_eq!(assigned.package, PACKAGE);
    assert_eq!(assigned.source_hash, tall.source_hash());
    assigned.validate(&tall).unwrap();
    h.state_mut().package_manager.open = false;
    settle(&mut h);
    if h.query_by_label("Save model bindings").is_none() {
        h.get_by_label("Static model binding").click_accesskit();
        settle(&mut h);
    }
    click(&mut h, "Undo model");
    assert_eq!(
        h.state().models.bindings.as_ref().unwrap().document(),
        &bindings
    );
    assert_eq!(files(&root), original);
    click(&mut h, "Redo model");
    assert_eq!(
        h.state().models.bindings.as_ref().unwrap().document(),
        &expected
    );
    assert_eq!(files(&root), original);
    click(&mut h, "Save model bindings");
    assert!(!h.state().models.bindings.as_ref().unwrap().dirty());
    shot(&mut h, &evidence, "08-assigned-undo-redo-saved.png");
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history(), &history);
    assert_eq!(frame_bytes(&mut h.state_mut().editor), authored_frame);
    h.state_mut().editor.select(None);
    settle(&mut h);
    let saved_pixels = viewport_pixels(&mut h, &evidence, "09-saved-viewport.png");
    let mut expected_files = original.clone();
    expected_files.insert(
        PathBuf::from("room.models.json"),
        fs::read(root.join("room.models.json")).unwrap(),
    );
    assert_eq!(files(&root), expected_files);
    drop(h);
    let mut h = harness(&root);
    settle(&mut h);
    assert_eq!(
        h.state().models.bindings.as_ref().unwrap().document(),
        &expected
    );
    assert!(!h.state().models.bindings.as_ref().unwrap().dirty());
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history(), &history);
    let reopened = model_bindings::load_binding(&root, &assigned).unwrap();
    assert_eq!(
        reopened.static_model().unwrap().source(),
        tall.static_model().unwrap().source()
    );
    assert_eq!(reopened.source_hash(), tall.source_hash());
    let reopened_pixels = viewport_pixels(&mut h, &evidence, "10-reopened-viewport.png");
    assert_eq!(
        saved_pixels, reopened_pixels,
        "destroyed host/app/caches must reopen exact viewport"
    );
    shot(&mut h, &evidence, "10-reopened.png");
    assert_eq!(files(&root), expected_files);
    assert_eq!(h.state().models.placements(&h.state().editor).len(), 2);
    // Actual app controls and physical E event. The canonical KEY is within the
    // simulation's interaction distance; presentation size never affects it.
    click(&mut h, orr_editor::app::LBL_PLAY);
    wait(&mut h, |h| h.state().editor.can_take_control());
    click(&mut h, "Take control");
    wait(&mut h, |h| h.state().editor.input_phase() == Phase::Active);
    let before_run = run(h.state());
    assert_eq!(before_run.key_collected, 0);
    key(&mut h, true);
    wait(&mut h, |h| run(h.state()).key_collected == 1);
    key(&mut h, false);
    wait(&mut h, |h| run(h.state()).previous_buttons == 0);
    h.state_mut().editor.pause();
    settle(&mut h);
    let live = frame_bytes(&mut h.state_mut().editor);
    let live_checksum = h.state().editor.snapshot().unwrap().predicted().checksum();
    let collected_run = run(h.state());
    assert_eq!(collected_run.key_collected, 1);
    let key_entity = h
        .state()
        .editor
        .rows()
        .iter()
        .find(|r| r.guid.as_ref() == Some(&guid))
        .unwrap()
        .entity;
    let placements = h.state().models.placements(&h.state().editor);
    assert_eq!(placements.len(), 1);
    assert!(placements.iter().all(|p| p.entity != key_entity));
    assert_eq!(
        h.state().models.bindings.as_ref().unwrap().document(),
        &expected
    );
    let collected_pixels = viewport_pixels(&mut h, &evidence, "11-collected-viewport.png");
    let collect_changed = differences(&reopened_pixels, &collected_pixels);
    assert!(
        collect_changed > 1000,
        "actual KEY cone must disappear: {collect_changed}"
    );
    shot(&mut h, &evidence, "11-collected.png");
    let stopped_tick = h.state().editor.timeline().unwrap().tick;
    let sequence = h.state().editor.snapshot().unwrap().seq();
    click(&mut h, "Restart room");
    wait(&mut h, |h| {
        h.state().editor.mode() == Mode::Play
            && h.state().editor.can_take_control()
            && h.state().editor.yard_rows_coherent()
            && h.state().editor.snapshot().is_some_and(|s| {
                s.seq() > sequence
                    && s.timeline().is_some_and(|t| t.playing)
                    && *s.predicted().singleton::<RoomRun>() == RoomRun::default()
            })
    });
    let stopped = h.state().editor.last_stopped().unwrap().clone();
    assert_eq!(stopped.tick, stopped_tick);
    assert_eq!(stopped.checksum, live_checksum);
    h.state_mut().editor.pause();
    settle(&mut h);
    h.state_mut().editor.seek(0);
    settle(&mut h);
    assert_eq!(h.state().editor.timeline().unwrap().tick, 0);
    assert_eq!(run(h.state()), RoomRun::default());
    assert_eq!(h.state().models.placements(&h.state().editor).len(), 2);
    assert_eq!(frame_bytes(&mut h.state_mut().editor), authored_frame);
    let restarted_pixels = viewport_pixels(&mut h, &evidence, "12-restarted-viewport.png");
    assert_eq!(
        restarted_pixels, reopened_pixels,
        "Restart restores exact cone and initial gameplay view"
    );
    shot(&mut h, &evidence, "12-restarted.png");
    // Replay the exact recorded GUI-admitted samples, never guessed durations,
    // in an independent simulation that has never loaded any model binding.
    let reader = orr_session::ReplayReader::<RoomEscapeV1>::parse(&stopped.replay).unwrap();
    let mut baseline = prepared.scene().simulation().unwrap();
    let mut saw_interact = false;
    let mut saw_release = false;
    let mut checked = 0;
    if let Some((_, sum)) = reader.checksums.iter().find(|(at, _)| *at == 0) {
        assert_eq!(
            baseline.frame().checksum(),
            *sum,
            "presentation-free initial checksum"
        );
        checked += 1;
    }
    for tick in 1..=reader.last_tick() {
        let (inputs, commands) = reader.tick(tick).unwrap();
        assert!(commands.is_empty());
        saw_interact |= inputs[0].buttons & INTERACT != 0;
        saw_release |= saw_interact && inputs[0].buttons == 0;
        let mut samples = TickInputs::new(tick, 1);
        samples.set_input(PlayerSlot(0), inputs[0]);
        baseline.step(&samples);
        if let Some((_, sum)) = reader.checksums.iter().find(|(at, _)| *at == tick) {
            assert_eq!(
                baseline.frame().checksum(),
                *sum,
                "presentation-free checksum at {tick}"
            );
            checked += 1;
        }
    }
    assert!(saw_interact && saw_release);
    assert!(checked > 0, "recorded checksum coverage must be nonempty");
    assert_eq!(
        checked,
        reader.checksums.len(),
        "every recorded checksum must be checked"
    );
    assert_eq!(baseline.frame().to_bytes(), live);
    assert_eq!(baseline.frame().checksum(), live_checksum);
    assert_eq!(*baseline.frame().singleton::<RoomRun>(), collected_run);
    fs::write(evidence.join("collected-authoritative.frame"), &live).unwrap();
    fs::write(
        evidence.join("presentation-free.frame"),
        baseline.frame().to_bytes(),
    )
    .unwrap();
    fs::write(evidence.join("actual-gui.orrp"), &stopped.replay).unwrap();
    click(&mut h, orr_editor::app::LBL_STOP);
    assert_eq!(h.state().editor.mode(), Mode::Edit);
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history(), &history);
    assert_eq!(frame_bytes(&mut h.state_mut().editor), authored_frame);
    assert_eq!(files(&root), expected_files);
    assert_eq!(files(&hidden_source), source_bytes);
    assert_eq!(files(&hidden_authoring), authoring_bytes);
    let lock = orr_editor::package_manager::inspect(&root).unwrap();
    let mut receipt = serde_json::json!({"schema":1,"status":"passed","producer_fnv1a64":hash(&fs::read(PathBuf::from(std::env::var_os("ORR_STATIC_MODEL_CONE_BIN").unwrap())).unwrap()),"cylinder_control_producer_fnv1a64":hash(&fs::read(PathBuf::from(std::env::var_os("ORR_STATIC_MODEL_CYLINDER_BIN").unwrap())).unwrap()),"test_source_fnv1a64":hash(include_bytes!("static_model_cone_package_preview_ui.rs")),"test":"installed_cone_key_preview_undo_reopen_collect_restart","mandatory_test_count":1,"cone_triangle_oracle":true,"tall_silhouette_pixels_checked":tall_oracle.0,"wide_silhouette_pixels_checked":wide_oracle.0,"cylinder_negative_control_mismatches":cylinder_oracle.1,"native_os_input":false,"native_gpu":false,"assigned_guid":guid.to_string(),"assigned_role":"KEY","assigned_binding":assigned,"installed_package_digest":lock.packages[PACKAGE].digest,"variant_asset_id":reopened.static_model().unwrap().source().asset_id,"variant_source_hash":reopened.source_hash(),"adapter":h.state().viewport3d_gpu().unwrap().gpu().adapter_name(),"renderer_options":"default","predictable_texture_filtering":false,"dithering":true});
    let lifecycle = serde_json::json!({"original_changed_pixels":original_changed,"proportions_changed_pixels":proportions_changed,"orbit_changed_pixels":orbit_changed,"zoom_changed_pixels":zoom_changed,"collect_changed_pixels":collect_changed,"reset_exact":true,"authoring_and_package_source_hidden":true,"preview_read_only":true,"key_use_undo_redo_save":true,"fresh_reopen_binding_exact":true,"fresh_reopen_viewport_pixels_exact":true,"normalized_viewport_size":[900,850],"actual_play_take_control_keyboard_interact":true,"key_placements_before_collect":2,"key_placements_after_collect":1,"key_placements_after_restart":2,"key_collected":collected_run.key_collected,"restart_key_collected":0,"restart_viewport_pixels_exact":true,"presentation_free_frame_bytes_exact":true,"presentation_free_checksums_exact":true,"recorded_ticks":reader.last_tick(),"recorded_checksums_checked":checked,"authoritative_checksum":format!("{live_checksum:016x}"),"source_and_installed_files_preserved":true});
    receipt
        .as_object_mut()
        .unwrap()
        .extend(lifecycle.as_object().unwrap().clone());
    fs::write(
        evidence.join("receipt.json"),
        serde_json::to_vec_pretty(&receipt).unwrap(),
    )
    .unwrap();
}
