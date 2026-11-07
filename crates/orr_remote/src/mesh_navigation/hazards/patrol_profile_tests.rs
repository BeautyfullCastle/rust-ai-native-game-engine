use super::*;
use bytemuck::Zeroable;
use orr_edit::{Op, Origin};
use orr_fp::{FP, fp};
use orr_games::mesh_navigation_game::{
    MESH_SET_GOAL,
    patrol::{MeshPatrol, MeshPatrolRun, PATROL_NAME},
};
use orr_navigation::{AgentProfile, mesh::StaticMeshGraph};
use orr_sim::{PlayerSlot, TickInputs};
fn document() -> EditorDoc {
    let graph = StaticMeshGraph::bake(
        &[
            [FP::ZERO; 3],
            [fp!(4), FP::ZERO, FP::ZERO],
            [FP::ZERO, FP::ZERO, fp!(4)],
            [fp!(4), FP::ZERO, fp!(4)],
        ],
        &[[0, 2, 3], [0, 3, 1]],
        AgentProfile::default(),
    )
    .unwrap();
    let doc = crate::mesh_navigation::authoring::create_mesh_scene(
        &graph,
        crate::mesh_navigation::AgentSpec {
            start: [fp!(0.5), FP::ZERO, fp!(0.5)],
            goal: [fp!(3.5), FP::ZERO, fp!(3.5)],
            max_vertical: FP::ZERO,
            distance_per_tick: fp!(0.25),
        },
    )
    .unwrap();
    hazards_doc_from_yaml(&doc.to_yaml()).unwrap()
}
fn config(doc: &EditorDoc) -> MeshPatrol {
    let pin = *doc.frame().singleton::<MeshScenePin>();
    let mut c = MeshPatrol::zeroed();
    c.enabled = 1;
    c.count = 2;
    c.looped = 1;
    c.wait_ticks = 2;
    c.graph_revision = pin.graph_revision;
    c.points[0] = [fp!(1.5), FP::ZERO, fp!(0.5)];
    c.points[1] = [fp!(3.5), FP::ZERO, fp!(3.5)];
    c
}
fn apply(doc: &mut EditorDoc, c: &MeshPatrol) -> Result<(), EditError> {
    let value = doc
        .types()
        .get(PATROL_NAME)
        .unwrap()
        .read(bytemuck::bytes_of(c));
    doc.apply(
        Op::SetSingletonField {
            singleton: PATROL_NAME.into(),
            path: String::new(),
            value,
        },
        Origin::User,
    )
    .map(|_| ())
}
#[test]
fn enabled_patrol_admits_first_goal_and_survives_authored_yaml_reopen() {
    let mut doc = document();
    let c = config(&doc);
    let original_goal = doc.frame().singleton::<MeshScenePin>().goal;
    assert_ne!(original_goal, c.points[0]);
    apply(&mut doc, &c).unwrap();
    assert_eq!(*doc.frame().singleton::<MeshPatrol>(), c);
    assert_eq!(doc.frame().singleton::<MeshScenePin>().goal, c.points[0]);
    hazards::validate_frame(doc.frame(), *doc.frame().singleton::<MeshScenePin>()).unwrap();
    let yaml = doc.to_yaml();
    let reopened = hazards_doc_from_yaml(&yaml).unwrap();
    assert_eq!(doc.frame().to_bytes(), reopened.frame().to_bytes());
    assert_eq!(
        *reopened.frame().singleton::<MeshPatrolRun>(),
        MeshPatrolRun::zeroed()
    );
    assert_eq!(MESH_HAZARDS_BUILD_ID, 0x4d48_415a_0000_0003);
}
#[test]
fn invalid_later_waypoint_revision_and_tail_are_atomic_document_rejections() {
    let mut doc = document();
    let good = config(&doc);
    apply(&mut doc, &good).unwrap();
    let before = doc.frame().to_bytes();
    let yaml = doc.to_yaml();
    for fault in 0..5 {
        let mut c = good;
        match fault {
            0 => c.points[1] = [fp!(80), FP::ZERO, FP::ZERO],
            1 => c.graph_revision[0] ^= 1,
            2 => c.points[7][0] = FP::ONE,
            3 => c.wait_ticks = 3601,
            _ => c.looped = 2,
        }
        assert!(apply(&mut doc, &c).is_err(), "fault {fault}");
        assert_eq!(doc.frame().to_bytes(), before);
        assert_eq!(doc.to_yaml(), yaml);
    }
}
#[test]
fn direct_admission_does_not_publish_derived_patrol_on_late_hazard_failure() {
    let doc = document();
    let c = config(&doc);
    let mut frame = Frame::new(Simulation::<MeshHazardNavigation>::build_registry());
    frame.set_singleton(*doc.frame().singleton::<MeshScenePin>());
    frame.set_singleton(c);
    let mut h = MeshHazards::zeroed();
    h.enabled = 1;
    h.count = 1;
    h.graph_revision = c.graph_revision;
    h.spheres[0] = HazardSphere {
        center: c.points[0],
        radius: FP::ZERO,
    };
    frame.set_singleton(h);
    let before = frame.to_bytes();
    let error = MeshHazardsAdmission
        .admit(&Scene::default(), &mut frame, &SceneIndex::default())
        .unwrap_err();
    assert!(error.to_string().contains("hazard"), "{error}");
    assert_eq!(frame.to_bytes(), before);
}
#[test]
fn runtime_patrol_padding_and_hazard_loss_cannot_be_authored() {
    let doc = document();
    let c = config(&doc);
    for fault in 0..3 {
        let mut frame = Frame::new(Simulation::<MeshHazardNavigation>::build_registry());
        frame.set_singleton(*doc.frame().singleton::<MeshScenePin>());
        frame.set_singleton(c);
        match fault {
            0 => {
                let mut r = MeshPatrolRun::zeroed();
                r.reserved[2] = 1;
                frame.set_singleton(r);
            }
            1 => {
                let mut r = MeshPatrolRun::zeroed();
                r.cursor = 1;
                frame.set_singleton(r);
            }
            _ => {
                frame.set_singleton(MeshHazardRun {
                    lost: 1,
                    hit_index: 0,
                    lost_tick: 1,
                });
            }
        }
        let before = frame.to_bytes();
        assert!(
            MeshHazardsAdmission
                .admit(&Scene::default(), &mut frame, &SceneIndex::default())
                .is_err(),
            "fault {fault}"
        );
        assert_eq!(frame.to_bytes(), before);
    }
}
#[test]
fn disabled_patrol_preserves_hazard_manual_movement_and_zero_progress() {
    let doc = document();
    let mut sim = Simulation::<MeshHazardNavigation>::from_frame(
        doc.frame(),
        YARD_TICK_RATE,
        MESH_HAZARDS_BUILD_ID,
    )
    .unwrap();
    let mut inputs = TickInputs::new(1, 1);
    let goal = [fp!(3), FP::ZERO, fp!(1)];
    inputs.set_input(
        PlayerSlot(0),
        orr_games::yard3d_game::YardInput {
            buttons: MESH_SET_GOAL,
            origin: goal.map(|v| v.raw() as i32),
            ..Default::default()
        },
    );
    sim.step(&inputs);
    assert_eq!(
        *sim.frame().singleton::<MeshPatrolRun>(),
        MeshPatrolRun::zeroed()
    );
    assert!(sim.frame().singleton::<MeshPatrol>().disabled());
    assert_eq!(sim.frame().singleton::<MeshScenePin>().goal, goal);
    let pin = *sim.frame().singleton::<MeshScenePin>();
    let agent =
        orr_navigation_runtime::mesh::MeshRuntime::agent(sim.frame(), pin.binding()).unwrap();
    assert_ne!(
        agent.position,
        doc.frame().singleton::<MeshScenePin>().start
    );
    assert!(agent.distance > FP::ZERO);
    assert_eq!(
        sim.frame()
            .singleton::<crate::mesh_navigation::MeshStepStatus>()
            .failed,
        0
    );
}

#[test]
fn terminal_loss_view_preserves_arrived_cursor_but_rejects_forged_stopped_state() {
    let mut doc = document();
    let pin = *doc.frame().singleton::<MeshScenePin>();
    let mut c = config(&doc);
    c.points[0] = pin.start;
    c.points[1] = [fp!(2.5), FP::ZERO, fp!(0.5)];
    c.wait_ticks = 0;
    apply(&mut doc, &c).unwrap();
    let mut h = MeshHazards::zeroed();
    h.enabled = 1;
    h.count = 1;
    h.graph_revision = c.graph_revision;
    h.spheres[0] = HazardSphere {
        // The admitted two-triangle corridor bends toward [2,0,2].
        // Its first quarter-unit sweep crosses this point, not the goal's X axis.
        center: [fp!(0.625), FP::ZERO, fp!(0.625)],
        radius: fp!(0.1),
    };
    let value = doc
        .types()
        .get(HAZARDS_NAME)
        .unwrap()
        .read(bytemuck::bytes_of(&h));
    doc.apply(
        Op::SetSingletonField {
            singleton: HAZARDS_NAME.into(),
            path: String::new(),
            value,
        },
        Origin::User,
    )
    .unwrap();
    let mut sim = Simulation::<MeshHazardNavigation>::from_frame(
        doc.frame(),
        YARD_TICK_RATE,
        MESH_HAZARDS_BUILD_ID,
    )
    .unwrap();
    sim.step(&TickInputs::new(1, 1));
    let before = *sim.frame().singleton::<MeshPatrolRun>();
    assert_eq!(before.arrived, 1);
    sim.step(&TickInputs::new(2, 1));
    assert_eq!(
        sim.frame().singleton::<MeshHazardRun>().lost,
        1,
        "step={:?}, patrol={:?}, mesh={:?}, hazard={:?}",
        sim.frame()
            .singleton::<crate::mesh_navigation::MeshStepStatus>(),
        sim.frame().singleton::<MeshPatrolRun>(),
        mesh_from_view(FrameView::of(sim.frame())).unwrap().agent,
        sim.frame().singleton::<MeshHazards>()
    );
    assert_eq!(*sim.frame().singleton::<MeshPatrolRun>(), before);
    let observed = crate::mesh_navigation::patrol::from_view(FrameView::of(sim.frame()))
        .unwrap()
        .unwrap();
    assert_eq!(observed.run, before);
    assert!(from_view(FrameView::of(sim.frame())).unwrap().is_some());
    let mut forged = sim.frame().clone();
    forged.set_singleton(MeshHazardRun::zeroed());
    assert!(crate::mesh_navigation::patrol::from_view(FrameView::of(&forged)).is_err());
    let mut future = sim.frame().clone();
    let mut run = *future.singleton::<MeshHazardRun>();
    run.lost_tick = future.tick() + 1;
    future.set_singleton(run);
    assert!(crate::mesh_navigation::patrol::from_view(FrameView::of(&future)).is_err());
}
