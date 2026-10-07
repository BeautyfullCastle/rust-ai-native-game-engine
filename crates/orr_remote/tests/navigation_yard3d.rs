//! Headless admission, atomic editing, and filesystem-free rollback proof.
#![cfg(feature = "navigation")]
#![allow(clippy::disallowed_types)]
use orr_bridge::FrameView;
use orr_ecs::Frame;
use orr_edit::{EditorDoc, Op, Origin, PlayController};
use orr_fp::{fp, FP};
use orr_navigation_runtime::{NavigationRuntime, RuntimeState};
use orr_reflect::Value;
use orr_remote::navigation_yard3d::*;
use orr_session::{ControlOp, PlaySession};
use orr_sim::{Simulation, TickInputs};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

const YAML: &str = include_str!("../../../scenes/navigation_point.scene.yaml");
const BLANK: &str = include_str!("../../../scenes/navigation_blank.scene.yaml");
const ASSET: &[u8] = include_bytes!("../../../scenes/navigation/point_demo.orrt");

struct Fixture {
    root: PathBuf,
    scene: PathBuf,
    asset: PathBuf,
}
impl Fixture {
    fn new(yaml: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!(
                "orr_navigation_host_{}_{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        fs::create_dir_all(root.join("navigation")).unwrap();
        let scene = root.join("point.scene.yaml");
        let asset = root.join("navigation/point_demo.orrt");
        fs::write(&scene, yaml).unwrap();
        fs::write(&asset, ASSET).unwrap();
        Self { root, scene, asset }
    }
    fn doc(&self) -> EditorDoc {
        navigation_yard3d_doc_from_path(&self.scene).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
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

#[test]
fn fixture_has_one_derived_agent_complete_route_and_reopens_identically() {
    let fixture = Fixture::new(YAML);
    let mut doc = fixture.doc();
    assert!(doc.scene().entities.is_empty());
    assert_eq!(doc.frame().alive_count(), 1);
    let view = navigation_from_view(FrameView::of(doc.frame())).unwrap();
    assert_eq!(view.terrain.cook(), ASSET);
    assert_eq!(
        view.graph.revision(),
        doc.frame().singleton::<NavigationScenePin>().graph_revision
    );
    assert_eq!(view.navigator.path().unwrap().corridor().len(), 2);
    NavigationRuntime::validate(doc.frame()).unwrap();
    let saved = doc.save_yaml();
    let reopened = navigation_yard3d_doc_from_yaml(
        &saved,
        LocalNavigationAdmission::for_scene(&fixture.scene).unwrap(),
    )
    .unwrap();
    assert_eq!(doc.frame().to_bytes(), reopened.frame().to_bytes());
}

#[test]
fn stale_pin_unsafe_path_and_authored_entities_are_atomic() {
    let fixture = Fixture::new(YAML);
    let mut doc = fixture.doc();
    let before = snapshot(&doc);
    for (field, value) in [
        ("terrain_revision[0]", Value::Int(0)),
        ("graph_revision[0]", Value::Int(0)),
        ("asset_id[0]", Value::Int(88)),
        ("agent.max_slope", Value::Fixed(fp!(2))),
        ("agent.distance_per_tick", Value::Fixed(FP::ZERO)),
    ] {
        assert!(doc
            .apply(
                Op::SetSingletonField {
                    singleton: PIN_NAME.into(),
                    path: field.into(),
                    value,
                },
                Origin::User
            )
            .is_err());
        assert_eq!(snapshot(&doc), before);
    }
    let terrain = orr_terrain::Terrain::load(ASSET).unwrap();
    let graph =
        orr_navigation::TerrainGraph::build(&terrain, orr_navigation::AgentProfile::default())
            .unwrap();
    let agent = doc.frame().singleton::<NavigationScenePin>().agent;
    for source in [
        "../escape.orrt",
        "/tmp/escape.orrt",
        ".orr/private.orrt",
        "navigation/../point_demo.orrt",
    ] {
        let pin = NavigationScenePin::new(source, &terrain, &graph, agent).unwrap();
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
                    value,
                },
                Origin::User
            )
            .is_err());
        assert_eq!(snapshot(&doc), before);
    }
    let authored = YAML.replace(
        "entities: {}",
        r#"entities:
  e_00000001:
    name: unsupported_box
    orr_physics3d::Body:
      pos: [0, 1, 0]
      rot: { kind: unit, x: 0, y: 0, z: 0, w: 1 }
      vel: [0, 0, 0]
      omega: [0, 0, 0]
      inv_mass: 1
      inv_inertia: [10, 10, 10]
      linear_damping: 0
      angular_damping: 0
      kind: dynamic
    orr_physics3d::Collider:
      shape: { kind: box, half_extents: [0.5, 0.5, 0.5] }
      restitution: 0
      friction: 0.6
      layer: 1
      mask: 4294967295
"#,
    );
    orr_reflect::Scene::parse(&authored, doc.types()).unwrap();
    assert!(doc.load_yaml(&authored).is_err());
    assert_eq!(snapshot(&doc), before);
    let changed = orr_terrain::Terrain::new(
        terrain.asset_id().into(),
        2,
        2,
        [FP::ZERO; 2],
        FP::ONE,
        vec![FP::ONE, FP::ZERO, FP::ZERO, FP::ZERO],
        vec![false],
    )
    .unwrap();
    fs::write(&fixture.asset, changed.cook()).unwrap();
    assert!(doc.rebake().is_err());
    assert_eq!(snapshot(&doc), before);
    fs::write(&fixture.asset, b"malformed terrain").unwrap();
    assert!(doc.rebake().is_err());
    assert_eq!(snapshot(&doc), before);
    fs::write(&fixture.asset, vec![0; orr_terrain::MAX_FILE_BYTES + 1]).unwrap();
    assert!(doc.rebake().is_err());
    assert_eq!(snapshot(&doc), before);
    fs::remove_file(&fixture.asset).unwrap();
    assert!(doc.rebake().is_err());
    assert_eq!(snapshot(&doc), before);
}

#[test]
fn deleted_source_cannot_change_play_seek_restore_or_replay() {
    let fixture = Fixture::new(YAML);
    let doc = fixture.doc();
    let initial = snapshot(&doc);
    fs::remove_file(&fixture.asset).unwrap();
    let mut play =
        PlayController::<NavigationYard3D>::start_play(&doc, doc.play_config(2, 60)).unwrap();
    assert!(!play.allow_play_edits());
    play.control(ControlOp::Step(2));
    let checkpoint = play.session().frame().to_bytes();
    let restored = Frame::from_bytes(doc.frame_registry().clone(), &checkpoint).unwrap();
    let mut cold = Simulation::<NavigationYard3D>::from_frame(&restored, 60, 0).unwrap();
    for tick in 3..=12 {
        cold.step(&TickInputs::new(tick, 2));
    }
    play.control(ControlOp::Step(10));
    let final_bytes = play.session().frame().to_bytes();
    assert_eq!(cold.frame().to_bytes(), final_bytes);
    assert_eq!(
        play.session()
            .frame()
            .singleton::<NavigationStepStatus>()
            .failed,
        0
    );
    play.control(ControlOp::Seek(2));
    assert_eq!(play.session().frame().to_bytes(), checkpoint);
    play.control(ControlOp::Branch);
    play.control(ControlOp::Step(10));
    assert_eq!(play.session().frame().to_bytes(), final_bytes);
    let stopped = play.stop_play();
    let mut replay = PlaySession::<NavigationYard3D>::open_replay(&stopped.replay, (), 0).unwrap();
    replay.control(ControlOp::Step(12));
    assert_eq!(replay.frame().to_bytes(), final_bytes);
    assert_eq!(
        terrain_from_view(FrameView::of(replay.frame()))
            .unwrap()
            .cook(),
        ASSET
    );
    assert_eq!(snapshot(&doc), initial);
}

#[test]
fn blank_canvas_is_editable_but_has_no_silent_navigation_fallback() {
    let fixture = Fixture::new(BLANK);
    let doc = fixture.doc();
    assert_eq!(doc.frame().alive_count(), 0);
    assert!(!doc.frame().singleton::<RuntimeState>().is_active());
    assert!(navigation_from_view(FrameView::of(doc.frame())).is_err());
    let mut sim = Simulation::<NavigationYard3D>::from_frame(doc.frame(), 60, 0).unwrap();
    sim.step(&TickInputs::new(1, 2));
    assert_eq!(sim.frame().singleton::<NavigationStepStatus>().failed, 1);
    assert_eq!(
        sim.frame().singleton::<NavigationStepStatus>().failed_tick,
        1
    );
    assert_eq!(sim.frame().alive_count(), 0);
}

#[test]
fn corrupted_frame_handles_fail_closed_without_panicking_or_moving() {
    let fixture = Fixture::new(YAML);
    let doc = fixture.doc();
    for alias in [false, true] {
        let mut frame =
            Frame::from_bytes(doc.frame_registry().clone(), &doc.frame().to_bytes()).unwrap();
        let mut state = *frame.singleton::<RuntimeState>();
        if alias {
            state.navigator_bytes = state.terrain_bytes;
        } else {
            let stale = frame.alloc_list::<u8>();
            frame.list_free(stale);
            state.navigator_bytes = stale;
        }
        frame.set_singleton(state);
        let before = frame.to_bytes();
        assert!(NavigationRuntime::validate(&frame).is_err());
        assert!(navigation_from_view(FrameView::of(&frame)).is_err());
        let mut sim = Simulation::<NavigationYard3D>::from_frame(&frame, 60, 0).unwrap();
        sim.step(&TickInputs::new(1, 2));
        let status = sim.frame().singleton::<NavigationStepStatus>();
        assert_eq!(status.failed, 1);
        assert_eq!(status.failed_tick, 1);
        assert!(!status.message().is_empty());
        let halted = sim.frame().to_bytes();
        sim.step(&TickInputs::new(2, 2));
        assert_eq!(
            sim.frame().singleton::<NavigationStepStatus>().failed_tick,
            1
        );
        assert_eq!(sim.frame().singleton::<RuntimeState>(), &state);
        assert_ne!(before, halted);
    }
}

#[cfg(unix)]
#[test]
fn internal_source_symlink_is_rejected_without_mutating_document() {
    let fixture = Fixture::new(YAML);
    let mut doc = fixture.doc();
    let before = snapshot(&doc);
    let kept = fixture.root.join("kept.orrt");
    fs::rename(&fixture.asset, &kept).unwrap();
    std::os::unix::fs::symlink(&kept, &fixture.asset).unwrap();
    assert!(doc.rebake().is_err());
    assert_eq!(snapshot(&doc), before);
    assert!(navigation_yard3d_doc_from_path(&fixture.scene).is_err());
}
