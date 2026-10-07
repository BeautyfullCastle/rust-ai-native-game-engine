//! End-to-end headless proof for optional scene-pinned terrain admission.
#![cfg(feature = "terrain-physics")]
#![allow(clippy::disallowed_types)]
use orr_ecs::Frame;
use orr_edit::{EditorDoc, Op, Origin, PlayController, Target};
use orr_fp::{fp, FPVec3, FP};
use orr_reflect::{Guid, Value};
use orr_remote::terrain_yard3d::*;
use orr_remote::{Auth, Caps, ErpClient, ServerConfig};
use orr_session::{ControlOp, PlaySession};
use orr_sim::{Simulation, TickInputs};
use orr_terrain_physics3d::asset::{reconstruct_terrain, TerrainAsset};
use serde_json::json;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
const YAML: &str = include_str!("../../../scenes/terrain_sphere.scene.yaml");
const ASSET: &[u8] = include_bytes!("../../../scenes/terrain/sphere_demo.orrt");
const BODY: &str = "orr_physics3d::Body";
const COLLIDER: &str = "orr_physics3d::Collider";
struct Fixture {
    root: PathBuf,
    scene: PathBuf,
    asset: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "orr_terrain_host_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("terrain")).unwrap();
        let scene = root.join("terrain.scene.yaml");
        let asset = root.join("terrain/sphere_demo.orrt");
        fs::write(&scene, YAML).unwrap();
        fs::write(&asset, ASSET).unwrap();
        Self { root, scene, asset }
    }
    fn doc(&self) -> EditorDoc {
        terrain_yard3d_doc_from_path(&self.scene).unwrap()
    }
    fn restore(&self) {
        fs::write(&self.asset, ASSET).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn guid() -> Guid {
    Guid::parse("e_00000001").unwrap()
}
fn lift(y: i32) -> Op {
    Op::SetField {
        guid: guid(),
        component: BODY.into(),
        path: "pos.y".into(),
        value: Value::Fixed(FP::from_int(y)),
    }
}
fn snapshot(doc: &EditorDoc) -> (String, Vec<u8>, u64, bool, bool, bool) {
    (
        doc.to_yaml(),
        doc.frame().to_bytes(),
        doc.revision(),
        doc.is_dirty(),
        doc.can_undo(),
        doc.can_redo(),
    )
}
fn y(doc: &EditorDoc, frame: &Frame, name: &str) -> FP {
    let entity = doc.index().entity(&Guid::parse(name).unwrap()).unwrap();
    match doc.types().get_field(frame, entity, BODY, "pos.y").unwrap() {
        Value::Fixed(y) => y,
        _ => panic!("fixed y"),
    }
}

#[test]
fn saved_scene_pins_full_asset_and_every_edit_undo_redo_proposal_readmits() {
    let fixture = Fixture::new();
    let mut doc = fixture.doc();
    let initial = doc.frame().to_bytes();
    assert_eq!(
        doc.scene().entities.len(),
        2,
        "no authored floor or terrain proxy"
    );
    assert_eq!(doc.frame().alive_count(), 3, "one derived terrain collider");
    assert_eq!(doc.to_yaml(), YAML);
    assert_eq!(
        reconstruct_terrain(doc.frame()).unwrap().unwrap().cook(),
        ASSET
    );
    assert_eq!(
        doc.frame().singleton::<TerrainAsset>().revision,
        doc.frame().singleton::<TerrainScenePin>().revision
    );
    let saved = doc.save_yaml();
    assert_eq!(
        terrain_yard3d_doc_from_yaml(
            &saved,
            LocalTerrainAdmission::for_scene(&fixture.scene).unwrap()
        )
        .unwrap()
        .frame()
        .to_bytes(),
        initial
    );

    doc.apply(lift(4), Origin::User).unwrap();
    let lifted = doc.frame().to_bytes();
    assert_ne!(lifted, initial);
    doc.undo().unwrap();
    assert_eq!(doc.frame().to_bytes(), initial);
    doc.redo().unwrap();
    assert_eq!(doc.frame().to_bytes(), lifted);
    let proposal = doc.propose("lift sphere", Origin::User).unwrap();
    doc.proposal_apply(proposal, lift(5)).unwrap();
    let before = snapshot(&doc);
    fs::write(&fixture.asset, b"malformed terrain").unwrap();
    assert!(doc.apply(lift(6), Origin::User).is_err());
    assert_eq!(snapshot(&doc), before);
    assert!(doc.undo().is_err());
    assert_eq!(snapshot(&doc), before);
    assert!(doc.proposal_apply(proposal, lift(6)).is_err());
    assert_eq!(snapshot(&doc), before);
    assert!(doc.accept(proposal).is_err());
    assert_eq!(snapshot(&doc), before);
    assert!(doc.load_yaml(YAML).is_err());
    assert_eq!(snapshot(&doc), before);
    fixture.restore();
    doc.accept(proposal).unwrap();
    assert_eq!(y(&doc, doc.frame(), "e_00000001"), fp!(5));
    doc.undo().unwrap();
    assert_eq!(doc.frame().to_bytes(), lifted);
    let before = snapshot(&doc);
    fs::remove_file(&fixture.asset).unwrap();
    assert!(doc.redo().is_err());
    assert_eq!(snapshot(&doc), before);
    fixture.restore();
    doc.redo().unwrap();
    assert_eq!(y(&doc, doc.frame(), "e_00000001"), fp!(5));
}

#[test]
fn invalid_revision_identity_shapes_and_numeric_profiles_leave_doc_atomic() {
    let fixture = Fixture::new();
    let mut doc = fixture.doc();
    let mut operations = vec![
        Op::SetSingletonField {
            singleton: PIN_NAME.into(),
            path: "revision[31]".into(),
            value: Value::Int(0),
        },
        Op::SetSingletonField {
            singleton: PIN_NAME.into(),
            path: "asset_id[0]".into(),
            value: Value::Int(88),
        },
        Op::RemoveSingleton {
            singleton: PIN_NAME.into(),
        },
        Op::SetField {
            guid: guid(),
            component: BODY.into(),
            path: "kind".into(),
            value: Value::Enum("static".into()),
        },
        Op::SetSingletonField {
            singleton: "orr_physics3d::PhysicsState".into(),
            path: "config.max_linear_speed".into(),
            value: Value::Fixed(fp!(500)),
        },
        Op::SetSingletonField {
            singleton: "TerrainStepStatus".into(),
            path: "error_len".into(),
            value: Value::Int(1),
        },
    ];
    for shape in [
        Value::Variant(
            "box".into(),
            vec![("half_extents".into(), Value::Vec3(FPVec3::splat(FP::HALF)))],
        ),
        Value::Variant(
            "capsule".into(),
            vec![
                ("half_length".into(), Value::Fixed(FP::HALF)),
                ("radius".into(), Value::Fixed(FP::HALF)),
            ],
        ),
    ] {
        operations.push(Op::SetField {
            guid: guid(),
            component: COLLIDER.into(),
            path: "shape".into(),
            value: shape,
        });
    }
    for op in operations {
        let before = snapshot(&doc);
        assert!(doc.apply(op, Origin::User).is_err());
        assert_eq!(snapshot(&doc), before);
    }
    // Filter exclusion cannot hide an unsupported proxy from this game policy.
    doc.apply(
        Op::SetField {
            guid: guid(),
            component: COLLIDER.into(),
            path: "mask".into(),
            value: Value::Int(0),
        },
        Origin::User,
    )
    .unwrap();
    let before = snapshot(&doc);
    assert!(doc
        .apply(
            Op::SetField {
                guid: guid(),
                component: COLLIDER.into(),
                path: "shape".into(),
                value: Value::Variant(
                    "box".into(),
                    vec![("half_extents".into(), Value::Vec3(FPVec3::splat(FP::HALF)))]
                )
            },
            Origin::User
        )
        .is_err());
    assert_eq!(snapshot(&doc), before);
    // A valid, freshly cooked different revision is still a stale pin.
    let mut changed = orr_terrain::TerrainDocument::new(orr_terrain::Terrain::load(ASSET).unwrap());
    changed
        .apply(&[orr_terrain::Edit::SetHeight {
            x: 0,
            z: 0,
            height: FP::ONE,
        }])
        .unwrap();
    fs::write(&fixture.asset, changed.terrain().cook()).unwrap();
    assert!(doc.rebake().is_err());
    assert_eq!(snapshot(&doc), before);
    // A malformed file cannot publish even a partial host.
    fs::write(&fixture.asset, b"broken").unwrap();
    let mut cfg = ServerConfig::new(Auth::DevNoAuth);
    cfg.listen = false;
    assert!(spawn_terrain_yard3d_scene_host(fixture.scene.clone(), cfg).is_err());
}

#[test]
fn play_restore_replay_seek_and_branch_use_frame_owned_content_after_source_deletion() {
    let fixture = Fixture::new();
    let doc = fixture.doc();
    let doc_before = snapshot(&doc);
    fs::remove_file(&fixture.asset).unwrap(); // Start play from an already admitted frame.
    let mut play =
        PlayController::<TerrainYard3D>::start_play(&doc, doc.play_config(2, 60)).unwrap();
    assert!(!play.allow_play_edits());
    assert!(play.capture_scene().is_err());
    assert!(play
        .set_field(&Target::Guid(guid()), BODY, "pos.y", Value::Fixed(fp!(6)))
        .is_err());
    play.control(ControlOp::Step(90));
    let checkpoint = play.session().frame().to_bytes();
    let restored = Frame::from_bytes(doc.frame_registry().clone(), &checkpoint).unwrap();
    assert_eq!(restored.to_bytes(), checkpoint);
    let mut cold = Simulation::<TerrainYard3D>::from_frame(&restored, 60, 0).unwrap();
    for tick in 91..=150 {
        cold.step(&TickInputs::new(tick, 2));
    }
    play.control(ControlOp::Step(60));
    let final_bytes = play.session().frame().to_bytes();
    assert_eq!(final_bytes, cold.frame().to_bytes());
    assert_eq!(
        play.session()
            .frame()
            .singleton::<TerrainStepStatus>()
            .failed,
        0
    );
    assert!((y(&doc, play.session().frame(), "e_00000001") - FP::HALF).abs() < fp!(0.1));
    assert!(
        y(&doc, play.session().frame(), "e_00000002") < fp!(-2),
        "hole has no hidden floor"
    );
    play.control(ControlOp::Seek(90));
    assert_eq!(play.session().frame().to_bytes(), checkpoint);
    play.control(ControlOp::Branch);
    play.control(ControlOp::Step(60));
    assert_eq!(play.session().frame().to_bytes(), final_bytes);
    let stopped = play.stop_play();
    let mut replay = PlaySession::<TerrainYard3D>::open_replay(&stopped.replay, (), 0).unwrap();
    replay.control(ControlOp::Step(150));
    assert_eq!(replay.frame().to_bytes(), final_bytes);
    assert_eq!(snapshot(&doc), doc_before);
    assert_eq!(
        terrain_from_view(orr_bridge::FrameView::of(replay.frame()))
            .unwrap()
            .cook(),
        ASSET
    );
}

#[test]
fn raw_erp_debug_and_path_changes_cannot_bypass_terrain_admission() {
    let fixture = Fixture::new();
    let mut cfg = ServerConfig::new(Auth::DevNoAuth);
    cfg.listen = false;
    cfg.limits.allow_scene_paths = true;
    let host = spawn_terrain_yard3d_scene_host(fixture.scene.clone(), cfg).unwrap();
    let mut client = ErpClient::with_transport(Box::new(
        host.connector().connect("user", Caps::ALL).unwrap(),
    ));
    assert_eq!(
        client.call("rpc.discover", json!({})).unwrap()["engine"]["game"],
        "TerrainYard3D"
    );
    assert!(client
        .call(
            "scene.save",
            json!({"write":true,"path":fixture.root.join("other.scene.yaml").display().to_string()})
        )
        .is_err());
    assert!(client.call("scene.load", json!({"text": YAML, "path": fixture.root.join("other.scene.yaml").display().to_string()})).is_err());
    client.call("sim.start", json!({})).unwrap();
    assert!(client
        .call("scene.save", json!({"source": "play"}))
        .is_err());
    assert!(client
        .call(
            "world.singleton.patch",
            json!({"name": PIN_NAME, "path": "friction", "value": 1})
        )
        .is_err());
    let before = client.call("sim.state", json!({})).unwrap();
    let err = client
        .call("sim.debug", json!({"cmd":"despawn","entity":"0v0"}))
        .unwrap_err();
    assert!(
        matches!(err, orr_remote::ClientError::Rpc(error) if error.kind() == Some("play_edit_refused"))
    );
    assert!(client
        .call(
            "world.patch",
            json!({"entity":"e_00000001","component":BODY,"path":"pos.y","value":6})
        )
        .is_err());
    assert_eq!(
        client.call("sim.state", json!({})).unwrap()["checksum"],
        before["checksum"]
    );
    client.call("sim.step", json!({"n":3})).unwrap();
    client.call("sim.stop", json!({})).unwrap();
    client.call("scene.save", json!({"write":true})).unwrap();
    assert_eq!(fs::read_to_string(&fixture.scene).unwrap(), YAML);
}

#[cfg(unix)]
#[test]
fn source_symlinks_and_traversal_are_rejected_before_publication() {
    let fixture = Fixture::new();
    let mut doc = fixture.doc();
    let before = snapshot(&doc);
    let terrain = orr_terrain::Terrain::load(ASSET).unwrap();
    for source in [
        "../escape.orrt",
        "/tmp/escape.orrt",
        ".orr/private.orrt",
        "terrain/../sphere_demo.orrt",
    ] {
        let pin = TerrainScenePin::new(source, &terrain).unwrap();
        let value = doc
            .types()
            .get(PIN_NAME)
            .unwrap()
            .read(bytemuck::bytes_of(&pin));
        assert!(doc
            .apply(
                Op::SetSingletonField {
                    singleton: PIN_NAME.into(),
                    path: String::new(),
                    value
                },
                Origin::User
            )
            .is_err());
        assert_eq!(snapshot(&doc), before);
    }
    fs::rename(&fixture.asset, fixture.root.join("kept.orrt")).unwrap();
    std::os::unix::fs::symlink(fixture.root.join("kept.orrt"), &fixture.asset).unwrap();
    assert!(doc.rebake().is_err());
    assert_eq!(snapshot(&doc), before);
}
