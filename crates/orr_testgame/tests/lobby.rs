//! Avatars from prefabs: `PlayerAdded` makes the avatar of a player with `orr_ecs::spawn_prefab`.
use orr_gfx::{Transform3, ViewRef};
use orr_sim::{PlayerLink, PlayerSlot, RawPlayerSignal, Simulation, TickInputs};
use orr_testgame::lobby::{Lobby, LobbyConfig, LobbyInput, Profile, STEP};

type Inputs = TickInputs<LobbyInput, ()>;

const BLUE: &str = "avatars/blue.orrview";
const RED: &str = "avatars/red.orrview";

/// Two avatar prefabs: a transform, a view reference and a link to no player yet.
fn config() -> LobbyConfig {
    LobbyConfig::from_view_assets(&[BLUE, RED])
}

fn tick(n: u64, signals: Vec<RawPlayerSignal>, input: LobbyInput) -> Inputs {
    let mut inputs = Inputs::new(n, 2);
    inputs.set_input(PlayerSlot(0), input);
    for signal in signals {
        inputs.push_signal(signal);
    }
    inputs
}

/// Every avatar as (player, asset, x, z), in player order.
fn avatars(sim: &Simulation<Lobby>) -> Vec<(u8, u64, i64, i64)> {
    let frame = sim.frame();
    let mut list: Vec<_> = frame
        .entities()
        .filter_map(|e| {
            let link = frame.get::<PlayerLink>(e)?;
            let placed = frame.get::<Transform3>(e)?;
            Some((link.player.0, frame.get::<ViewRef>(e)?.asset, placed.position.x.raw(), placed.position.z.raw()))
        })
        .collect();
    list.sort();
    list
}

#[test]
fn each_added_player_gets_the_avatar_prefab_of_their_slot_with_their_link_and_place() {
    let mut sim = Simulation::<Lobby>::new(config(), 60, 1);
    sim.step(&tick(1, vec![], LobbyInput::default()));
    assert!(avatars(&sim).is_empty(), "no avatar before a signal");

    sim.step(&tick(2, vec![RawPlayerSignal::added(PlayerSlot(0)), RawPlayerSignal::added(PlayerSlot(1))], LobbyInput::default()));

    let x = |n: i32| (orr_fp::fp!(3) * orr_fp::FP::from_int(n) - orr_fp::fp!(1.5)).raw();
    assert_eq!(avatars(&sim), vec![(0, ViewRef::at(BLUE).asset, x(0), 0), (1, ViewRef::at(RED).asset, x(1), 0)]);
}

#[test]
fn the_start_data_can_choose_the_prefab_and_a_player_has_one_avatar() {
    let mut sim = Simulation::<Lobby>::new(config(), 60, 1);
    // Choice 2 is the second prefab (red), whatever the slot is.
    sim.step(&tick(1, vec![RawPlayerSignal::added_with(PlayerSlot(0), &Profile { choice: 2, reserved: 0 })], LobbyInput::default()));
    assert_eq!(avatars(&sim)[0].1, ViewRef::at(RED).asset);
    // The same player added again does not make a second avatar.
    sim.step(&tick(2, vec![RawPlayerSignal::added(PlayerSlot(0))], LobbyInput::default()));
    assert_eq!(avatars(&sim).len(), 1);
    // A slot past the last prefab starts again from the first.
    sim.step(&tick(3, vec![RawPlayerSignal::added(PlayerSlot(2))], LobbyInput::default()));
    assert_eq!(avatars(&sim)[1].1, ViewRef::at(BLUE).asset);
}

#[test]
fn the_avatar_walks_with_the_input_of_its_player_and_is_deleted_when_the_player_is_removed() {
    let mut sim = Simulation::<Lobby>::new(config(), 60, 1);
    sim.step(&tick(1, vec![RawPlayerSignal::added(PlayerSlot(0))], LobbyInput::default()));
    let start = avatars(&sim)[0];

    sim.step(&tick(2, vec![], LobbyInput { dx: 1, dz: -3 }));

    let moved = avatars(&sim)[0];
    assert_eq!((moved.2 - start.2, moved.3 - start.3), (STEP.raw(), -STEP.raw()), "an input of any size is one step");
    sim.step(&tick(3, vec![RawPlayerSignal::removed(PlayerSlot(0))], LobbyInput::default()));
    assert!(avatars(&sim).is_empty());
}

#[test]
fn a_game_without_prefabs_ignores_added() {
    let mut sim = Simulation::<Lobby>::new(LobbyConfig::default(), 60, 1);
    sim.step(&tick(1, vec![RawPlayerSignal::added(PlayerSlot(0))], LobbyInput::default()));
    assert!(avatars(&sim).is_empty());
}

#[test]
fn two_runs_of_the_same_signals_and_inputs_have_the_same_checksum_every_tick() {
    let script = |n: u64| match n {
        2 => tick(n, vec![RawPlayerSignal::added(PlayerSlot(1)), RawPlayerSignal::added(PlayerSlot(0))], LobbyInput { dx: 1, dz: 0 }),
        5 => tick(n, vec![RawPlayerSignal::removed(PlayerSlot(1))], LobbyInput { dx: 0, dz: 1 }),
        7 => tick(n, vec![RawPlayerSignal::added(PlayerSlot(1))], LobbyInput { dx: -1, dz: 1 }),
        _ => tick(n, vec![], LobbyInput { dx: 1, dz: 1 }),
    };
    let (mut one, mut two) = (Simulation::<Lobby>::new(config(), 60, 7), Simulation::<Lobby>::new(config(), 60, 7));
    for n in 1..=12 {
        one.step(&script(n));
        two.step(&script(n));
        assert_eq!(one.frame().checksum(), two.frame().checksum(), "tick {n}");
    }
    assert_eq!(avatars(&one).len(), 2);
    assert_ne!(Simulation::<Lobby>::new(config(), 60, 7).frame().checksum(), Simulation::<Lobby>::new(LobbyConfig::default(), 60, 7).frame().checksum(), "the prefabs are part of the state");
}
