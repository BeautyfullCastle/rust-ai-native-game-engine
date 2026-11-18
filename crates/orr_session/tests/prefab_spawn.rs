//! Spawning avatars from prefabs on `PlayerAdded` stays deterministic: a replay reaches the same
//! checksums, and a signal that arrives late (so the tick is resimulated after a rollback) gives
//! the same state as a signal that was known in time.
use orr_session::{replay_verify, AdvanceResult, LoopbackNetwork, PlayConfig, PlaySession, ReplayReader, Session, SessionConfig};
use orr_sim::{PlayerLink, PlayerSlot, RawPlayerSignal};
use orr_testgame::lobby::{Lobby, LobbyConfig, LobbyInput};

const BLUE: &str = "avatars/blue.orrview";
const RED: &str = "avatars/red.orrview";

fn config() -> LobbyConfig {
    LobbyConfig::from_view_assets(&[BLUE, RED])
}

fn input(slot: u8, tick: u64) -> LobbyInput {
    LobbyInput { dx: ((tick * 7 + u64::from(slot) * 3) % 3) as i32 - 1, dz: (tick % 2) as i32 }
}

fn links(frame: &orr_ecs::Frame) -> Vec<u8> {
    let mut list: Vec<u8> = frame.dense::<PlayerLink>().1.iter().map(|l| l.player.0).collect();
    list.sort_unstable();
    list
}

fn plan() -> Vec<(u64, RawPlayerSignal)> {
    vec![
        (1, RawPlayerSignal::added(PlayerSlot(0))),
        (1, RawPlayerSignal::added(PlayerSlot(1))),
        (9, RawPlayerSignal::removed(PlayerSlot(1))),
        (12, RawPlayerSignal::added(PlayerSlot(1))),
    ]
}

#[test]
fn a_replay_spawns_the_same_avatars_as_the_live_run() {
    let mut live = PlaySession::<Lobby>::new(PlayConfig::new(2, 5, 60), config());
    for tick in 1..=16 {
        for slot in 0..2 {
            live.set_input(PlayerSlot(slot), input(slot, tick));
        }
        for (at, signal) in plan() {
            if at == tick {
                live.push_signal(signal).unwrap();
            }
        }
        live.step_now().unwrap();
    }
    let checksums: Vec<u64> = (0..=16).map(|t| live.checksum_at(t).unwrap()).collect();
    assert_eq!(links(live.frame()), vec![0, 1]);
    let bytes = live.save_replay();

    let report = replay_verify::<Lobby>(&bytes, config()).unwrap();
    assert!(report.ok(), "replay diverged: {:?}", report.mismatch);

    let reader = ReplayReader::<Lobby>::parse(&bytes).unwrap();
    for tick in 1..=16 {
        assert_eq!(reader.seek(config(), tick).unwrap().checksum(), checksums[tick as usize], "tick {tick}");
    }
    assert_eq!(links(reader.seek(config(), 10).unwrap().frame()), vec![0], "slot 1 was removed in tick 9");
}

// ---- prediction: a late signal is resimulated after a rollback ----

const LATENCY: u64 = 6;
const CALLS: u64 = 50;

struct Pair {
    a: Session<Lobby, orr_session::LoopbackEnd<Lobby>>,
    b: Session<Lobby, orr_session::LoopbackEnd<Lobby>>,
    clock: orr_session::LoopbackClock,
    rollbacks: Vec<orr_session::RollbackInfo>,
    call: u64,
}

impl Pair {
    fn new() -> Self {
        let (end_a, end_b, clock) = LoopbackNetwork::new::<Lobby>(LATENCY, 0, 11);
        let a = Session::<Lobby, _>::new(config(), SessionConfig::new(2, PlayerSlot(0), 42, 60), end_a);
        let b = Session::<Lobby, _>::new(config(), SessionConfig::new(2, PlayerSlot(1), 42, 60), end_b);
        Self { a, b, clock, rollbacks: Vec::new(), call: 0 }
    }

    fn signal(&mut self, tick: u64, signal: RawPlayerSignal) {
        self.a.schedule_signal(tick, signal.clone()).unwrap();
        self.b.schedule_signal(tick, signal).unwrap();
    }

    fn advance(&mut self) {
        self.call += 1;
        let target = self.call + 2;
        self.clock.tick();
        let result = self.a.advance(input(0, target), Vec::new());
        let _ = self.b.advance(input(1, target), Vec::new());
        if let AdvanceResult::Advanced { rollback, .. } = result {
            self.rollbacks.extend(rollback);
        }
    }

    fn drain(&mut self) {
        for _ in 0..(LATENCY + 4) {
            self.clock.tick();
            self.rollbacks.extend(self.a.poll_confirmed().1);
            let _ = self.b.poll_confirmed();
        }
        assert_eq!(self.a.verified_tick(), self.a.head_tick(), "every tick is verified");
    }
}

/// `late`: each signal is scheduled when the head has passed its tick, so the tick was predicted
/// without the avatar and is resimulated with it.
fn scenario(late: bool) -> Pair {
    let mut pair = Pair::new();
    let signals = [
        (4, RawPlayerSignal::added(PlayerSlot(0))),
        (5, RawPlayerSignal::added(PlayerSlot(1))),
        (22, RawPlayerSignal::removed(PlayerSlot(0))),
        (30, RawPlayerSignal::added(PlayerSlot(0))),
    ];
    if !late {
        for (tick, signal) in signals.clone() {
            pair.signal(tick, signal);
        }
    }
    let mut done = Vec::new();
    for _ in 0..CALLS {
        let mut due = Vec::new();
        if late {
            for (tick, signal) in signals.clone() {
                if !done.contains(&tick) && pair.a.head_tick() > tick {
                    pair.signal(tick, signal);
                    done.push(tick);
                    due.push(tick);
                }
            }
        }
        let before = pair.rollbacks.len();
        pair.advance();
        for tick in due {
            assert!(pair.rollbacks[before..].iter().any(|r| r.from_tick <= tick && tick <= r.to_tick), "tick {tick} was resimulated");
        }
    }
    pair.drain();
    pair
}

#[test]
fn an_avatar_spawned_in_a_resimulated_tick_equals_the_one_spawned_in_time() {
    let in_time = scenario(false);
    let late = scenario(true);

    assert!(in_time.rollbacks.is_empty() || late.rollbacks.len() > in_time.rollbacks.len(), "the late run rolled back more");
    assert!(!late.rollbacks.is_empty());
    let head = in_time.a.head_tick();
    assert_eq!(head, late.a.head_tick());
    for tick in 1..=head {
        let (one, two) = (in_time.a.frame_at(tick).map(orr_ecs::Frame::checksum), late.a.frame_at(tick).map(orr_ecs::Frame::checksum));
        if one.is_some() && two.is_some() {
            assert_eq!(one, two, "tick {tick}");
        }
    }
    let final_frame = late.a.verified_frame().expect("everything is verified");
    assert_eq!(links(final_frame), vec![0, 1]);
    assert_eq!(final_frame.checksum(), in_time.a.verified_frame().unwrap().checksum());
    assert_eq!(late.a.verified_frame().unwrap().checksum(), late.b.verified_frame().unwrap().checksum(), "both peers agree");
}
