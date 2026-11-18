//! An `Input` packet never has more entries than the server decodes, however
//! large `input_redundancy` is: equal inputs are small, so the byte budget alone
//! would let more than `MAX_INPUT_ENTRIES` of them into one packet.
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use orr_proto::{Channel, ClientMsg, Link, LinkEvent, ServerMsg, Welcome, MAX_INPUT_ENTRIES};
use orr_session::{ClientState, DumpCollector, RelayClient, RelayClientConfig};
use orr_testgame::{Arena, ArenaConfig, ArenaInput};

#[derive(Clone, Default)]
struct ScriptedLink {
    incoming: Rc<RefCell<VecDeque<LinkEvent>>>,
    sent: Rc<RefCell<Vec<Vec<u8>>>>,
}

impl ScriptedLink {
    fn receive(&self, msg: ServerMsg) {
        self.incoming.borrow_mut().push_back(LinkEvent::Message { channel: Channel::Reliable, data: msg.encode() });
    }
}

impl Link for ScriptedLink {
    fn send(&mut self, _channel: Channel, data: &[u8]) {
        self.sent.borrow_mut().push(data.to_vec());
    }
    fn poll(&mut self) -> Option<LinkEvent> {
        self.incoming.borrow_mut().pop_front()
    }
    fn close(&mut self) {}
}

/// Plays `ticks` ticks of one unchanging input without any confirmation from
/// the server, with the given `input_redundancy`. Returns the `Input` packets
/// the server would receive, as the server decodes them. Any packet the
/// server cannot decode fails the test.
fn packets_after(redundancy: u32, ticks: u64) -> Vec<ClientMsg> {
    const TICK_RATE: u64 = 60;
    const START: u64 = 1_000_000;
    let link = ScriptedLink::default();
    link.incoming.borrow_mut().push_back(LinkEvent::Connected);
    let mut cfg = RelayClientConfig::new(7, 1);
    cfg.input_redundancy = redundancy;
    cfg.max_prediction = 1000;
    let mut client = RelayClient::<Arena, _>::new(cfg, link.clone(), |w| ArenaConfig { player_count: w.player_count }, DumpCollector::new());
    let mut same = |_| (ArenaInput::default(), Vec::new());
    client.update(0, &mut same);
    link.receive(ServerMsg::Welcome(Welcome {
        room: 7,
        slot: 0,
        player_count: 2,
        tick_rate: TICK_RATE as u32,
        seed: 42,
        build_hash: orr_sim::build_hash_of(1, 0),
        input_size: std::mem::size_of::<ArenaInput>() as u32,
        checksum_interval: 30,
        token: 1,
        running: false,
        t0_us: 0,
        server_time_us: START,
        finalized_tick: 0,
        config: Vec::new(),
        flags: 0,
    }));
    client.update(0, &mut same);
    link.receive(ServerMsg::Start { t0_us: 0, server_time_us: START });
    client.update(START, &mut same);
    assert_eq!(client.state(), &ClientState::Playing);
    link.sent.borrow_mut().clear();
    // One long step makes all the ticks due at once. The next call sends the
    // newest inputs (nothing is confirmed, so nothing is dropped).
    client.update(START + ticks * 1_000_000 / TICK_RATE, &mut same);
    let sent = link.sent.borrow().clone();
    let decoded: Vec<ClientMsg> = sent.iter().map(|b| ClientMsg::decode(b).expect("the server must decode every packet")).collect();
    decoded.into_iter().filter(|m| matches!(m, ClientMsg::Input { .. })).collect()
}

fn check(redundancy: u32, ticks: u64) {
    let packets = packets_after(redundancy, ticks);
    assert!(!packets.is_empty(), "no input packet sent");
    for p in packets {
        let ClientMsg::Input { entries, .. } = p else { unreachable!() };
        assert!(entries.len() <= MAX_INPUT_ENTRIES, "{} entries", entries.len());
        assert_eq!(entries.len(), (redundancy as usize).min(MAX_INPUT_ENTRIES).min(ticks as usize), "redundancy {redundancy}");
        // The newest inputs are the ones kept, in tick order.
        assert!(entries.windows(2).all(|w| w[1].tick == w[0].tick + 1));
    }
}

#[test]
fn a_redundancy_of_256_sends_256_entries() {
    check(256, 400);
}

#[test]
fn a_redundancy_of_257_is_lowered_to_the_codec_maximum() {
    check(257, 400);
}

#[test]
fn a_redundancy_of_300_is_lowered_to_the_codec_maximum() {
    check(300, 400);
}

#[test]
fn a_huge_redundancy_is_lowered_to_the_codec_maximum() {
    check(u32::MAX, 400);
}
