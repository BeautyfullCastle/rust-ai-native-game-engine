//! Relay mode with the physics sample game (small scene): 4 clients and a
//! relay server on the simulated network at 150 ms round trip, 2% loss.
use std::rc::Rc;

use orr_proto::netsim::{LinkParams, PathParams};
use orr_proto::Welcome;
use orr_sample::physics_game::{PhysConfig, PhysGame, PhysInput, SceneMode, TICK_RATE};
use orr_server::harness::{headless_checksums, ClientSpec, Harness};
use orr_server::RoomConfig;

const SEED: u64 = 0x5EED;
const TICKS: u64 = 2500;

fn mix(mut x: u64) -> u64 {
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

fn scene() -> PhysConfig {
    PhysConfig::new(60, SceneMode::Rain)
}

#[test]
fn four_clients_at_150ms_rtt_match_the_headless_physics_run() {
    let mut room = RoomConfig::new(4, TICK_RATE, SEED, std::mem::size_of::<PhysInput>() as u32);
    room.record_all = true;
    let script = Rc::new(|client: usize, tick: u64| {
        let h = mix((tick / 9 + client as u64 * 5) ^ (client as u64) << 33);
        let input = PhysInput::new((h % 3) as i32 - 1, ((h >> 8) % 3) as i32 - 1, ((h >> 16) % 3) as i32 - 1, (h >> 24) % 4 == 0);
        (input, Vec::new())
    });
    let mut h = Harness::<PhysGame>::new(11, room, script, |_: &Welcome| scene());
    let p = PathParams::symmetric(
        LinkParams::new(70_000, 10_000).with_loss_ppm(20_000).with_dup_ppm(5_000).with_reorder(20_000, 15_000),
    );
    for _ in 0..4 {
        h.add_client(&ClientSpec { build_id: 0, ..ClientSpec::new(p) });
    }
    assert!(h.run_until_all_playing(10_000_000));
    assert!(h.run_until_tick(300, 30_000_000));
    let warm: Vec<(u64, u64, u64)> = (0..4)
        .map(|i| {
            let c = &h.clients[i].client;
            (c.session().unwrap().rollback_count(), c.stats().stall_episodes, c.stats().stalled_us / 1000)
        })
        .collect();
    assert!(h.run_until_tick(TICKS, 120_000_000));
    h.run_for_us(500_000);

    let reference = headless_checksums::<PhysGame>(scene(), TICK_RATE, SEED, 0, 4, 30, h.recorded());
    let mut compared = 0;
    for (i, c) in h.clients.iter().enumerate() {
        let s = c.client.session().unwrap();
        for &(tick, cs) in s.checksums() {
            assert_eq!(cs, reference[&tick], "client {i} differs from the headless run at tick {tick}");
            compared += 1;
        }
        let rb = s.rollback_count() - warm[i].0;
        eprintln!(
            "client {i}: rollbacks {rb} ({:.1}/s), stall episodes {} ({} ms), delay {}, rate {} ppm, verified {}",
            rb as f64 / ((TICKS - 300) as f64 / 60.0),
            c.client.stats().stall_episodes - warm[i].1,
            c.client.stats().stalled_us / 1000 - warm[i].2,
            c.client.delay(),
            c.client.rate_ppm(),
            s.verified_tick()
        );
        assert!(s.verified_tick() + 60 >= TICKS);
        assert!(c.dumps.is_empty());
    }
    assert!(compared >= 4 * 70, "{compared}");
    assert_eq!(h.net.stats().oversize_dropped, 0);
    assert_eq!(h.server.room_stats(h.room).unwrap().desyncs, 0);
}
