//! One client owning several slots (players that share one screen): the inputs
//! of every owned slot reach the sim at each tick, and other slots are refused.

use orr_bridge::{Bridge, BridgeConfig, BridgeError, InProc, Pacing, PlayConfig, PlayHost, PlaySession, PlayerSlot, Threaded, ThreadedConfig};
use orr_fp::FP;
use orr_testgame::{Arena, ArenaConfig, ArenaInput};

const PLAYERS: u8 = 3;
const TICKS: u32 = 24;

fn play_config() -> PlayConfig {
    let mut cfg = PlayConfig::new(PLAYERS, 11, 50);
    cfg.start_paused = false;
    cfg
}

fn session() -> PlaySession<Arena> {
    PlaySession::<Arena>::new(play_config(), ArenaConfig { player_count: PLAYERS })
}

fn host(extra: &[u8]) -> PlayHost<Arena> {
    PlayHost::new(session(), PlayerSlot(0)).with_extra_local_slots(extra.iter().map(|s| PlayerSlot(*s)))
}

fn input(x: i32, y: i32) -> ArenaInput {
    ArenaInput::new(FP::from_int(x), FP::from_int(y), false)
}

/// The checksum after `TICKS` ticks of a session that gets the inputs directly.
fn reference(inputs: &[(u8, ArenaInput)]) -> u64 {
    let mut session = session();
    for (slot, input) in inputs {
        session.set_input(PlayerSlot(*slot), *input);
    }
    for _ in 0..TICKS {
        session.step_now().expect("the session ticks");
    }
    session.frame().checksum()
}

fn checksum(bridge: &impl Bridge<Arena>) -> u64 {
    bridge.snapshot().expect("a snapshot").predicted().checksum()
}

#[test]
fn a_bridge_reports_every_slot_it_owns_with_the_first_one_first() {
    let single = InProc::new(host(&[]), BridgeConfig::default());
    assert_eq!(single.local_slots(), vec![PlayerSlot(0)]);

    // Slot 0 again, a slot past the player count and a repeat are dropped.
    let two = InProc::new(host(&[1, 0, 9, 1]), BridgeConfig::default());
    assert_eq!(two.local_slot(), PlayerSlot(0));
    assert_eq!(two.local_slots(), vec![PlayerSlot(0), PlayerSlot(1)]);
}

#[test]
fn inputs_of_both_local_slots_reach_the_sim_every_tick() {
    let mut bridge = InProc::new(host(&[1]), BridgeConfig::default());
    bridge.set_input(PlayerSlot(0), input(1, 0)).unwrap();
    bridge.set_input(PlayerSlot(1), input(0, -1)).unwrap();
    bridge.step(TICKS);

    let both = reference(&[(0, input(1, 0)), (1, input(0, -1))]);
    assert_eq!(checksum(&bridge), both);
    // Each slot matters: the same run without one of the inputs ends elsewhere.
    assert_ne!(both, reference(&[(0, input(1, 0))]));
    assert_ne!(both, reference(&[(1, input(0, -1))]));
}

#[test]
fn an_input_change_of_the_second_slot_applies_to_the_following_ticks() {
    let mut bridge = InProc::new(host(&[1]), BridgeConfig::default());
    bridge.set_input(PlayerSlot(1), input(1, 1)).unwrap();
    bridge.step(TICKS / 2);
    bridge.set_input(PlayerSlot(1), input(-1, 0)).unwrap();
    bridge.step(TICKS - TICKS / 2);

    let mut by_hand = session();
    by_hand.set_input(PlayerSlot(1), input(1, 1));
    for _ in 0..TICKS / 2 {
        by_hand.step_now().unwrap();
    }
    by_hand.set_input(PlayerSlot(1), input(-1, 0));
    for _ in 0..TICKS - TICKS / 2 {
        by_hand.step_now().unwrap();
    }
    assert_eq!(checksum(&bridge), by_hand.frame().checksum());
}

#[test]
fn a_slot_the_client_does_not_own_is_refused_and_changes_nothing() {
    let mut bridge = InProc::new(host(&[1]), BridgeConfig::default());
    let refused = bridge.set_input(PlayerSlot(2), input(1, 1));
    assert_eq!(refused, Err(BridgeError::NotLocalPlayer { player: PlayerSlot(2), local: PlayerSlot(0) }));
    assert_eq!(bridge.set_input(PlayerSlot::NONE, input(1, 1)), Err(BridgeError::NotLocalPlayer { player: PlayerSlot::NONE, local: PlayerSlot(0) }));
    bridge.step(TICKS);
    assert_eq!(checksum(&bridge), reference(&[]));

    // A single-slot bridge still refuses every other slot, slot 1 included.
    let mut single = InProc::new(host(&[]), BridgeConfig::default());
    assert!(matches!(single.set_input(PlayerSlot(1), input(1, 0)), Err(BridgeError::NotLocalPlayer { .. })));
}

#[test]
fn the_threaded_adapter_owns_the_same_slots_and_gives_the_same_state() {
    let mut threaded = Threaded::spawn(
        || host(&[1]),
        BridgeConfig::default(),
        ThreadedConfig { pacing: Pacing::Manual, max_catchup: 8 },
    )
    .unwrap();
    assert_eq!(threaded.local_slots(), vec![PlayerSlot(0), PlayerSlot(1)]);
    assert!(threaded.set_input(PlayerSlot(2), input(1, 0)).is_err());

    threaded.set_input(PlayerSlot(0), input(1, 0)).unwrap();
    threaded.set_input(PlayerSlot(1), input(0, -1)).unwrap();
    threaded.step(TICKS);
    assert_eq!(checksum(&threaded), reference(&[(0, input(1, 0)), (1, input(0, -1))]));
}
