//! Mandatory GPU consumer check for a generated three-role Room layout.
//! This exercises the producer output through project admission and the shared Room renderer.
#![cfg(feature = "room-project")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

use orr_bridge::FrameView;
use orr_ecs::{Entity, Frame};
use orr_fp::{FPVec3, FP};
use orr_games::room_escape_game::{RoomActor, RoomConfig, RoomEscapeV1, EXIT, KEY, PLAYER};
use orr_model_bindings::model_bindings::{self, Binding, Document, LocalTransform};
use orr_package::{Project, Runtime};
use orr_reflect::{decimal, Scene, Value};
use orr_sample::{
    room_project::{self, PreparedProject, PreparedScene, SEED},
    room_view,
};
use orr_sim::Simulation;
use serde_json::{json, Value as Json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
};

const PACKAGE: &str = "sample-imported-scene";
const MODEL: &str = "foreground.glb";
const BODY: &str = "orr_physics3d::Body";
const SIZE: u32 = 512;
const TARGETS: [(&str, u32, i32, i32); 3] = [
    ("player", PLAYER, -3, -3),
    ("key", KEY, 0, -3),
    ("exit", EXIT, 3, -3),
];

fn actor(frame: &Frame, kind: u32) -> Entity {
    frame
        .entities()
        .find(|&entity| {
            frame
                .get::<RoomActor>(entity)
                .is_some_and(|actor| actor.kind == kind)
        })
        .unwrap()
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let kind = entry.file_type().unwrap();
        assert!(
            !kind.is_symlink(),
            "fixture source must not contain symlinks"
        );
        let target = to.join(entry.file_name());
        if kind.is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            assert!(kind.is_file());
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().canonicalize().unwrap();
        let root = base.join("saved Room");
        let source = base.join("source package");
        fs::create_dir(&root).unwrap();
        copy_tree(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../assets/imported_scene_demo")
                .canonicalize()
                .unwrap(),
            &source,
        );
        let simulation = Simulation::<RoomEscapeV1>::new(RoomConfig, 60, SEED);
        let scene = Scene::unbake(&room_project::types(), simulation.frame(), None)
            .unwrap()
            .to_yaml();
        fs::write(root.join("room.scene.yaml"), &scene).unwrap();
        fs::write(
            root.join("orr.project.json"),
            serde_json::to_vec_pretty(&json!({
                "schema": 2,
                "engine": "*",
                "entry": {
                    "game": "room-escape-v1",
                    "scene": "room.scene.yaml",
                    "models": "room.models.json"
                }
            }))
            .unwrap(),
        )
        .unwrap();
        Project::open_for_install(&root, Runtime::content_only().engine_version)
            .unwrap()
            .install(std::slice::from_ref(&source))
            .unwrap();
        let loaded = model_bindings::load_asset(&root, PACKAGE, MODEL).unwrap();
        let binding = Binding::from_asset(
            PACKAGE.into(),
            MODEL.into(),
            &loaded,
            LocalTransform::default(),
        )
        .unwrap();
        let admitted = PreparedScene::parse(&scene).unwrap();
        let mut bindings = BTreeMap::new();
        for (_, kind, _, _) in TARGETS {
            let entity = actor(admitted.frame(), kind);
            bindings.insert(
                admitted.index().guid(entity).unwrap().to_string(),
                binding.clone(),
            );
        }
        let models = Document {
            version: 2,
            scene: "room.scene.yaml".into(),
            project: ".".into(),
            bindings,
        };
        fs::write(
            root.join("room.models.json"),
            serde_json::to_vec_pretty(&models).unwrap(),
        )
        .unwrap();
        Self { _temp: temp, root }
    }
}

fn tree(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let kind = entry.file_type().unwrap();
            assert!(!kind.is_symlink(), "admitted project contains a symlink");
            if kind.is_dir() {
                visit(root, &entry.path(), out);
            } else {
                assert!(kind.is_file());
                out.insert(
                    entry.path().strip_prefix(root).unwrap().to_owned(),
                    fs::read(entry.path()).unwrap(),
                );
            }
        }
    }
    let mut files = BTreeMap::new();
    visit(root, root, &mut files);
    files
}

fn fixed(value: &Json) -> FP {
    let text = value
        .as_str()
        .map(str::to_owned)
        .or_else(|| value.as_number().map(ToString::to_string))
        .expect("position coordinate must be an exact JSON number");
    decimal::parse_fp(&text).unwrap()
}

fn apply_layout(prepared: &PreparedProject, bytes: &[u8]) -> (orr_edit::EditorDoc, Vec<[FP; 3]>) {
    let ops: Json = serde_json::from_slice(bytes).unwrap();
    let ops = ops
        .as_array()
        .expect("layout output must be an operation array");
    assert_eq!(
        ops.len(),
        TARGETS.len(),
        "layout must contain three patches"
    );
    let mut doc = orr_edit::EditorDoc::from_yaml(
        prepared.scene().text(),
        room_project::types(),
        Simulation::<RoomEscapeV1>::build_registry(),
        SEED,
    )
    .unwrap();
    let mut next_positions = Vec::new();
    let mut seen = BTreeSet::new();
    for (op_index, op) in ops.iter().enumerate() {
        let object = op.as_object().unwrap();
        assert_eq!(
            object.keys().map(String::as_str).collect::<BTreeSet<_>>(),
            ["op", "entity", "component", "path", "value"]
                .into_iter()
                .collect()
        );
        assert_eq!(op["op"], "patch");
        assert_eq!(op["component"], BODY);
        assert_eq!(op["path"], "pos");
        let guid_text = op["entity"].as_str().unwrap();
        let expected = TARGETS
            .iter()
            .find_map(|(role, kind, target_x, target_z)| {
                let entity = actor(prepared.scene().frame(), *kind);
                (prepared.scene().index().guid(entity).unwrap().as_str() == guid_text)
                    .then_some((*role, entity, *target_x, *target_z))
            })
            .expect("patch must target an existing objective-role GUID");
        assert_eq!(
            expected.0, TARGETS[op_index].0,
            "patch order must be player, key, exit"
        );
        assert!(seen.insert(expected.0), "duplicate role patch");
        let coordinates = op["value"].as_array().unwrap();
        assert_eq!(coordinates.len(), 3);
        let entity = expected.1;
        let position_y = prepared
            .scene()
            .frame()
            .get::<orr_physics3d::Body>(entity)
            .unwrap()
            .pos
            .y;
        let position = [
            fixed(&coordinates[0]),
            fixed(&coordinates[1]),
            fixed(&coordinates[2]),
        ];
        assert_eq!(
            position,
            [
                FP::from_int(expected.2),
                position_y,
                FP::from_int(expected.3)
            ]
        );
        doc.apply(
            orr_edit::Op::SetField {
                guid: orr_reflect::Guid::parse(guid_text).unwrap(),
                component: BODY.into(),
                path: "pos".into(),
                value: Value::Vec3(FPVec3::new(position[0], position[1], position[2])),
            },
            orr_edit::Origin::User,
        )
        .unwrap();
        next_positions.push(position);
    }
    assert_eq!(seen, ["player", "key", "exit"].into_iter().collect());
    (doc, next_positions)
}

fn render(
    gpu: &orr_render::orr_rhi::Wgpu,
    prepared: &PreparedProject,
    scene: &PreparedScene,
) -> Vec<u8> {
    let before = scene.frame().to_bytes();
    let mut renderer = room_view::RoomRenderer::new(gpu, (SIZE, SIZE), prepared.models()).unwrap();
    renderer
        .render(
            FrameView::of(scene.frame()),
            scene.index(),
            prepared.models(),
            &room_view::camera((SIZE, SIZE)),
        )
        .unwrap();
    assert_eq!(
        scene.frame().to_bytes(),
        before,
        "Room rendering mutated the scene"
    );
    renderer.read_rgba8()
}

fn save_capture(dir: &Path, name: &str, bytes: &[u8]) -> Json {
    let path = dir.join(name);
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .unwrap();
    use std::io::Write;
    file.write_all(bytes).unwrap();
    file.sync_all().unwrap();
    json!({"file": name, "bytes": bytes.len(), "fnv1a64": fnv(bytes)})
}

fn save_png(dir: &Path, name: &str, rgba: &[u8]) -> Json {
    let mut encoded = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut encoded, SIZE, SIZE);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(rgba)
            .unwrap();
    }
    save_capture(dir, name, &encoded)
}

fn pixel_delta_in(image_a: &[u8], image_b: &[u8], center: [f32; 2]) -> usize {
    let (center_x, center_y) = (center[0].round() as i32, center[1].round() as i32);
    let mut changed = 0;
    for pixel_y in (center_y - 8).max(0)..(center_y + 8).min(SIZE as i32) {
        for pixel_x in (center_x - 8).max(0)..(center_x + 8).min(SIZE as i32) {
            let offset = ((pixel_y as usize * SIZE as usize) + pixel_x as usize) * 4;
            if image_a[offset..offset + 4] != image_b[offset..offset + 4] {
                changed += 1;
            }
        }
    }
    changed
}

fn fnv(bytes: &[u8]) -> String {
    format!(
        "{:016x}",
        bytes.iter().fold(0xcbf29ce484222325u64, |hash, byte| (hash
            ^ u64::from(*byte))
        .wrapping_mul(0x100000001b3))
    )
}

#[test]
#[ignore = "mandatory gate: built Room layout CLI, software GPU and fresh capture directory"]
fn generated_room_layout_moves_all_three_roles_in_the_gpu_view() {
    use orr_render::orr_rhi::{Wgpu, WgpuOptions};

    assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
    let capture_dir = PathBuf::from(
        std::env::var_os("ORR_ROOM_LAYOUT_CAPTURE_DIR").expect("required retained captures"),
    );
    assert!(capture_dir.is_absolute());
    fs::create_dir(&capture_dir).expect("capture directory must be new");
    let producer = PathBuf::from(
        std::env::var_os("ORR_ROOM_LAYOUT_BIN").expect("required built room_layout example"),
    );
    assert!(producer.is_absolute() && producer.is_file());

    let gpu = Wgpu::headless(WgpuOptions {
        force_software: true,
        ..WgpuOptions::default()
    })
    .expect("required software GPU unavailable");
    assert!(gpu.is_software(), "GPU gate requires a software adapter");
    let info = gpu.adapter().get_info();
    let adapter = json!({
        "name": info.name,
        "backend": format!("{:?}", info.backend),
        "device_type": format!("{:?}", info.device_type),
        "vendor": info.vendor,
        "device": info.device,
        "driver": info.driver,
        "driver_info": info.driver_info,
        "software": gpu.is_software()
    });
    let adapter_capture = save_capture(
        &capture_dir,
        "adapter.json",
        &serde_json::to_vec_pretty(&adapter).unwrap(),
    );

    let fixture = Fixture::new();
    let prepared = PreparedProject::open(&fixture.root).expect("saved fixture must be admitted");
    let project_before = tree(&fixture.root);
    let initial = prepared.scene().frame().to_bytes();
    let output = fixture._temp.path().join("layout.ops.json");
    let result = Command::new(&producer)
        .arg("--project")
        .arg(&fixture.root)
        .args([
            "--player", "-3", "-3", "--key", "0", "-3", "--exit", "3", "-3", "--output",
        ])
        .arg(&output)
        .output()
        .expect("run room_layout producer");
    assert!(
        result.status.success(),
        "room_layout failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(
        result.stdout.is_empty(),
        "producer must write only the requested file"
    );
    let ops = fs::read(&output).expect("producer must create its operations file");
    let (doc, positions) = apply_layout(&prepared, &ops);
    let authored_scene =
        PreparedScene::parse(&doc.to_yaml()).expect("layout scene must be admitted");
    assert_ne!(authored_scene.frame().to_bytes(), initial);
    assert_eq!(
        tree(&fixture.root),
        project_before,
        "producer changed saved project files"
    );
    let reopened = PreparedProject::open(&fixture.root).unwrap();
    assert_eq!(
        reopened.scene().frame().to_bytes(),
        initial,
        "producer changed admitted baseline"
    );
    assert_eq!(reopened.models().document, prepared.models().document);
    assert_eq!(
        reopened.models().assets.len(),
        prepared.models().assets.len()
    );

    let layout_root = fixture._temp.path().join("admitted layout");
    copy_tree(&fixture.root, &layout_root);
    fs::write(layout_root.join("room.scene.yaml"), doc.to_yaml()).unwrap();
    let layout = PreparedProject::open(&layout_root).expect("generated layout must be admitted");
    assert_eq!(layout.models().document, prepared.models().document);
    assert_eq!(
        layout.camera().map(|camera| &camera.bytes),
        prepared.camera().map(|camera| &camera.bytes)
    );

    let before_pixels = render(&gpu, &prepared, prepared.scene());
    let after_pixels = render(&gpu, &layout, layout.scene());
    assert_eq!(before_pixels.len(), (SIZE * SIZE * 4) as usize);
    assert_eq!(after_pixels.len(), before_pixels.len());
    let camera = room_view::camera((SIZE, SIZE));
    let mut role_deltas = Vec::new();
    for (slot, (_, kind, _, _)) in TARGETS.iter().enumerate() {
        let entity = actor(prepared.scene().frame(), *kind);
        let baseline_position = prepared
            .scene()
            .frame()
            .get::<orr_physics3d::Body>(entity)
            .unwrap()
            .pos;
        let old_screen = camera
            .world_to_screen(
                [
                    baseline_position.x.to_f32(),
                    baseline_position.y.to_f32(),
                    baseline_position.z.to_f32(),
                ],
                (SIZE, SIZE),
            )
            .unwrap();
        let layout_position = positions[slot];
        let new_screen = camera
            .world_to_screen(
                [
                    layout_position[0].to_f32(),
                    layout_position[1].to_f32(),
                    layout_position[2].to_f32(),
                ],
                (SIZE, SIZE),
            )
            .unwrap();
        let old_delta = pixel_delta_in(&before_pixels, &after_pixels, old_screen);
        let new_delta = pixel_delta_in(&before_pixels, &after_pixels, new_screen);
        assert!(
            old_delta > 4,
            "{kind} movement left no visible change at its old position"
        );
        assert!(
            new_delta > 4,
            "{kind} movement left no visible change at its new position"
        );
        role_deltas.push(json!({"kind": kind, "old_pixels": old_delta, "new_pixels": new_delta}));
    }
    let mut total_delta = 0;
    for (before, after) in before_pixels
        .chunks_exact(4)
        .zip(after_pixels.chunks_exact(4))
    {
        if before != after {
            total_delta += 1;
        }
    }
    assert!(
        total_delta > 100,
        "all role moves should alter visible pixels"
    );

    let captures = [
        adapter_capture,
        save_capture(&capture_dir, "baseline.rgba", &before_pixels),
        save_capture(&capture_dir, "layout.rgba", &after_pixels),
        save_png(&capture_dir, "baseline.png", &before_pixels),
        save_png(&capture_dir, "layout.png", &after_pixels),
        save_capture(&capture_dir, "layout.ops.json", &ops),
    ];
    let receipt = json!({
        "schema": 1,
        "status": "passed",
        "width": SIZE,
        "height": SIZE,
        "adapter": adapter,
        "baseline_checksum": format!("{:016x}", prepared.scene().frame().checksum()),
        "authored_checksum": format!("{:016x}", layout.scene().frame().checksum()),
        "project_files_unchanged": true,
        "role_pixel_deltas": role_deltas,
        "total_changed_pixels": total_delta,
        "producer": producer.file_name().unwrap().to_string_lossy(),
        "producer_fnv1a64": fnv(&fs::read(&producer).unwrap()),
        "test_fnv1a64": fnv(include_bytes!("room_layout_gpu.rs")),
        "captures": captures
    });
    save_capture(
        &capture_dir,
        "receipt.json",
        &serde_json::to_vec_pretty(&receipt).unwrap(),
    );
}
