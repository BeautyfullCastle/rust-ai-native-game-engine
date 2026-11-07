use super::*;
use bytemuck::Zeroable;
use orr_edit::{Op, Origin};
use orr_fp::{fp, FP};
use orr_games::mesh_navigation_game::{
    patrol::{MeshPatrol, MeshPatrolRun, PATROL_NAME},
    MESH_SET_GOAL,
};
use orr_navigation::{mesh::StaticMeshGraph, AgentProfile};
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
#[test]
fn enabled_patrol_is_rejected_atomically_in_hazard_profile() {
    let mut doc = document();
    let pin = *doc.frame().singleton::<MeshScenePin>();
    let mut config = MeshPatrol::zeroed();
    config.enabled = 1;
    config.count = 1;
    config.graph_revision = pin.graph_revision;
    config.points[0] = pin.goal;
    let value = doc
        .types()
        .get(PATROL_NAME)
        .unwrap()
        .read(bytemuck::bytes_of(&config));
    let before = doc.frame().to_bytes();
    let yaml = doc.to_yaml();
    let error = doc
        .apply(
            Op::SetSingletonField {
                singleton: PATROL_NAME.into(),
                path: String::new(),
                value,
            },
            Origin::User,
        )
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("does not support automatic waypoint patrol"),
        "{error}"
    );
    assert_eq!(doc.frame().to_bytes(), before);
    assert_eq!(doc.to_yaml(), yaml);
    assert_eq!(MESH_HAZARDS_BUILD_ID, 0x4d48_415a_0000_0002);
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
    let agent = orr_navigation_runtime::mesh::MeshRuntime::agent(sim.frame(), pin.binding()).unwrap();
    assert_ne!(agent.position, doc.frame().singleton::<MeshScenePin>().start);
    assert!(agent.distance > FP::ZERO);
    assert_eq!(
        sim.frame()
            .singleton::<crate::mesh_navigation::MeshStepStatus>()
            .failed,
        0
    );
}
