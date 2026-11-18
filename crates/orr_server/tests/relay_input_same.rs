//! The "same as the previous input" flag of client `Input` messages under
//! packet loss, duplication and reordering, and the version of `Hello`.
mod common;

use common::*;
use orr_proto::netsim::{PathParams, SimNet};
use orr_proto::{
    Channel, ClientMsg, Hello, Link, LinkEvent, RejectReason, ServerMsg, FLAG_ABSENT, FLAG_REPEATED, NO_SLOT, PROTOCOL_VERSION,
};
use orr_server::harness::{headless_checksums, ClientSpec, Harness};
use orr_server::RelayServer;
use orr_testgame::{Arena, ArenaConfig};
use std::collections::BTreeMap;

const PLAYERS: u8 = 3;
const WARM_UP: u64 = 300;

struct Run {
    h: Harness<Arena>,
    lost: u64,
}

fn run(period: u64, loss_ppm: u32, net_seed: u64) -> Run {
    let mut h = Harness::<Arena>::new(net_seed, arena_room(PLAYERS), periodic_script(period), |w| ArenaConfig {
        player_count: w.player_count,
    });
    // Both runs start on a clean network, so their first 300 ticks are the
    // same. The lossy run gets loss, duplicates and reordering after that.
    let clean = path(35, 6, 0);
    for _ in 0..PLAYERS {
        h.add_client(&ClientSpec::new(clean));
    }
    assert!(h.run_until_all_playing(10_000_000));
    assert!(h.run_until_tick(WARM_UP, 60_000_000));
    for i in 0..PLAYERS as usize {
        h.set_path(i, path(35, 6, loss_ppm));
    }
    assert!(h.run_until_tick(1800, 120_000_000));
    h.run_for_us(500_000);
    let lost = h.net.stats().unreliable_lost;
    Run { h, lost }
}

fn checkpoints(h: &Harness<Arena>) -> BTreeMap<u64, u64> {
    headless_checksums::<Arena>(ArenaConfig { player_count: PLAYERS }, TICK_RATE, SEED, 1, PLAYERS, 30, h.recorded())
}

#[test]
fn packet_loss_never_changes_an_input_the_server_confirms() {
    for (period, net_seed) in [(0u64, 11u64), (10, 12), (3, 13)] {
        let clean = run(period, 0, net_seed);
        let lossy = run(period, 60_000, net_seed);
        assert!(lossy.lost > 100, "period {period}: only {} packets lost", lossy.lost);
        assert_eq!(clean.lost, 0);

        // Every input the server took from a packet is the one the client
        // meant for that tick, whichever packets were lost, doubled or late.
        for run in [&clean, &lossy] {
            let slot_client: BTreeMap<u8, usize> =
                (0..PLAYERS as usize).map(|i| (run.h.clients[i].client.welcome().unwrap().slot, i)).collect();
            let mut judged = 0;
            for b in run.h.recorded() {
                for (slot, s) in b.slots.iter().enumerate() {
                    if s.flags & (FLAG_REPEATED | FLAG_ABSENT) != 0 {
                        continue;
                    }
                    let client = slot_client[&(slot as u8)];
                    let want = periodic_input(client, b.tick, period);
                    assert_eq!(s.input.as_slice(), bytemuck::bytes_of(&want), "period {period}, tick {} slot {slot}", b.tick);
                    judged += 1;
                }
            }
            assert!(judged > 3000, "period {period}: judged {judged} inputs");
        }

        // Every client agrees with a clean replay of what the server confirmed.
        assert_matches_headless::<Arena>(&lossy.h, || ArenaConfig { player_count: PLAYERS }, PLAYERS, None);
        assert_matches_headless::<Arena>(&clean.h, || ArenaConfig { player_count: PLAYERS }, PLAYERS, None);
        assert_clients_agree(&lossy.h);

        // Up to the first tick the lossy server had to repeat an input, both
        // runs confirmed the same inputs, so their checksums are the same.
        let first_repeat = lossy
            .h
            .recorded()
            .iter()
            .find(|b| b.tick > WARM_UP && b.slots.iter().any(|s| s.flags & (FLAG_REPEATED | FLAG_ABSENT) != 0))
            .map_or(u64::MAX, |b| b.tick);
        let (a, b) = (checkpoints(&clean.h), checkpoints(&lossy.h));
        let mut compared = 0;
        for (t, c) in &a {
            if *t < first_repeat {
                assert_eq!(b.get(t), Some(c), "period {period}: checksum at tick {t}");
                compared += 1;
            }
        }
        eprintln!(
            "period {period}: {} packets lost, first repeated input at tick {}, {compared} checkpoints equal",
            lossy.lost,
            if first_repeat == u64::MAX { 0 } else { first_repeat }
        );
        assert!(compared >= 10, "period {period}: only {compared} checkpoints compared");
    }
}

// ---- the version in Hello ---------------------------------------------------

/// The server's answer to a `Hello` whose header states `version`.
fn answer(version: u32) -> ServerMsg {
    let net = SimNet::new(3);
    let mut server = RelayServer::new(net.endpoint(), 5);
    server.create_room(1, arena_room(2));
    let mut link = net.connect(PathParams::default());
    assert_eq!(link.poll(), Some(LinkEvent::Connected));
    let hello = Hello {
        build_hash: orr_sim::build_hash_of(1, 0),
        room: 1,
        input_size: std::mem::size_of::<orr_testgame::ArenaInput>() as u32,
        want_slot: NO_SLOT,
        token: 0,
        player_data: Vec::new(),
    };
    let mut bytes = ClientMsg::Hello(hello).encode();
    // The version is checked before the message checksum, so no re-sealing.
    bytes[4..8].copy_from_slice(&version.to_le_bytes());
    link.send(Channel::Reliable, &bytes);
    server.update(0);
    let Some(LinkEvent::Message { channel: Channel::Reliable, data }) = link.poll() else { panic!("no answer to Hello") };
    ServerMsg::decode(&data).expect("valid answer")
}

#[test]
fn a_hello_of_another_version_is_refused_with_both_versions() {
    for client in [0, 2, 5, PROTOCOL_VERSION + 1, u32::MAX] {
        assert_eq!(answer(client), ServerMsg::Reject(RejectReason::BadVersion { server: PROTOCOL_VERSION, client }), "client {client}");
    }
}

#[test]
fn a_hello_of_the_current_version_is_welcomed() {
    assert!(matches!(answer(PROTOCOL_VERSION), ServerMsg::Welcome(_)));
}
