use super::*;
use crate::mesh_navigation_game::patrol::{self, MeshPatrol, MeshPatrolRun};
use orr_navigation::{mesh::StaticMeshGraph, AgentProfile};
use orr_navigation_runtime::mesh::AgentSpec;
use orr_sim::{PlayerSlot, Simulation, TickInputs};
fn p(x: i32, z: i32) -> [FP; 3] {
    [FP::from_int(x), FP::ZERO, FP::from_int(z)]
}
fn simulation(wait: u32, hazard: Option<[FP; 3]>) -> Simulation<MeshHazardNavigation> {
    let graph = StaticMeshGraph::bake(
        &[p(0, 0), p(0, 8), p(8, 0), p(8, 8)],
        &[[0, 1, 2], [1, 3, 2]],
        AgentProfile::default(),
    )
    .unwrap();
    let spec = AgentSpec {
        start: p(0, 0),
        goal: p(8, 0),
        max_vertical: FP::ZERO,
        distance_per_tick: FP::ONE,
    };
    let mut pin = MeshScenePin::new(&graph, spec).unwrap();
    let mut s = Simulation::<MeshHazardNavigation>::new((), 60, 91);
    s.frame_mut().set_singleton(pin);
    MeshRuntime::admit(s.frame_mut(), &graph, pin.binding(), spec).unwrap();
    let mut c = MeshPatrol {
        enabled: 1,
        count: 3,
        looped: 1,
        wait_ticks: wait,
        graph_revision: graph.revision(),
        ..MeshPatrol::zeroed()
    };
    c.points[0] = p(1, 0);
    c.points[1] = p(3, 0);
    c.points[2] = p(5, 0);
    s.frame_mut().set_singleton(c);
    patrol::admit(s.frame_mut(), &mut pin).unwrap();
    if let Some(center) = hazard {
        let mut h = MeshHazards {
            enabled: 1,
            count: 1,
            graph_revision: graph.revision(),
            ..MeshHazards::zeroed()
        };
        h.spheres[0] = HazardSphere {
            center,
            radius: FP::from_raw(FP::SCALE / 4),
        };
        s.frame_mut().set_singleton(h);
    }
    admit(s.frame(), pin).unwrap();
    s
}
fn tick(s: &mut Simulation<MeshHazardNavigation>, goal: Option<[FP; 3]>) {
    let mut input = TickInputs::new(s.tick() + 1, 1);
    if let Some(goal) = goal {
        input.set_input(
            PlayerSlot(0),
            YardInput {
                buttons: crate::mesh_navigation_game::MESH_SET_GOAL,
                origin: goal.map(|v| v.raw() as i32),
                ..YardInput::zeroed()
            },
        );
    }
    s.step(&input);
    assert_eq!(s.frame().singleton::<MeshStepStatus>().failed, 0);
}
fn position(s: &Simulation<MeshHazardNavigation>) -> [FP; 3] {
    let pin = *s.frame().singleton::<MeshScenePin>();
    MeshRuntime::agent(s.frame(), pin.binding())
        .unwrap()
        .position
}
#[test]
fn patrol_waits_exact_ticks_in_hazard_profile_then_advances() {
    let mut s = simulation(2, None);
    tick(&mut s, None);
    assert_eq!(position(&s), p(1, 0));
    for wait in [1, 0] {
        tick(&mut s, None);
        assert_eq!(position(&s), p(1, 0));
        assert_eq!(patrol::read_run(s.frame()).unwrap().wait_remaining, wait);
    }
    tick(&mut s, None);
    assert_eq!(position(&s), p(2, 0));
    assert_eq!(patrol::read_run(s.frame()).unwrap().cursor, 1);
}
#[test]
fn collision_discards_waypoint_replan_and_keeps_arrived_wait_cursor_goal() {
    let mut s = simulation(1, Some(p(2, 0)));
    tick(&mut s, None);
    tick(&mut s, None);
    let run = patrol::read_run(s.frame()).unwrap();
    let pin = *s.frame().singleton::<MeshScenePin>();
    assert_eq!(run.arrived, 1);
    assert_eq!(run.wait_remaining, 0);
    tick(&mut s, None);
    assert_eq!(read_run(s.frame()).unwrap().lost, 1);
    assert_eq!(position(&s), p(1, 0));
    assert_eq!(patrol::read_run(s.frame()).unwrap(), run);
    assert_eq!(*s.frame().singleton::<MeshScenePin>(), pin);
    validate_frame(s.frame(), pin).unwrap();
    for _ in 0..4 {
        tick(&mut s, Some(p(8, 0)));
        assert_eq!(patrol::read_run(s.frame()).unwrap(), run);
        assert_eq!(position(&s), p(1, 0));
    }
}
#[test]
fn collision_also_rolls_back_manual_pause_and_new_goal() {
    let mut s = simulation(2, Some(p(2, 0)));
    tick(&mut s, None);
    let run = patrol::read_run(s.frame()).unwrap();
    let pin = *s.frame().singleton::<MeshScenePin>();
    tick(&mut s, Some(p(8, 0)));
    assert_eq!(read_run(s.frame()).unwrap().lost, 1);
    assert_eq!(patrol::read_run(s.frame()).unwrap(), run);
    assert_eq!(*s.frame().singleton::<MeshScenePin>(), pin);
}
#[test]
fn successful_manual_goal_pauses_and_does_not_resume_patrol() {
    let mut s = simulation(2, None);
    tick(&mut s, None);
    tick(&mut s, Some(p(8, 0)));
    assert_eq!(patrol::read_run(s.frame()).unwrap().paused, 1);
    for _ in 0..12 {
        tick(&mut s, None);
    }
    assert_eq!(position(&s), p(8, 0));
    assert_eq!(patrol::read_run(s.frame()).unwrap().paused, 1);
}
#[test]
fn win_freezes_patrol_and_snapshot_restore_replays_identically() {
    let mut a = simulation(0, None);
    let pin = *a.frame().singleton::<MeshScenePin>();
    let mut c = mission::MeshMission {
        enabled: 1,
        count: 1,
        graph_revision: pin.graph_revision,
        ..mission::MeshMission::zeroed()
    };
    c.points[0] = mission::MissionPoint {
        position: p(1, 0),
        radius: FP::from_raw(FP::SCALE / 4),
    };
    c.exit = c.points[0];
    a.frame_mut().set_singleton(c);
    let initial = a.frame().clone();
    tick(&mut a, None);
    assert_eq!(mission::read_run(a.frame()).unwrap().won, 1);
    let winning_checksum = a.checksum();
    let run = patrol::read_run(a.frame()).unwrap();
    let pos = position(&a);
    tick(&mut a, Some(p(8, 0)));
    assert_eq!(position(&a), pos);
    assert_eq!(patrol::read_run(a.frame()).unwrap(), run);
    a.restore(&initial);
    tick(&mut a, None);
    assert_eq!(mission::read_run(a.frame()).unwrap().won, 1);
    assert_eq!(patrol::read_run(a.frame()).unwrap(), run);
    assert_eq!(a.checksum(), winning_checksum);
}
#[test]
fn arrived_stopped_without_qualified_loss_is_rejected() {
    let mut s = simulation(2, Some(p(7, 7)));
    tick(&mut s, None);
    let pin = *s.frame().singleton::<MeshScenePin>();
    MeshRuntime::stop(s.frame_mut(), pin.binding()).unwrap();
    assert!(patrol::validate_frame(s.frame(), pin).is_err());
    assert!(validate_frame(s.frame(), pin).is_err());
    let tick = s.tick();
    s.frame_mut().set_singleton(MeshHazardRun {
        lost: 1,
        hit_index: 0,
        lost_tick: tick + 1,
    });
    assert!(patrol::validate_frame(s.frame(), pin).is_err());
    s.frame_mut().set_singleton(MeshHazardRun {
        lost: 1,
        hit_index: 0,
        lost_tick: tick,
    });
    s.frame_mut().set_singleton(mission::MeshMissionRun {
        collected: 0,
        won: 1,
    });
    assert!(patrol::validate_frame(s.frame(), pin).is_err());
    assert_eq!(patrol::read_run(s.frame()).unwrap().arrived, 1);
}

#[test]
fn invalid_patrol_state_cannot_move_or_change_hazard_outcome() {
    let mut s = simulation(0, None);
    let before = position(&s);
    let mut r = patrol::read_run(s.frame()).unwrap();
    r.reserved[0] = 1;
    s.frame_mut().set_singleton(r);
    let input = TickInputs::new(s.tick() + 1, 1);
    s.step(&input);
    assert_eq!(position(&s), before);
    assert_eq!(*s.frame().singleton::<MeshPatrolRun>(), r);
    assert_eq!(read_run(s.frame()).unwrap(), MeshHazardRun::zeroed());
    assert_eq!(s.frame().singleton::<MeshStepStatus>().failed, 1);
}

#[test]
fn hazard_profile_runs_a_full_three_waypoint_loop() {
    let mut s = simulation(0, None);
    for (x, cursor) in [
        (1, 0),
        (2, 1),
        (3, 1),
        (4, 2),
        (5, 2),
        (4, 0),
        (3, 0),
        (2, 0),
        (1, 0),
    ] {
        tick(&mut s, None);
        assert_eq!(position(&s), p(x, 0));
        assert_eq!(patrol::read_run(s.frame()).unwrap().cursor, cursor);
    }
    assert_eq!(patrol::read_run(s.frame()).unwrap().completed, 0);
}
#[test]
fn hazard_collision_precedes_mission_collection_at_attempted_patrol_waypoint() {
    let mut s = simulation(0, Some(p(1, 0)));
    let pin = *s.frame().singleton::<MeshScenePin>();
    let mut c = mission::MeshMission {
        enabled: 1,
        count: 1,
        graph_revision: pin.graph_revision,
        ..mission::MeshMission::zeroed()
    };
    c.points[0] = mission::MissionPoint {
        position: p(1, 0),
        radius: FP::from_raw(FP::SCALE / 4),
    };
    c.exit = c.points[0];
    s.frame_mut().set_singleton(c);
    let old_mission = mission::read_run(s.frame()).unwrap();
    let old_patrol = patrol::read_run(s.frame()).unwrap();
    tick(&mut s, None);
    assert_eq!(read_run(s.frame()).unwrap().lost, 1);
    assert_eq!(position(&s), p(0, 0));
    assert_eq!(mission::read_run(s.frame()).unwrap(), old_mission);
    assert_eq!(patrol::read_run(s.frame()).unwrap(), old_patrol);
}
