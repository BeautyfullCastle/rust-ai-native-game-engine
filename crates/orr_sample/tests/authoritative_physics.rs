//! Authoritative mode with the physics sample game: a server-driven bot
//! paddle, and what the server's simulation costs per tick (1000 bodies).
#![allow(clippy::disallowed_types)]
#![allow(clippy::float_arithmetic)]

use std::rc::Rc;
use std::time::{Duration, Instant};

use orr_proto::netsim::{LinkParams, PathParams};
use orr_proto::{Bundle, SlotConfirmed, Welcome};
use orr_sample::physics_game::{bot_input, NoCommand, PhysConfig, PhysGame, PhysInput, SceneMode, TICK_RATE};
use orr_server::harness::{ClientSpec, Harness};
use orr_server::{AuthoritativeConfig, GameSim, RoomConfig, ServerSim};
use orr_sim::PlayerSlot;

const SEED: u64 = 0x5EED;
const BOT_SLOT: u8 = 3;

fn scene(bodies: u32, players: u8) -> PhysConfig {
    let mut c = PhysConfig::new(bodies, SceneMode::Rain);
    c.paddles = u32::from(players);
    c
}

fn mix(mut x: u64) -> u64 {
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

fn bot_sim(bodies: u32, players: u8) -> GameSim<PhysGame> {
    GameSim::<PhysGame>::new(scene(bodies, players), TICK_RATE, SEED, 0, players)
        .with_brain(|_frame, tick, slot| (bot_input(SEED, tick, PlayerSlot(slot)), Vec::<NoCommand>::new()))
}

#[test]
fn a_server_driven_bot_paddle_is_seen_identically_by_the_clients() {
    let players = 4u8;
    let mut room = RoomConfig::new(players, TICK_RATE, SEED, std::mem::size_of::<PhysInput>() as u32);
    room.record_all = true;
    room.min_players_to_start = 3;
    let script = Rc::new(|client: usize, tick: u64| {
        let h = mix((tick / 9 + client as u64 * 5) ^ (client as u64) << 33);
        (PhysInput::new((h % 3) as i32 - 1, ((h >> 8) % 3) as i32 - 1, ((h >> 16) % 3) as i32 - 1, (h >> 24) % 4 == 0), Vec::new())
    });
    let auth = AuthoritativeConfig { server_slots: vec![BOT_SLOT], ..AuthoritativeConfig::default() };
    let mut h = Harness::<PhysGame>::authoritative(
        11,
        room,
        auth,
        Box::new(bot_sim(200, players)),
        script,
        move |_: &Welcome| scene(200, players),
    );
    let p = PathParams::symmetric(LinkParams::new(50_000, 8_000).with_loss_ppm(15_000).with_dup_ppm(5_000).with_reorder(15_000, 12_000));
    for _ in 0..3 {
        h.add_client(&ClientSpec { build_id: 0, ..ClientSpec::new(p) });
    }
    assert!(h.run_until_all_playing(10_000_000));
    assert!(h.run_until_tick(1500, 120_000_000));
    h.run_for_us(500_000);

    let server: std::collections::BTreeMap<u64, u64> = h.server.server_checksums(h.room).iter().copied().collect();
    let mut compared = 0;
    for (i, c) in h.clients.iter().enumerate() {
        let s = c.client.session().unwrap();
        for &(tick, cs) in s.checksums() {
            assert_eq!(cs, server[&tick], "client {i} differs from the server at tick {tick}");
            compared += 1;
        }
        assert_eq!(c.client.stats().corrections, 0);
        assert!(c.dumps.is_empty());
    }
    assert!(compared >= 3 * 40, "{compared}");
    let bot_active = h.recorded().iter().filter(|b| b.slots[usize::from(BOT_SLOT)].input != vec![0u8; std::mem::size_of::<PhysInput>()]).count();
    assert!(bot_active > 500, "the bot paddle was active on only {bot_active} ticks");
    let st = h.server.room_stats(h.room).unwrap();
    assert_eq!((st.desyncs, st.corrections, st.kicks), (0, 0, 0));
    eprintln!("physics authoritative with a bot slot: {compared} client checkpoints equal the server's, bot active on {bot_active} ticks");
}

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    sorted[((sorted.len() as f64 * p) as usize).min(sorted.len() - 1)]
}

/// What the server pays per tick: `drive`, `step` (the sim), the checkpoint
/// checksum, and a snapshot (frame bytes + lz4, what a correction or a late
/// join costs). Printed; the asserts only catch pathologies.
#[test]
fn server_sim_cost_per_tick_with_1000_bodies() {
    let players = 4u8;
    let mut sim = bot_sim(1000, players);
    let size = std::mem::size_of::<PhysInput>();
    let ticks = 900u64;
    let mut step = Vec::new();
    let mut checksum = Vec::new();
    for tick in 1..=ticks {
        let driven = sim.drive(tick, &[BOT_SLOT]);
        let slots: Vec<SlotConfirmed> = (0..players)
            .map(|s| {
                let input = if s == BOT_SLOT { driven[0].1.clone() } else { bytemuck::bytes_of(&bot_input(SEED ^ 7, tick, PlayerSlot(s))).to_vec() };
                assert_eq!(input.len(), size);
                SlotConfirmed { input, commands: Vec::new(), flags: 0 }
            })
            .collect();
        let bundle = Bundle { tick, slots };
        let t = Instant::now();
        let v = sim.step(&bundle);
        step.push(t.elapsed());
        assert!(v.is_empty());
        if tick % 30 == 0 {
            let t = Instant::now();
            let _ = sim.checksum();
            checksum.push(t.elapsed());
        }
    }
    step.sort();
    checksum.sort();
    let sum: Duration = step.iter().sum();
    let avg = sum / step.len() as u32;
    let t = Instant::now();
    let bytes = sim.frame_bytes();
    let to_bytes = t.elapsed();
    let t = Instant::now();
    let packed = lz4_flex::block::compress_prepend_size(&bytes);
    let lz4 = t.elapsed();
    eprintln!(
        "PhysGame 1000 bodies, {ticks} ticks: step avg {:.3} ms, p50 {:.3}, p99 {:.3}, max {:.3} ms (budget 16.7 ms at 60 Hz); checksum {:.3} ms (every 30 ticks); \
         snapshot: to_bytes {:.3} ms + lz4 {:.3} ms, {} B raw -> {} B",
        avg.as_secs_f64() * 1e3,
        percentile(&step, 0.5).as_secs_f64() * 1e3,
        percentile(&step, 0.99).as_secs_f64() * 1e3,
        step.last().unwrap().as_secs_f64() * 1e3,
        percentile(&checksum, 0.5).as_secs_f64() * 1e3,
        to_bytes.as_secs_f64() * 1e3,
        lz4.as_secs_f64() * 1e3,
        bytes.len(),
        packed.len()
    );
    assert!(avg < Duration::from_millis(30), "the server sim takes {avg:?} per tick");
}
