use super::*;
use crate::navigation_yard3d::*;
use bytemuck::Zeroable;
use orr_ecs::Frame;
use orr_fp::{FP, fp};
use orr_games::navigation_yard3d_game::patrol as core_patrol;
use orr_navigation::{AgentProfile, TerrainGraph};
use orr_navigation_runtime::NavigationRuntime;
use orr_sim::{Simulation, TickInputs};
const ASSET: &[u8] = include_bytes!("../../../../../scenes/navigation/point_demo.orrt");
fn admitted() -> Frame {
    let terrain = orr_terrain::Terrain::load(ASSET).unwrap();
    let spec = NavigationAgentSpec {
        start: [fp!(0.25), fp!(0.75)],
        goal: [fp!(0.75), fp!(0.25)],
        max_slope: FP::ONE,
        distance_per_tick: fp!(0.125),
    };
    let graph = TerrainGraph::build(
        &terrain,
        AgentProfile {
            max_slope: FP::ONE,
            radius: FP::ZERO,
            headroom: FP::ZERO,
            max_step: FP::ZERO,
        },
    )
    .unwrap();
    let pin =
        NavigationScenePin::new("navigation/point_demo.orrt", &terrain, &graph, spec).unwrap();
    let mut frame = Frame::new(Simulation::<NavigationYard3D>::build_registry());
    frame.set_singleton(pin);
    NavigationRuntime::admit(&mut frame, &terrain, &graph, spec.to_runtime()).unwrap();
    frame
}
fn config(frame: &Frame) -> TerrainPatrol {
    let pin = *frame.singleton::<NavigationScenePin>();
    let mut c = TerrainPatrol::zeroed();
    c.enabled = 1;
    c.count = 2;
    c.looped = 1;
    c.wait_ticks = 1;
    c.graph_revision = pin.graph_revision;
    c.points[0] = [fp!(0.75), fp!(0.25)];
    c.points[1] = [fp!(0.25), fp!(0.75)];
    c
}
#[test]
fn reflected_patch_validates_all_legs_without_mutating_frame_and_run_is_not_reflected() {
    let frame = admitted();
    let view = navigation_from_view(FrameView::of(&frame)).unwrap();
    let c = config(&frame);
    let before = frame.to_bytes();
    let patch = patch_request(&c, &view).unwrap();
    assert_eq!(patch["name"], PATROL_NAME);
    assert_eq!(patch["value"]["count"], 2);
    assert_eq!(frame.to_bytes(), before);
    let types = navigation_yard3d_types();
    assert!(types.get(PATROL_NAME).is_some());
    assert!(types.get(RUN_NAME).is_none());
    for fault in 0..4 {
        let mut bad = c;
        match fault {
            0 => bad.points[1] = [fp!(100), FP::ZERO],
            1 => bad.graph_revision[0] ^= 1,
            2 => bad.points[7][1] = FP::ONE,
            _ => bad.wait_ticks = 3601,
        };
        assert!(patch_request(&bad, &view).is_err());
        assert_eq!(frame.to_bytes(), before);
    }
}
#[test]
fn immutable_views_follow_runtime_cursor_and_reject_forged_leg_progress() {
    let mut frame = admitted();
    let c = config(&frame);
    let pin = *frame.singleton::<NavigationScenePin>();
    frame.set_singleton(c);
    core_patrol::admit(&mut frame, pin).unwrap();
    let mut sim =
        Simulation::<NavigationYard3D>::from_frame(&frame, YARD_TICK_RATE, NAVIGATION_BUILD_ID)
            .unwrap();
    let mut advanced = false;
    for tick in 1..=24 {
        sim.step(&TickInputs::new(tick, 1));
        let before = sim.frame().to_bytes();
        let view = navigation_from_view(FrameView::of(sim.frame())).unwrap();
        let p = from_view(FrameView::of(sim.frame())).unwrap().unwrap();
        assert_eq!(view.pin, pin);
        advanced |= p.run.cursor == 1;
        assert_eq!(sim.frame().to_bytes(), before);
    }
    assert!(advanced);
    let mut forged = sim.frame().clone();
    let mut run = *forged.singleton::<TerrainPatrolRun>();
    run.reserved[2] = 1;
    forged.set_singleton(run);
    assert!(from_view(FrameView::of(&forged)).is_err());
}
#[cfg(target_os = "linux")]
mod filesystem {
    use super::*;
    use std::fs;
    struct Temp {
        root: std::path::PathBuf,
    }
    impl Temp {
        fn path(&self) -> &std::path::Path {
            &self.root
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
    const YAML: &str = include_str!("../../../../../scenes/navigation_point.scene.yaml");
    fn fixture() -> (Temp, std::path::PathBuf) {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "orr-terrain-patrol-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let temp = Temp { root };
        fs::create_dir(temp.path().join("navigation")).unwrap();
        fs::write(temp.path().join("navigation/point_demo.orrt"), ASSET).unwrap();
        let scene = temp.path().join("point.scene.yaml");
        fs::write(&scene, YAML).unwrap();
        (temp, scene)
    }
    #[test]
    fn atomic_patrol_edit_undo_redo_save_reopen_and_source_hidden_play() {
        let (temp, scene) = fixture();
        let mut doc = navigation_yard3d_doc_from_path(&scene).unwrap();
        let c = config(doc.frame());
        let original = doc.frame().to_bytes();
        set_patrol(&mut doc, &c).unwrap();
        let authored = doc.frame().to_bytes();
        assert_ne!(authored, original);
        doc.undo().unwrap();
        assert_eq!(doc.frame().to_bytes(), original);
        doc.redo().unwrap();
        assert_eq!(doc.frame().to_bytes(), authored);
        fs::write(&scene, doc.save_yaml()).unwrap();
        let reopened = navigation_yard3d_doc_from_path(&scene).unwrap();
        assert_eq!(reopened.frame().to_bytes(), authored);
        fs::rename(
            temp.path().join("navigation/point_demo.orrt"),
            temp.path().join("hidden-terrain"),
        )
        .unwrap();
        assert!(
            navigation_yard3d_doc_from_path(&scene).is_err(),
            "new bakes still require original source"
        );
        let mut a = Simulation::<NavigationYard3D>::from_frame(
            reopened.frame(),
            YARD_TICK_RATE,
            NAVIGATION_BUILD_ID,
        )
        .unwrap();
        let mut b = Simulation::<NavigationYard3D>::from_frame(
            reopened.frame(),
            YARD_TICK_RATE,
            NAVIGATION_BUILD_ID,
        )
        .unwrap();
        for tick in 1..=24 {
            let input = TickInputs::new(tick, 1);
            a.step(&input);
            b.step(&input);
            assert_eq!(a.frame().checksum(), b.frame().checksum());
            from_view(FrameView::of(a.frame())).unwrap();
        }
    }
    #[test]
    fn invalid_later_segment_preserves_document_yaml_frame_history_and_file() {
        let (_temp, scene) = fixture();
        let mut doc = navigation_yard3d_doc_from_path(&scene).unwrap();
        let c = config(doc.frame());
        set_patrol(&mut doc, &c).unwrap();
        let yaml = doc.to_yaml();
        let bytes = doc.frame().to_bytes();
        let revision = doc.revision();
        let source = fs::read(&scene).unwrap();
        let mut bad = c;
        bad.points[1] = [fp!(100), FP::ZERO];
        assert!(set_patrol(&mut doc, &bad).is_err());
        assert_eq!(doc.to_yaml(), yaml);
        assert_eq!(doc.frame().to_bytes(), bytes);
        assert_eq!(doc.revision(), revision);
        assert_eq!(fs::read(scene).unwrap(), source);
    }
    #[test]
    fn authored_runtime_padding_and_late_patrol_failure_leave_raw_frame_unchanged() {
        let (_temp, scene) = fixture();
        let doc = navigation_yard3d_doc_from_path(&scene).unwrap();
        let pin = *doc.frame().singleton::<NavigationScenePin>();
        let admission = LocalNavigationAdmission::for_scene(&scene).unwrap();
        for fault in 0..2 {
            let mut raw = Frame::new(Simulation::<NavigationYard3D>::build_registry());
            raw.set_singleton(pin);
            let mut c = config(doc.frame());
            if fault == 0 {
                c.points[1] = [fp!(100), FP::ZERO];
            } else {
                let mut r = TerrainPatrolRun::zeroed();
                r.reserved[1] = 1;
                raw.set_singleton(r);
            }
            raw.set_singleton(c);
            let before = raw.to_bytes();
            assert!(
                orr_edit::BakeAdmission::admit(&admission, doc.scene(), &mut raw, doc.index())
                    .is_err()
            );
            assert_eq!(raw.to_bytes(), before);
        }
    }
}
