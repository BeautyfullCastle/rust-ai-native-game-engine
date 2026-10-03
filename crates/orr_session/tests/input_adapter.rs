//! Opt-in local input derivation records commands once, without changing raw
//! bridge/replay behavior or the Arena simulation's existing golden values.
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use orr_fp::FP;
use orr_session::{ControlOp, PlayConfig, PlaySession, ReplayReader};
use orr_sim::PlayerSlot;
use orr_testgame::{Arena, ArenaConfig, ArenaInput, SpawnBulletCmd, FIRE};

fn derive(slot: PlayerSlot, input: &ArenaInput) -> Vec<SpawnBulletCmd> {
    if input.buttons & FIRE != 0 {
        vec![SpawnBulletCmd {
            owner: u32::from(slot.0),
        }]
    } else {
        vec![]
    }
}

#[test]
fn held_inputs_derive_each_live_tick_and_replay_and_seek_do_not() {
    let mut cfg = PlayConfig::new(2, 42, 60);
    cfg.ring_capacity = 2;
    cfg.keyframe_interval = 0;
    let mut play = PlaySession::<Arena>::new(cfg, ArenaConfig { player_count: 2 });
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    play.set_commands_from_input(move |slot, input| {
        count.fetch_add(1, Ordering::Relaxed);
        derive(slot, input)
    });
    let fire = ArenaInput::new(FP::ZERO, FP::ZERO, true);
    play.set_input(PlayerSlot(0), fire);
    play.set_input(PlayerSlot(1), fire);
    play.control(ControlOp::Step(4));
    assert_eq!(calls.load(Ordering::Relaxed), 8);
    play.set_input(PlayerSlot(0), ArenaInput::default());
    play.set_input(PlayerSlot(1), ArenaInput::default());
    play.control(ControlOp::Step(2));
    let checksums: Vec<_> = (0..=6)
        .map(|tick| play.checksum_at(tick).unwrap())
        .collect();
    let bytes = play.save_replay();
    let replay = ReplayReader::<Arena>::parse(&bytes).unwrap();
    for tick in 1..=4 {
        assert_eq!(
            replay.tick(tick).unwrap().1,
            vec![
                (PlayerSlot(0), SpawnBulletCmd { owner: 0 }),
                (PlayerSlot(1), SpawnBulletCmd { owner: 1 })
            ]
        );
    }
    assert!(replay.tick(5).unwrap().1.is_empty());
    play.seek(0).unwrap();
    play.seek(6).unwrap();
    assert_eq!(play.frame().checksum(), checksums[6]);
    assert_eq!(calls.load(Ordering::Relaxed), 12);
    let mut viewer =
        PlaySession::<Arena>::open_replay(&bytes, ArenaConfig { player_count: 2 }, 0).unwrap();
    viewer.set_commands_from_input(|_, _| panic!("recorded commands must never be derived again"));
    // Both setters are read-only in Viewer, including their derivation mode.
    viewer.set_input(PlayerSlot(0), fire);
    viewer.set_input_without_commands(PlayerSlot(1), fire);
    for checksum in checksums.iter().skip(1) {
        viewer.control(ControlOp::Step(1));
        assert_eq!(&viewer.frame().checksum(), checksum);
    }
}

#[test]
fn identical_raw_and_structured_values_switch_modes_without_duplicate_commands() {
    let mut play =
        PlaySession::<Arena>::new(PlayConfig::new(2, 42, 60), ArenaConfig { player_count: 2 });
    play.set_commands_from_input(derive);
    let fire = ArenaInput::new(FP::ZERO, FP::ZERO, true);
    play.set_input(PlayerSlot(0), fire);
    play.control(ControlOp::Step(1));
    // Raw bridge supplies the same bytes AND its already-derived command.
    play.set_input_without_commands(PlayerSlot(0), fire);
    play.push_command(PlayerSlot(0), SpawnBulletCmd { owner: 0 })
        .unwrap();
    play.control(ControlOp::Step(2));
    // The same bytes through the structured path re-enable held derivation.
    play.set_input(PlayerSlot(0), fire);
    play.push_command(PlayerSlot(1), SpawnBulletCmd { owner: 1 })
        .unwrap();
    play.control(ControlOp::Step(1));
    let bytes = play.save_replay();
    let replay = ReplayReader::<Arena>::parse(&bytes).unwrap();
    assert_eq!(replay.tick(1).unwrap().1.len(), 1);
    assert_eq!(replay.tick(2).unwrap().1.len(), 1);
    assert!(replay.tick(3).unwrap().1.is_empty());
    // Explicit order stays first, followed by derived slots in ascending order.
    assert_eq!(
        replay.tick(4).unwrap().1,
        vec![
            (PlayerSlot(1), SpawnBulletCmd { owner: 1 }),
            (PlayerSlot(0), SpawnBulletCmd { owner: 0 })
        ]
    );
}

#[test]
fn callback_is_opt_in_and_viewer_rejected_input_does_not_leak_into_branch() {
    let mut play =
        PlaySession::<Arena>::new(PlayConfig::new(2, 42, 60), ArenaConfig { player_count: 2 });
    let fire = ArenaInput::new(FP::ZERO, FP::ZERO, true);
    play.set_input(PlayerSlot(0), fire);
    play.control(ControlOp::Step(1));
    let bytes = play.save_replay();
    assert!(ReplayReader::<Arena>::parse(&bytes)
        .unwrap()
        .tick(1)
        .unwrap()
        .1
        .is_empty());
    let mut viewer =
        PlaySession::<Arena>::open_replay(&bytes, ArenaConfig { player_count: 2 }, 0).unwrap();
    viewer.set_commands_from_input(derive);
    viewer.set_input(PlayerSlot(0), fire);
    viewer.set_input_without_commands(PlayerSlot(1), fire);
    viewer.control(ControlOp::Branch);
    viewer.control(ControlOp::Step(1));
    let branched = viewer.save_replay();
    assert!(ReplayReader::<Arena>::parse(&branched)
        .unwrap()
        .tick(1)
        .unwrap()
        .1
        .is_empty());
}
