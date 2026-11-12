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
fn run() -> game::DefenseRun {
    game::DefenseRun {
        phase: game::PLAYING,
        ..Default::default()
    }
}
fn release() -> DefenseEvent {
    DefenseEvent {
        kind: Kind::Release as u32,
        unit_id: 1,
        enemy_id: 2,
        projectile_id: 3,
        role: game::RIVET,
        ..Default::default()
    }
}
fn emit(v: &mut DefenseVfx, tick: u64, status: EventStatus<DefenseEvent>) {
    v.consume_items([(EventKey::new(tick, 0, 0), status)])
        .unwrap();
}

#[test]
fn only_release_by_the_exact_live_actor_recoils() {
    let mut v = DefenseVfx::default();
    let shot = release();
    emit(
        &mut v,
        1,
        EventStatus::Verified(DefenseEvent {
            kind: Kind::Hit as u32,
            ..shot
        }),
    );
    v.advance_to(1);
    assert_eq!(v.unit_recoil(&unit(), &run()), 0.0);
    emit(&mut v, 2, EventStatus::Verified(shot));
    v.advance_to(2);
    assert_eq!(v.unit_recoil(&unit(), &run()), 6.0);
    for other in [
        game::DefenseUnit { id: 2, ..unit() },
        game::DefenseUnit { id: 0, ..unit() },
        game::DefenseUnit { board: 1, ..unit() },
        game::DefenseUnit {
            role: game::MORTAR,
            ..unit()
        },
        game::DefenseUnit {
            active: 0,
            ..unit()
        },
    ] {
        assert_eq!(v.unit_recoil(&other, &run()), 0.0);
    }
    assert_eq!(
        v.unit_recoil(
            &unit(),
            &game::DefenseRun {
                generation: 1,
                ..run()
            }
        ),
        0.0
    );
}

#[test]
fn session_ticks_recover_and_duplicate_verification_does_not_restart() {
    let mut v = DefenseVfx::default();
    emit(&mut v, 10, EventStatus::Predicted(release()));
    v.advance_to(9);
    assert_eq!(v.unit_recoil(&unit(), &run()), 0.0); // Future event is not anticipation.
    v.advance_to(10);
    assert_eq!(v.unit_recoil(&unit(), &run()), 6.0);
    v.advance_to(14);
    emit(&mut v, 10, EventStatus::Verified(release()));
    for _ in 0..3 {
        assert_eq!(v.unit_recoil(&unit(), &run()), 3.0); // Repaints do not advance.
    }
    emit(
        &mut v,
        15,
        EventStatus::Verified(DefenseEvent {
            projectile_id: 4,
            ..release()
        }),
    );
    v.advance_to(15);
    assert_eq!(v.unit_recoil(&unit(), &run()), 6.0); // A genuine new shot starts again.
    v.advance_to(23);
    assert_eq!(v.unit_recoil(&unit(), &run()), 0.0);
}

#[test]
fn cancellation_and_corrected_payload_remove_wrong_actor_motion() {
    let mut v = DefenseVfx::default();
    emit(&mut v, 1, EventStatus::Predicted(release()));
    v.advance_to(2);
    assert!(v.unit_recoil(&unit(), &run()) > 0.0);
    emit(&mut v, 1, EventStatus::Canceled);
    assert_eq!(v.unit_recoil(&unit(), &run()), 0.0);
    emit(
        &mut v,
        1,
        EventStatus::Predicted(DefenseEvent {
            unit_id: 2,
            ..release()
        }),
    );
    assert_eq!(v.unit_recoil(&unit(), &run()), 0.0);
    assert!(v.unit_recoil(&game::DefenseUnit { id: 2, ..unit() }, &run()) > 0.0);
}

#[test]
fn restart_replacement_and_terminal_frames_cannot_replay_old_shots() {
    let mut v = DefenseVfx::default();
    emit(&mut v, 1, EventStatus::Verified(release()));
    v.advance_to(1);
    for phase in [game::WON, game::LOST] {
        assert_eq!(
            v.unit_recoil(&unit(), &game::DefenseRun { phase, ..run() }),
            0.0
        );
    }
    emit(
        &mut v,
        2,
        EventStatus::Verified(DefenseEvent {
            kind: Kind::Restart as u32,
            generation: 1,
            ..Default::default()
        }),
    );
    v.advance_to(2);
    let restarted = game::DefenseRun {
        generation: 1,
        ..run()
    };
    assert_eq!(v.unit_recoil(&unit(), &restarted), 0.0);
    let shot = DefenseEvent {
        generation: 1,
        ..release()
    };
    emit(&mut v, 3, EventStatus::Verified(shot));
    v.advance_to(3);
    assert_eq!(v.unit_recoil(&unit(), &restarted), 6.0);
    v.replace(4, 1);
    emit(&mut v, 3, EventStatus::Verified(shot));
    assert_eq!(v.unit_recoil(&unit(), &restarted), 0.0);
}

#[test]
fn moving_actor_retains_motion_but_reused_cell_does_not_inherit_it() {
    let mut v = DefenseVfx::default();
    emit(&mut v, 1, EventStatus::Verified(release()));
    v.advance_to(1);
    assert_eq!(
        v.unit_recoil(&game::DefenseUnit { cell: 8, ..unit() }, &run()),
        6.0
    );
    assert_eq!(
        v.unit_recoil(&game::DefenseUnit { id: 7, ..unit() }, &run()),
        0.0
    );
}
