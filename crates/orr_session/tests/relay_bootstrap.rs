//! Valid reliable bootstrap traffic must not depend on transport poll batching.
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use orr_proto::{Bundle, Channel, ClientMsg, Link, LinkEvent, ServerMsg, SlotConfirmed, Welcome};
use orr_session::{ClientState, DumpCollector, RelayClient, RelayClientConfig};
use orr_sim::{Simulation, TickInputs};
use orr_testgame::{Arena, ArenaConfig, ArenaInput, SpawnBulletCmd};

#[derive(Clone, Default)]
struct ScriptedLink {
    incoming: Rc<RefCell<VecDeque<LinkEvent>>>,
    sent: Rc<RefCell<Vec<ClientMsg>>>,
}

impl ScriptedLink {
    fn receive(&self, msg: ServerMsg) {
        self.incoming.borrow_mut().push_back(LinkEvent::Message {
            channel: Channel::Reliable,
            data: msg.encode(),
        });
    }
}

impl Link for ScriptedLink {
    fn send(&mut self, _channel: Channel, data: &[u8]) {
        self.sent.borrow_mut().push(ClientMsg::decode(data).unwrap());
    }
    fn poll(&mut self) -> Option<LinkEvent> {
        self.incoming.borrow_mut().pop_front()
    }
    fn close(&mut self) {}
}

fn rejoin_bootstrap(split_after_welcome: bool) {
    const BUILD: u64 = 1;
    const SNAPSHOT: u64 = 155;
    const NOW: u64 = 2_600_000;
    let link = ScriptedLink::default();
    link.incoming.borrow_mut().push_back(LinkEvent::Connected);
    let mut cfg = RelayClientConfig::new(7, BUILD);
    cfg.token = 123;
    let mut client = RelayClient::<Arena, _>::new(
        cfg, link.clone(), |w| ArenaConfig { player_count: w.player_count }, DumpCollector::new(),
    );
    let mut idle = |_| (ArenaInput::default(), Vec::new());
    client.update(0, &mut idle);
    assert_eq!(client.state(), &ClientState::Handshake);
    assert!(matches!(link.sent.borrow().first(), Some(ClientMsg::Hello(h)) if h.token == 123));

    let mut sim = Simulation::<Arena>::with_build_id(ArenaConfig { player_count: 2 }, 60, 42, BUILD);
    for tick in 1..=SNAPSHOT {
        sim.step(&TickInputs::<ArenaInput, SpawnBulletCmd>::new(tick, 2));
    }
    let snapshot = ServerMsg::JoinSnapshot {
        tick: SNAPSHOT,
        checksum: sim.checksum(),
        data: lz4_flex::block::compress_prepend_size(&sim.frame().to_bytes()),
    };
    link.receive(ServerMsg::Welcome(Welcome {
        room: 7, slot: 1, player_count: 2, tick_rate: 60, seed: 42,
        build_hash: orr_sim::build_hash_of(BUILD, 0),
        input_size: std::mem::size_of::<ArenaInput>() as u32,
        checksum_interval: 1, token: 123, running: true, t0_us: 0,
        server_time_us: NOW, finalized_tick: SNAPSHOT + 1, config: Vec::new(), flags: 0,
    }));
    if split_after_welcome {
        client.update(0, &mut idle);
        assert_eq!(client.state(), &ClientState::AwaitSnapshot);
    }
    link.receive(snapshot);
    let confirmed = |tick| ServerMsg::Confirmed {
        input_size: std::mem::size_of::<ArenaInput>() as u32,
        bundles: vec![Bundle {
            tick,
            slots: vec![SlotConfirmed {
                input: bytemuck::bytes_of(&ArenaInput::default()).to_vec(),
                commands: Vec::new(), flags: 0,
            }; 2],
        }],
    };
    // The bootstrap backlog is sent once, as on the reliable QUIC stream.
    link.receive(confirmed(SNAPSHOT + 1));
    link.receive(ServerMsg::Pong { seq: 1, finalized_tick: SNAPSHOT + 1, client_time_us: NOW, server_time_us: NOW, t0_us: 0 });
    client.update(NOW, &mut idle);
    assert_eq!(client.session().unwrap().verified_tick(), SNAPSHOT + 1, "the reliable bootstrap backlog must advance verification");
    assert_eq!(client.source_stats().decode_errors, 0, "valid backlog rejected before Welcome was applied");
    assert_eq!(client.source_stats().bundles_received, 1);
    assert_eq!(client.session().unwrap().verified_tick(), SNAPSHOT + 1);
    assert_eq!(client.session_mut().unwrap().source_mut().acked_tick(), SNAPSHOT + 1);
    sim.step(&TickInputs::<ArenaInput, SpawnBulletCmd>::new(SNAPSHOT + 1, 2));
    assert_eq!(client.session().unwrap().verified_frame().unwrap().checksum(), sim.checksum());

    // Later traffic must advance continuously without retransmitting tick 156.
    for tick in SNAPSHOT + 2..=180 {
        link.receive(confirmed(tick));
        client.update(NOW + (tick - SNAPSHOT - 1) * 16_667, &mut idle);
        sim.step(&TickInputs::<ArenaInput, SpawnBulletCmd>::new(tick, 2));
        assert_eq!(client.session().unwrap().verified_tick(), tick);
        assert_eq!(client.session().unwrap().verified_frame().unwrap().checksum(), sim.checksum());
    }
    assert_eq!(client.source_stats().decode_errors, 0);
    assert_eq!(client.session_mut().unwrap().source_mut().acked_tick(), 180);
}

#[test]
fn rejoin_bootstrap_same_poll_preserves_backlog() {
    rejoin_bootstrap(false);
}

#[test]
fn rejoin_bootstrap_split_poll_preserves_backlog() {
    rejoin_bootstrap(true);
}
