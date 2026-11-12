use super::*;
use orr_games::coop_defense_game as game;

fn unit() -> game::DefenseUnit {
    game::DefenseUnit {
        active: 1,
        id: 1,
        role: game::RIVET,
        ..Default::default()
    }
}
fn shot() -> DefenseEvent {
    DefenseEvent {
        kind: Kind::Release as u32,
        unit_id: 1,
        enemy_id: 7,
        projectile_id: 3,
        role: game::RIVET,
        x: 200,
        ..Default::default()
    }
}
fn emit(v: &mut DefenseVfx, tick: u64, status: EventStatus<DefenseEvent>) {
    v.consume_items([(EventKey::new(tick, 0, 0), status)])
        .unwrap();
}
fn frame(tick: u64, generation: u32, enemy_id: u32, active: u32, progress: u32) -> orr_ecs::Frame {
    let mut simulation = orr_sim::Simulation::<game::DefenseGame>::new(Default::default(), 60, 1);
    let frame = simulation.frame_mut();
    frame.set_tick(tick);
    frame.singleton_mut::<game::DefenseRun>().generation = generation;
    frame.singleton_mut::<game::DefenseRun>().phase = game::PLAYING;
    let entity = frame.query::<(&game::DefenseEnemy,)>().next().unwrap().0;
    *frame.get_mut::<game::DefenseEnemy>(entity).unwrap() = game::DefenseEnemy {
        active,
        id: enemy_id,
        lane: 0,
        progress,
        ..Default::default()
    };
    frame.clone()
}

#[test]
fn attack_facing_locks_left_across_target_motion_loss_and_duplicate_verification() {
    let mut v = DefenseVfx::default();
    emit(&mut v, 10, EventStatus::Predicted(shot()));
    v.advance_to(10);
    let left = frame(10, 0, 7, 1, 0); // x50 < release origin x200.
    let before = left.to_bytes();
    v.resolve_attack_facing(&left);
    assert_eq!(
        v.unit_release_pose(&unit(), &game::run(&left)),
        Some((0, true))
    );
    assert_eq!(left.to_bytes(), before);
    let once = v.state.clone();
    v.resolve_attack_facing(&left);
    assert_eq!(v.state, once);
    emit(&mut v, 10, EventStatus::Verified(shot()));
    v.resolve_attack_facing(&frame(10, 0, 7, 1, 500)); // Target now on right.
    v.resolve_attack_facing(&frame(10, 0, 7, 0, 500)); // Then gone.
    assert_eq!(
        v.unit_release_pose(&unit(), &game::run(&left)),
        Some((0, true))
    );
}

#[test]
fn attack_facing_requires_coherent_clock_generation_and_exact_live_target() {
    let mut v = DefenseVfx::default();
    emit(&mut v, 10, EventStatus::Verified(shot()));
    v.advance_to(10);
    for untrusted in [
        frame(9, 0, 7, 1, 0),
        frame(10, 1, 7, 1, 0),
        frame(10, 0, 8, 1, 0),
        frame(10, 0, 7, 0, 0),
    ] {
        v.resolve_attack_facing(&untrusted);
        let run = game::run(&frame(10, 0, 7, 1, 0));
        assert_eq!(v.unit_release_pose(&unit(), &run), Some((0, false)));
        assert!(v
            .state
            .active
            .values()
            .next()
            .unwrap()
            .facing_left
            .is_none());
    }
    v.resolve_attack_facing(&frame(10, 0, 7, 1, 0));
    assert_eq!(
        v.unit_release_pose(&unit(), &game::run(&frame(10, 0, 7, 1, 0))),
        Some((0, true))
    );
}

#[test]
fn attack_facing_right_and_vertical_targets_keep_original_orientation() {
    for (progress, origin_x) in [(500, 200), (0, 50)] {
        let mut v = DefenseVfx::default();
        emit(
            &mut v,
            10,
            EventStatus::Verified(DefenseEvent {
                x: origin_x,
                ..shot()
            }),
        );
        v.advance_to(10);
        let frame = frame(10, 0, 7, 1, progress);
        v.resolve_attack_facing(&frame);
        assert_eq!(
            v.unit_release_pose(&unit(), &game::run(&frame)),
            Some((0, false))
        );
        assert_eq!(
            v.state.active.values().next().unwrap().facing_left,
            Some(false)
        );
    }
}

#[test]
fn attack_facing_cancel_and_corrected_target_start_from_new_orientation() {
    let mut v = DefenseVfx::default();
    emit(&mut v, 10, EventStatus::Predicted(shot()));
    v.advance_to(10);
    let left = frame(10, 0, 7, 1, 0);
    v.resolve_attack_facing(&left);
    emit(&mut v, 10, EventStatus::Canceled);
    assert_eq!(v.unit_release_pose(&unit(), &game::run(&left)), None);
    emit(
        &mut v,
        10,
        EventStatus::Predicted(DefenseEvent {
            enemy_id: 8,
            ..shot()
        }),
    );
    v.resolve_attack_facing(&frame(10, 0, 8, 1, 500));
    assert_eq!(
        v.unit_release_pose(&unit(), &game::run(&left)),
        Some((0, false))
    );
    assert_eq!(
        v.unit_release_pose(&game::DefenseUnit { id: 2, ..unit() }, &game::run(&left)),
        None
    );
}

#[test]
fn attack_facing_future_restart_and_save_replacement_never_inherit_old_direction() {
    let mut v = DefenseVfx::default();
    emit(&mut v, 10, EventStatus::Verified(shot()));
    v.advance_to(9);
    v.resolve_attack_facing(&frame(9, 0, 7, 1, 0));
    assert!(v
        .state
        .active
        .values()
        .next()
        .unwrap()
        .facing_left
        .is_none());
    v.advance_to(10);
    let frame0 = frame(10, 0, 7, 1, 0);
    v.resolve_attack_facing(&frame0);
    assert_eq!(
        v.unit_release_pose(&unit(), &game::run(&frame0)),
        Some((0, true))
    );
    emit(
        &mut v,
        11,
        EventStatus::Verified(DefenseEvent {
            kind: Kind::Restart as u32,
            generation: 1,
            ..Default::default()
        }),
    );
    emit(
        &mut v,
        12,
        EventStatus::Verified(DefenseEvent {
            generation: 1,
            ..shot()
        }),
    );
    v.advance_to(12);
    let frame1 = frame(12, 1, 7, 1, 500);
    v.resolve_attack_facing(&frame1);
    assert_eq!(
        v.unit_release_pose(&unit(), &game::run(&frame1)),
        Some((0, false))
    );
    v.replace(12, 1);
    assert_eq!(v.unit_release_pose(&unit(), &game::run(&frame1)), None);
}
