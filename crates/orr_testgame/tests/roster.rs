//! Player signals at the simulation level: `Players::on_player` gets each
//! signal in the tick it belongs to, with typed start data, in a fixed order.
use orr_sim::{PlayerSlot, RawPlayerSignal, Simulation, TickInputs};
use orr_testgame::roster::{Avatar, Census, Profile, Roster, RosterInput};
use orr_testgame::{Arena, ArenaConfig, ArenaInput, SpawnBulletCmd};

type Inputs = TickInputs<RosterInput, ()>;

fn tick(n: u64, signals: Vec<RawPlayerSignal>) -> Inputs {
    let mut inputs = Inputs::new(n, 2);
    for signal in signals {
        inputs.push_signal(signal);
    }
    inputs
}

fn avatars(sim: &Simulation<Roster>) -> Vec<Avatar> {
    let mut list = sim.frame().dense::<Avatar>().1.to_vec();
    list.sort_by_key(|a| a.slot);
    list
}

fn census(sim: &Simulation<Roster>) -> Census {
    *sim.frame().singleton::<Census>()
}

#[test]
fn each_signal_reaches_on_player_in_its_own_tick() {
    let mut sim = Simulation::<Roster>::new((), 60, 1);
    let slot0 = PlayerSlot(0);
    let slot1 = PlayerSlot(1);

    sim.step(&tick(1, vec![]));
    assert!(avatars(&sim).is_empty(), "no avatar exists before a signal");

    sim.step(&tick(2, vec![RawPlayerSignal::added_with(slot0, &Profile { color: 7, _pad: 0 })]));
    assert_eq!(avatars(&sim), vec![Avatar { slot: 0, x: 0, color: 7, away: 0 }]);

    sim.step(&tick(3, vec![RawPlayerSignal::added(slot1)]));
    assert_eq!(avatars(&sim).len(), 2);
    assert_eq!(avatars(&sim)[1].color, 0, "no data means the default start data");

    sim.step(&tick(4, vec![RawPlayerSignal::disconnected(slot0)]));
    assert_eq!(avatars(&sim)[0].away, 1);

    sim.step(&tick(5, vec![RawPlayerSignal::reconnected(slot0)]));
    assert_eq!(avatars(&sim)[0].away, 0);

    sim.step(&tick(6, vec![RawPlayerSignal::removed(slot1)]));
    assert_eq!(avatars(&sim).len(), 1);
    assert_eq!(avatars(&sim)[0].slot, 0);

    assert_eq!(census(&sim), Census { added: 2, disconnected: 1, reconnected: 1, removed: 1 });
}

#[test]
fn start_data_of_the_wrong_size_is_the_default_value() {
    let mut sim = Simulation::<Roster>::new((), 60, 1);
    let odd = RawPlayerSignal { kind: orr_sim::SignalKind::Added, slot: PlayerSlot(0), data: vec![9, 9, 9] };
    sim.step(&tick(1, vec![odd]));
    assert_eq!(avatars(&sim)[0].color, 0);
}

#[test]
fn signals_of_one_tick_apply_in_slot_order_whatever_the_insertion_order() {
    let run = |signals: Vec<RawPlayerSignal>| {
        let mut sim = Simulation::<Roster>::new((), 60, 1);
        sim.step(&tick(1, signals));
        (sim.checksum(), avatars(&sim))
    };
    let a = RawPlayerSignal::added_with(PlayerSlot(0), &Profile { color: 1, _pad: 0 });
    let b = RawPlayerSignal::added_with(PlayerSlot(1), &Profile { color: 2, _pad: 0 });
    let forward = run(vec![a.clone(), b.clone()]);
    let backward = run(vec![b, a]);
    assert_eq!(forward, backward);
}

#[test]
fn signals_of_one_slot_keep_the_order_they_were_given() {
    let slot = PlayerSlot(0);
    let setup = |sim: &mut Simulation<Roster>| sim.step(&tick(1, vec![RawPlayerSignal::added(slot)]));

    let mut sim = Simulation::<Roster>::new((), 60, 1);
    setup(&mut sim);
    sim.step(&tick(2, vec![RawPlayerSignal::disconnected(slot), RawPlayerSignal::reconnected(slot)]));
    assert_eq!(avatars(&sim)[0].away, 0, "disconnected then reconnected: present");

    let mut sim = Simulation::<Roster>::new((), 60, 1);
    setup(&mut sim);
    sim.step(&tick(2, vec![RawPlayerSignal::reconnected(slot), RawPlayerSignal::disconnected(slot)]));
    assert_eq!(avatars(&sim)[0].away, 1, "reconnected then disconnected: away");
}

#[test]
fn a_game_without_players_ignores_signals() {
    let run = |with_signals: bool| {
        let mut sim = Simulation::<Arena>::new(ArenaConfig { player_count: 2 }, 60, 3);
        for n in 1..=5 {
            let mut inputs = TickInputs::<ArenaInput, SpawnBulletCmd>::new(n, 2);
            if with_signals {
                inputs.push_signal(RawPlayerSignal::added(PlayerSlot(0)));
                inputs.push_signal(RawPlayerSignal::removed(PlayerSlot(1)));
            }
            sim.step(&inputs);
        }
        sim.checksum()
    };
    assert_eq!(run(false), run(true));
}
