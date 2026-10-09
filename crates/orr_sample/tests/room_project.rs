//! CPU admission/ownership checks are separate from explicit GPU acceptance.
//! Run the ignored readback check with ORR_REQUIRE_GPU=1 and --ignored.
//! This file does not claim editor authoring or executable-export acceptance.
#![cfg(feature = "room-project")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

use orr_bridge::{FrameView, PlayerSlot};
use orr_ecs::{Entity, Frame};
use orr_model_bindings::model_bindings::{self, Binding, Document, LocalTransform};
use orr_package::{Project, Runtime};
use orr_physics3d::Body;
use orr_reflect::Scene;
use orr_sample::{
    room_game::{
        RoomActor, RoomConfig, RoomEscapeV1, RoomInput, RoomRun, INTERACT, KEY, PLAYER, WALL,
    },
    room_project::{self, PreparedProject, PreparedScene, SEED},
    room_view,
};
use orr_sim::{Simulation, TickInputs};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

const PACKAGE: &str = "sample-imported-scene";
const MODEL: &str = "foreground.glb";

fn actor(frame: &Frame, kind: u32) -> Entity {
    frame
        .entities()
        .find(|&e| frame.get::<RoomActor>(e).is_some_and(|a| a.kind == kind))
        .unwrap()
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let kind = entry.file_type().unwrap();
        assert!(
            !kind.is_symlink(),
            "fixtures must not depend on source symlinks"
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
    source: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().canonicalize().unwrap();
        let root = base.join("authored room");
        let source = base.join("source package");
        fs::create_dir(&root).unwrap();
        copy_tree(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../assets/imported_scene_demo")
                .canonicalize()
                .unwrap(),
            &source,
        );
        let sim = Simulation::<RoomEscapeV1>::new(RoomConfig, 60, SEED);
        let scene = Scene::unbake(&room_project::types(), sim.frame(), None)
            .unwrap()
            .to_yaml();
        fs::write(root.join("room.scene.yaml"), &scene).unwrap();
        fs::write(root.join("orr.project.json"), serde_json::to_vec_pretty(&serde_json::json!({
            "schema":2,"engine":"*","entry":{"game":"room-escape-v1","scene":"room.scene.yaml","models":"room.models.json"}
        })).unwrap()).unwrap();
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
        for kind in [PLAYER, KEY] {
            let entity = actor(admitted.frame(), kind);
            bindings.insert(
                admitted.index().guid(entity).unwrap().to_string(),
                binding.clone(),
            );
        }
        let doc = Document {
            version: 2,
            scene: "room.scene.yaml".into(),
            project: ".".into(),
            bindings,
        };
        fs::write(
            root.join("room.models.json"),
            serde_json::to_vec_pretty(&doc).unwrap(),
        )
        .unwrap();
        Self {
            _temp: temp,
            root,
            source,
        }
    }
    fn json(&self, file: &str, change: impl FnOnce(&mut serde_json::Value)) {
        let path = self.root.join(file);
        let mut value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        change(&mut value);
        fs::write(path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    }
    fn rejected(&self) {
        assert!(
            PreparedProject::open(&self.root).is_err(),
            "invalid authored project was admitted"
        );
    }
}

fn step(sim: &mut Simulation<RoomEscapeV1>, input: RoomInput) {
    let mut inputs = TickInputs::new(sim.tick(), 1);
    inputs.set_input(PlayerSlot(0), input);
    sim.step(&inputs);
}

#[test]
fn prepared_initial_frame_session_and_simulation_have_exact_parity() {
    let fixture = Fixture::new();
    let prepared = PreparedProject::open(&fixture.root).unwrap();
    assert_eq!(prepared.root(), fixture.root);
    assert_eq!(prepared.models().assets.len(), 1);
    assert_eq!(prepared.models().document.bindings.len(), 2);
    let initial = prepared.scene().frame();
    let mut simulation = prepared.scene().simulation().unwrap();
    let direct =
        Simulation::<RoomEscapeV1>::from_frame(initial, 60, room_project::build_id()).unwrap();
    let mut session = prepared.scene().session().unwrap();
    for frame in [simulation.frame(), direct.frame(), session.frame()] {
        assert_eq!(frame.to_bytes(), initial.to_bytes());
        assert_eq!(frame.checksum(), initial.checksum());
    }
    for tick in 0..90 {
        let input = RoomInput {
            move_x: 1,
            move_z: if tick < 30 { 0 } else { -1 },
            buttons: if tick % 7 == 0 { INTERACT } else { 0 },
            ..RoomInput::default()
        };
        step(&mut simulation, input);
        session.set_input(PlayerSlot(0), input);
        assert!(session.step_now().is_some());
        assert_eq!(simulation.frame().to_bytes(), session.frame().to_bytes());
        assert_eq!(simulation.frame().checksum(), session.frame().checksum());
    }
    assert_eq!(prepared.scene().frame().to_bytes(), initial.to_bytes());
}

#[test]
fn owned_scene_and_models_survive_removal_of_every_source() {
    let fixture = Fixture::new();
    let prepared = PreparedProject::open(&fixture.root).unwrap();
    let before = prepared.scene().frame().to_bytes();
    fs::remove_dir_all(&fixture.source).unwrap();
    fs::remove_dir_all(&fixture.root).unwrap();
    assert!(PreparedProject::open(&fixture.root).is_err());
    let mut a = prepared.scene().simulation().unwrap();
    let mut b = prepared.scene().session().unwrap();
    for tick in 0..40 {
        let input = RoomInput {
            buttons: if tick == 0 { INTERACT } else { 0 },
            move_z: 1,
            ..RoomInput::default()
        };
        step(&mut a, input);
        b.set_input(PlayerSlot(0), input);
        assert!(b.step_now().is_some());
        assert_eq!(a.frame().to_bytes(), b.frame().to_bytes());
        let bytes = a.frame().to_bytes();
        let placed = room_view::placements(
            FrameView::of(a.frame()),
            prepared.scene().index(),
            prepared.models(),
        )
        .unwrap();
        assert_eq!(
            placed.len(),
            1,
            "collected key is hidden; owned player model remains"
        );
        assert!(!placed[0].model.source().primitives.is_empty());
        assert!(!room_view::items(FrameView::of(a.frame())).is_empty());
        assert_eq!(
            bytes,
            a.frame().to_bytes(),
            "CPU presentation must be read-only"
        );
    }
    assert_eq!(a.frame().singleton::<RoomRun>().key_collected, 1);
    assert_eq!(prepared.scene().frame().to_bytes(), before);
    assert_eq!(
        prepared.scene().session().unwrap().frame().to_bytes(),
        before
    );
}

#[test]
fn project_manifest_rejects_unsupported_routes_and_escaping_model_paths() {
    for (field, value) in [
        ("game", serde_json::json!("unknown-room")),
        ("game", serde_json::json!("arena")),
        (
            "ui",
            serde_json::json!({"profile":"arena-korean-v1","font":{"package":"font","asset":"font.otf"}}),
        ),
        ("sprites", serde_json::json!("sprites.json")),
        ("models", serde_json::json!("../room.models.json")),
        ("models", serde_json::json!("nested/room.models.json")),
    ] {
        let fixture = Fixture::new();
        fixture.json("orr.project.json", |doc| doc["entry"][field] = value);
        fixture.rejected();
    }
    let fixture = Fixture::new();
    fixture.json("orr.project.json", |doc| doc["progress"] = serde_json::json!({"schema":1,"game_id":"12345678-1234-4234-8234-123456789abc","profile":"collect-dodge-highscore-v1"}));
    fixture.rejected();
}

#[test]
fn sidecar_rejects_redirected_roots_unknown_guids_missing_packages_and_stale_identity() {
    for case in 0..7 {
        let fixture = Fixture::new();
        fixture.json("room.models.json", |doc| match case {
            0 => doc["project"] = serde_json::json!(".."),
            1 => doc["scene"] = serde_json::json!("other.scene.yaml"),
            2 => {
                let bindings = doc["bindings"].as_object_mut().unwrap();
                let binding = bindings.values().next().unwrap().clone();
                bindings.insert("e_ffffffff".into(), binding);
            }
            _ => {
                let binding = doc["bindings"]
                    .as_object_mut()
                    .unwrap()
                    .values_mut()
                    .next()
                    .unwrap();
                match case {
                    3 => binding["package"] = serde_json::json!("missing-package"),
                    4 => binding["source_hash"] = serde_json::json!("0".repeat(64)),
                    5 => binding["package_digest"] = serde_json::json!("0".repeat(64)),
                    _ => binding["asset"] = serde_json::json!("../foreground.glb"),
                }
            }
        });
        fixture.rejected();
    }
    let fixture = Fixture::new();
    fs::remove_dir_all(fixture.root.join(".orr/packages/objects")).unwrap();
    fixture.rejected();
}

#[test]
fn authored_scene_rejects_initial_overlap_and_noncanonical_roles() {
    for case in 0..3 {
        let mut sim = Simulation::<RoomEscapeV1>::new(RoomConfig, 60, SEED);
        let player = actor(sim.frame(), PLAYER);
        match case {
            0 => {
                let wall = actor(sim.frame(), WALL);
                let position = sim.frame().get::<Body>(wall).unwrap().pos;
                sim.frame_mut().get_mut::<Body>(player).unwrap().pos = position;
            }
            1 => sim.frame_mut().get_mut::<RoomActor>(player).unwrap().kind = KEY,
            _ => {
                sim.frame_mut()
                    .get_mut::<RoomActor>(player)
                    .unwrap()
                    .ordinal = 1
            }
        }
        let yaml = Scene::unbake(&room_project::types(), sim.frame(), None)
            .unwrap()
            .to_yaml();
        assert!(PreparedScene::parse(&yaml).is_err());
    }
}

#[test]
#[ignore = "explicit GPU acceptance: requires ORR_REQUIRE_GPU=1 and a working adapter"]
fn owned_real_model_offscreen_readback_is_nonblank_and_read_only() {
    use orr_render::orr_rhi::{Wgpu, WgpuOptions};
    assert_eq!(
        std::env::var("ORR_REQUIRE_GPU").as_deref(),
        Ok("1"),
        "GPU acceptance must be explicitly requested"
    );
    let fixture = Fixture::new();
    let prepared = PreparedProject::open(&fixture.root).unwrap();
    let model = prepared
        .models()
        .assets
        .values()
        .next()
        .unwrap()
        .static_model()
        .unwrap();
    assert!(
        model
            .source()
            .primitives
            .iter()
            .flat_map(|p| &p.vertices)
            .any(|v| v.uv[0] > 0.0 || v.uv[1] > 0.0),
        "real UV-bearing fixture is required"
    );
    fs::remove_dir_all(&fixture.root).unwrap();
    fs::remove_dir_all(&fixture.source).unwrap();
    let (_, _, scene, mut models) = prepared.into_parts();
    let gpu = Wgpu::headless(WgpuOptions::default()).expect("required GPU adapter unavailable");
    let mut renderer = room_view::RoomRenderer::new(&gpu, (512, 384), &models).unwrap();
    let frame = scene.frame();
    let before = frame.to_bytes();
    renderer
        .render(
            FrameView::of(frame),
            scene.index(),
            &models,
            &room_view::camera((512, 384)),
        )
        .unwrap();
    let image = renderer.read_rgba8();
    assert_eq!(image.len(), 512 * 384 * 4);
    let first = &image[..4];
    let changed = image.chunks(4).filter(|pixel| *pixel != first).count();
    assert!(
        changed > 100,
        "offscreen readback must contain visible scene geometry"
    );
    // Move only presentation-local model transforms outside the camera. The
    // exact same host, procedural geometry, lighting and GPU assets remain.
    // A differing readback proves imported models contributed visible pixels.
    for binding in models.document.bindings.values_mut() {
        binding.transform.translation = [1000.0, 1000.0, 1000.0];
    }
    renderer
        .render(
            FrameView::of(frame),
            scene.index(),
            &models,
            &room_view::camera((512, 384)),
        )
        .unwrap();
    let without_models = renderer.read_rgba8();
    let model_pixels = image
        .chunks(4)
        .zip(without_models.chunks(4))
        .filter(|(a, b)| a != b)
        .count();
    assert!(
        model_pixels > 20,
        "real imported models must visibly contribute to readback"
    );
    assert_eq!(
        frame.to_bytes(),
        before,
        "GPU rendering must not mutate the host frame"
    );
}

/// This is only an early-rejection assertion, not executable export acceptance.
/// A nonexistent runtime makes any accidental advance past project admission
/// fail with a different diagnostic; no smoke command can execute.
fn assert_presentation_rejected_before_export(fixture: &Fixture, diagnostic: &str) {
    let error = match PreparedProject::open(&fixture.root) {
        Ok(_) => panic!("presentation-invalid project was admitted"),
        Err(error) => error,
    };
    assert!(error.contains(diagnostic), "unexpected rejection: {error}");
    #[cfg(all(
        feature = "project-export",
        target_os = "linux",
        target_arch = "x86_64"
    ))]
    {
        let output = fixture.root.parent().unwrap().join("rejected export");
        let result =
            orr_sample::project_export::export_room(&orr_sample::project_export::ExportOptions {
                project: fixture.root.clone(),
                runtime: fixture.root.parent().unwrap().join("missing runtime"),
                runtime_sha256: "0".repeat(64),
                output: output.clone(),
                trusted_runtime: true,
                source_revision: None,
            });
        let error = result.expect_err("export must reject presentation before runtime or smoke");
        assert!(
            error.contains(diagnostic),
            "export advanced past presentation admission: {error}"
        );
        assert!(!output.exists());
        assert!(fs::read_dir(fixture.root.parent().unwrap())
            .unwrap()
            .all(|entry| {
                !entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".orr-export-")
            }));
    }
}

#[test]
fn admission_rejects_player_transform_that_exceeds_bounds_only_after_movement() {
    let fixture = Fixture::new();
    let prepared = PreparedProject::open(&fixture.root).unwrap();
    let player = actor(prepared.scene().frame(), PLAYER);
    let guid = prepared.scene().index().guid(player).unwrap().to_string();
    let mut binding = prepared.models().document.bindings[&guid].clone();
    binding.transform.translation = [999_990.0, 0.0, 0.0];
    let loaded = &prepared.models().assets[&(binding.package.clone(), binding.asset.clone())];
    binding.validate(loaded).unwrap();
    // The initial origin is valid even under composed model geometry checks.
    orr_render::StaticInstance {
        translation: binding.transform.translation,
        rotation: binding.transform.rotation,
        scale: binding.transform.scale,
        material_override: binding.material_override,
    }
    .validate_for(loaded.static_model().unwrap())
    .unwrap();
    fixture.json("room.models.json", |doc| {
        doc["bindings"][&guid] = serde_json::to_value(binding).unwrap();
    });
    assert_presentation_rejected_before_export(&fixture, "reachable model placement");
}

#[test]
fn admission_rejects_reused_model_aggregate_draw_budget_before_export_smoke() {
    const BUDGET_PACKAGE: &str = "room-draw-budget";
    const BUDGET_ASSET: &str = "budget.model.json";
    let fixture = Fixture::new();
    let loaded = model_bindings::load_asset(&fixture.root, PACKAGE, MODEL).unwrap();
    let mut source = loaded.static_model().unwrap().source().clone();
    let mut triangle = source.primitives[0].clone();
    triangle.vertices.truncate(3);
    triangle.indices = vec![0, 1, 2];
    source.asset_id = BUDGET_ASSET.into();
    source.primitives = (0..1024)
        .map(|i| {
            let mut primitive = triangle.clone();
            primitive.id = format!("{BUDGET_ASSET}#node={i}/mesh=0/primitive=0");
            primitive
        })
        .collect();
    let model = orr_model::StaticModel::new(source).unwrap();
    assert_eq!(model.source().primitives.len(), 1024);
    let package = fixture.root.parent().unwrap().join("draw budget package");
    fs::create_dir(&package).unwrap();
    fs::write(package.join(BUDGET_ASSET), model.to_bytes().unwrap()).unwrap();
    fs::write(
        package.join("orr.package.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema":1,"name":BUDGET_PACKAGE,"version":"1.0.0","engine":"^0.0.1",
            "capabilities":["models"],"dependencies":{},"files":[BUDGET_ASSET]
        }))
        .unwrap(),
    )
    .unwrap();
    Project::open_for_install(&fixture.root, Runtime::content_only().engine_version)
        .unwrap()
        .install(&[fixture.source.clone(), package])
        .unwrap();
    let loaded = model_bindings::load_asset(&fixture.root, BUDGET_PACKAGE, BUDGET_ASSET).unwrap();
    let binding = Binding::from_asset(
        BUDGET_PACKAGE.into(),
        BUDGET_ASSET.into(),
        &loaded,
        LocalTransform::default(),
    )
    .unwrap();
    binding.validate(&loaded).unwrap();
    let scene =
        PreparedScene::parse(&fs::read_to_string(fixture.root.join("room.scene.yaml")).unwrap())
            .unwrap();
    let bindings = scene
        .index()
        .iter()
        .take(4)
        .map(|(guid, _)| (guid.to_string(), binding.clone()))
        .collect();
    let document = Document {
        version: 2,
        scene: "room.scene.yaml".into(),
        project: ".".into(),
        bindings,
    };
    document.validate().unwrap();
    assert_eq!(document.bindings.len(), 4);
    fs::write(
        fixture.root.join("room.models.json"),
        serde_json::to_vec_pretty(&document).unwrap(),
    )
    .unwrap();
    // Four valid bindings reuse a single small immutable asset. Unique-asset
    // admission alone misses 4096 imported draws plus procedural reservations.
    assert_presentation_rejected_before_export(&fixture, "draw limit");
}

#[test]
fn authored_camera_is_owned_presentation_without_simulation_changes() {
    let f=Fixture::new();
    let legacy=PreparedProject::open(&f.root).unwrap();
    assert!(legacy.camera().is_none());
    let checksum=legacy.scene().frame().checksum();
    let doc=orr_sample::room_camera::Document::readable_default();
    fs::write(f.root.join("room.camera.json"),doc.to_bytes().unwrap()).unwrap();
    f.json("orr.project.json",|m|m["entry"]["camera"]="room.camera.json".into());
    let authored=PreparedProject::open(&f.root).unwrap();
    assert_eq!(authored.scene().frame().checksum(),checksum);
    let camera=authored.camera().unwrap();
    assert_eq!(camera.document,doc);
    assert_eq!(camera.bytes,doc.to_bytes().unwrap());
    fs::remove_dir_all(&f.root).unwrap();
    assert_eq!(authored.scene().frame().checksum(),checksum);
    assert_eq!(camera.document.camera(&doc.orbit(),(1024,768)).unwrap(),doc.camera(&doc.orbit(),(1024,768)).unwrap());
}

#[test]
fn authored_camera_routes_paths_and_documents_fail_closed() {
    for value in [serde_json::Value::Null,serde_json::json!("../camera.json"),serde_json::json!("/camera.json"),serde_json::json!("nested/camera.json"),serde_json::json!("room.scene.yaml"),serde_json::json!("ROOM.MODELS.JSON"),serde_json::json!("orr.project.json"),serde_json::json!("orr.packages.lock.json"),serde_json::json!(".orr") ] {
        let f=Fixture::new(); f.json("orr.project.json",|m|m["entry"]["camera"]=value); f.rejected();
    }
    for bytes in [b"null".as_slice(),b"{}".as_slice(),&vec![b' ';4097]] {
        let f=Fixture::new(); fs::write(f.root.join("room.camera.json"),bytes).unwrap();
        f.json("orr.project.json",|m|m["entry"]["camera"]="room.camera.json".into()); f.rejected();
    }
    for game in ["arena","collect-dodge-v1"] {
        let f=Fixture::new(); f.json("orr.project.json",|m| {m["entry"]["game"]=game.into();m["entry"]["camera"]="camera.json".into();m["entry"].as_object_mut().unwrap().remove("models");});
        assert!(Project::open(&f.root,Runtime::content_only()).is_err());
    }
}
#[cfg(unix)]
#[test]
fn camera_admission_rejects_nonregular_and_symlink_documents() {
    let f=Fixture::new(); f.json("orr.project.json",|m|m["entry"]["camera"]="camera.json".into());
    fs::create_dir(f.root.join("camera.json")).unwrap();f.rejected();fs::remove_dir(f.root.join("camera.json")).unwrap();
    fs::write(f.root.join("other.json"),orr_sample::room_camera::Document::readable_default().to_bytes().unwrap()).unwrap();
    std::os::unix::fs::symlink(f.root.join("other.json"),f.root.join("camera.json")).unwrap();f.rejected();
}
