//! `TickInputs` player flags in a relay game: `predicted` is set on slots whose input was a guess
//! when the tick was simulated, `disconnected` comes from the server's confirmed bundle (slot
//! vacant) and is the same on every peer, so a game can use it. Also: every resimulated tick of a
//! rollback is counted in the client's `resim_ticks`.
mod common;

use std::cell::Cell;

use bytemuck::{Pod, Zeroable};
use common::{path, SEED, TICK_RATE};
use orr_proto::FLAG_ABSENT;
use orr_server::harness::{ClientSpec, Harness, Script};
use orr_server::RoomConfig;
use orr_sim::{decode_pod, encode_pod, Game, PlayerSlot, SimCommand, SimContext, System};
use std::rc::Rc;

#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug, Pod, Zeroable)]
struct In {
    add: u32,
    _pad: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug, Pod, Zeroable)]
struct NoCmd {
    _pad: u32,
}
impl SimCommand for NoCmd {
    fn encode(&self, out: &mut Vec<u8>) {
        encode_pod(self, out)
    }
    fn decode(bytes: &[u8]) -> Option<Self> {
        decode_pod(bytes)
    }
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Pod, Zeroable)]
struct NoEvent {
    _pad: u32,
}

/// Game state: per slot, the sum of its inputs while present, and the ticks it was disconnected.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Pod, Zeroable)]
struct Seen {
    sums: [u64; 3],
    disconnected: [u64; 3],
}

thread_local! {
    /// Slots-ticks simulated with the `predicted` flag set (not part of the state: a view hint).
    static PREDICTED_SEEN: Cell<u64> = const { Cell::new(0) };
}

struct FlagGame;
struct FlagSystem;
impl System<FlagGame> for FlagSystem {
    fn name(&self) -> &'static str {
        "FlagSystem"
    }
    fn run(&mut self, ctx: &mut SimContext<FlagGame>) {
        for s in 0..ctx.inputs.player_count().min(3) {
            let flags = ctx.inputs.flags(PlayerSlot(s));
            if flags.predicted {
                PREDICTED_SEEN.with(|c| c.set(c.get() + 1));
            }
            let seen = ctx.frame.singleton_mut::<Seen>();
            if flags.disconnected {
                // A game may ignore a disconnected player's input instead of repeating it.
                seen.disconnected[s as usize] += 1;
            } else {
                seen.sums[s as usize] += u64::from(ctx.inputs.input(PlayerSlot(s)).add);
            }
        }
    }
}
impl Game for FlagGame {
    type Input = In;
    type Command = NoCmd;
    type Event = NoEvent;
    type Config = ();
    fn register(b: &mut orr_ecs::ComponentRegistryBuilder) {
        b.register_singleton::<Seen>("Seen");
    }
    fn setup(frame: &mut orr_ecs::Frame, _: &()) {
        frame.set_singleton(Seen { sums: [0; 3], disconnected: [0; 3] });
    }
    fn systems() -> Vec<Box<dyn System<Self>>> {
        vec![Box::new(FlagSystem)]
    }
}

fn script() -> Script<FlagGame> {
    Rc::new(|client, tick| (In { add: ((tick / 5 + client as u64 * 7) % 4) as u32, _pad: 0 }, Vec::new()))
}

#[test]
fn predicted_and_disconnected_flags_reach_the_game() {
    let mut cfg = RoomConfig::new(3, TICK_RATE, SEED, 8);
    cfg.record_all = true;
    let mut h = Harness::<FlagGame>::new(5, cfg, script(), |_| ());
    let p = path(50, 8, 10_000);
    for _ in 0..3 {
        h.add_client(&ClientSpec::new(p));
    }
    assert!(h.run_until_all_playing(10_000_000));
    assert!(h.run_until_tick(300, 60_000_000));

    // Client 1 drops off the network (its slot is whatever the server gave it); the server keeps confirming its slot as vacant (repeated input).
    let gone = usize::from(h.clients[1].client.welcome().unwrap().slot);
    h.cut(1);
    let cut_tick = h.server.finalized_tick(h.room).unwrap();
    assert!(h.run_until_tick(cut_tick + 300, 60_000_000));
    h.run_for_us(500_000);

    // The predicted flag was set while the clients guessed other players' inputs.
    assert!(PREDICTED_SEEN.with(Cell::get) > 0, "no tick was simulated with a predicted input flag");

    // The disconnected flag: what the server recorded as absent, the verified state of the
    // others counted, and every peer counted the same.
    let absent_up_to = |tick: u64| h.recorded().iter().filter(|b| b.tick <= tick && b.slots[gone].flags & FLAG_ABSENT != 0).count() as u64;
    let mut checked = 0;
    let survivors = [0usize, 2];
    for i in survivors {
        let session = h.clients[i].client.session().expect("playing");
        let (verified, frame) = (session.verified_tick(), session.verified_frame().expect("a verified frame"));
        assert!(verified > cut_tick + 200, "client {i} verified only {verified}");
        let seen = frame.singleton::<Seen>();
        assert_eq!(seen.disconnected[gone], absent_up_to(verified), "client {i}: ticks slot {gone} was flagged disconnected");
        assert!(seen.disconnected[gone] > 150, "client {i}: {}", seen.disconnected[gone]);
        for other in (0..3).filter(|s| *s != gone) {
            assert_eq!(seen.disconnected[other], 0, "a present player is never flagged");
        }
        checked += 1;
    }
    assert_eq!(checked, 2);
    // Both survivors hold the same confirmed state.
    let sums: Vec<_> = [0usize, 2]
        .iter()
        .map(|&i| {
            let s = h.clients[i].client.session().unwrap();
            (s.verified_tick(), s.checksums().to_vec())
        })
        .collect();
    let common_len = sums[0].1.len().min(sums[1].1.len());
    assert!(common_len > 5);
    assert_eq!(sums[0].1[..common_len], sums[1].1[..common_len], "the clients disagree");

    // Every tick resimulated by a rollback is counted (there were rollbacks on this lossy path).
    for i in [0usize, 2] {
        let c = &h.clients[i].client;
        let rollbacks = c.session().unwrap().rollback_count();
        assert!(rollbacks > 0);
        assert!(c.stats().resim_ticks >= rollbacks, "client {i}: {} resimulated ticks for {rollbacks} rollbacks", c.stats().resim_ticks);
    }
}
