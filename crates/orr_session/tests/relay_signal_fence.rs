//! The client side of the signal fence, with a scripted server: a confirmed
//! bundle that counts signals the client does not have yet is not verified
//! until the signals are in, whatever the order they come in.
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use orr_proto::{Bundle, Channel, ClientMsg, Link, LinkEvent, ServerMsg, SlotConfirmed, Welcome};
use orr_session::{ClientEvent, ClientState, DumpCollector, RelayClient, RelayClientConfig};
use orr_testgame::{Arena, ArenaConfig, ArenaInput};

const BUILD: u64 = 1;

#[derive(Clone, Default)]
struct ScriptedLink {
    incoming: Rc<RefCell<VecDeque<LinkEvent>>>,
}

impl ScriptedLink {
    fn receive(&self, msg: ServerMsg) {
        self.incoming.borrow_mut().push_back(LinkEvent::Message { channel: Channel::Reliable, data: msg.encode() });
    }
}

impl Link for ScriptedLink {
    fn send(&mut self, _channel: Channel, data: &[u8]) {
        ClientMsg::decode(data).unwrap();
    }
    fn poll(&mut self) -> Option<LinkEvent> {
        self.incoming.borrow_mut().pop_front()
    }
    fn close(&mut self) {}
}

struct Rig {
    link: ScriptedLink,
    client: RelayClient<Arena, ScriptedLink>,
    now_us: u64,
}

impl Rig {
    /// A client of a two-player room that has just started.
    fn started() -> Self {
        let link = ScriptedLink::default();
        link.incoming.borrow_mut().push_back(LinkEvent::Connected);
        let client = RelayClient::<Arena, _>::new(
            RelayClientConfig::new(7, BUILD),
            link.clone(),
            |w| ArenaConfig { player_count: w.player_count },
            DumpCollector::new(),
        );
        let mut rig = Rig { link, client, now_us: 0 };
        rig.update();
        link_welcome(&rig.link);
        rig.update();
        assert_eq!(rig.client.state(), &ClientState::Syncing);
        rig.link.receive(ServerMsg::Start { t0_us: 0, server_time_us: 0 });
        rig.update();
        assert_eq!(rig.client.state(), &ClientState::Playing);
        rig
    }

    fn update(&mut self) {
        self.now_us += 100_000;
        let mut idle = |_| (ArenaInput::default(), Vec::new());
        self.client.update(self.now_us, &mut idle);
    }

    /// Runs enough updates for the session to take in everything that came.
    fn settle(&mut self) {
        for _ in 0..20 {
            self.update();
        }
    }

    fn bundles(&self, ticks: std::ops::RangeInclusive<u64>, signals: u32) {
        let bundles = ticks
            .map(|tick| Bundle {
                tick,
                signals,
                slots: vec![
                    SlotConfirmed {
                        input: bytemuck::bytes_of(&ArenaInput::default()).to_vec(),
                        commands: Vec::new(),
                        flags: 0,
                    };
                    2
                ],
            })
            .collect();
        self.link.receive(ServerMsg::Confirmed { input_size: std::mem::size_of::<ArenaInput>() as u32, bundles });
    }

    fn signal(&self, seq: u32, slot: u8, from_tick: u64) {
        self.link.receive(ServerMsg::Presence { slot, present: true, from_tick, seq });
    }

    fn verified(&self) -> u64 {
        self.client.session().unwrap().verified_tick()
    }
}

fn link_welcome(link: &ScriptedLink) {
    link.receive(ServerMsg::Welcome(Welcome {
        room: 7,
        slot: 0,
        player_count: 2,
        tick_rate: 60,
        seed: 42,
        build_hash: orr_sim::build_hash_of(BUILD, 0),
        input_size: std::mem::size_of::<ArenaInput>() as u32,
        checksum_interval: 1,
        token: 5,
        running: false,
        t0_us: 0,
        server_time_us: 0,
        finalized_tick: 0,
        config: Vec::new(),
        flags: 0,
    }));
}

#[test]
fn bundles_wait_for_the_signals_they_count() {
    let mut rig = Rig::started();
    // The bundles of ticks 1..=20 count one signal (for tick 5), which has not come.
    rig.bundles(1..=4, 0);
    rig.bundles(5..=20, 1);
    rig.settle();
    assert_eq!(rig.verified(), 4, "the ticks before the signal do not wait");
    assert_eq!(rig.client.source_stats().bundles_fenced, 16);

    rig.signal(1, 0, 5);
    rig.settle();
    assert_eq!(rig.verified(), 20);
    assert_eq!(rig.client.stats().signals_late, 0);
    assert_eq!(*rig.client.state(), ClientState::Playing);
}

#[test]
fn signals_that_come_out_of_order_or_twice_open_the_fence_once_they_are_all_in() {
    let mut rig = Rig::started();
    rig.bundles(1..=10, 2);
    rig.signal(2, 1, 3);
    rig.settle();
    assert_eq!(rig.verified(), 0, "signal 1 is missing");
    rig.signal(2, 1, 3);
    rig.settle();
    assert_eq!(rig.verified(), 0, "a repeated signal 2 is not signal 1");
    rig.signal(1, 0, 2);
    rig.settle();
    assert_eq!(rig.verified(), 10);
    // A repeat of a signal that is in changes nothing (it is not scheduled twice).
    rig.signal(1, 0, 2);
    rig.bundles(11..=12, 2);
    rig.settle();
    assert_eq!(rig.verified(), 12);
    assert_eq!(rig.client.stats().signals_late, 0);
}

#[test]
fn a_signal_for_a_verified_tick_is_a_visible_failure() {
    let mut rig = Rig::started();
    rig.bundles(1..=10, 0);
    rig.settle();
    assert_eq!(rig.verified(), 10);
    // A server without the fence (counts of 0) sends the signal too late.
    rig.signal(1, 0, 4);
    rig.settle();
    assert_eq!(rig.client.stats().signals_late, 1);
    assert!(matches!(rig.client.state(), ClientState::Failed(_)), "{:?}", rig.client.state());
    assert!(rig.client.drain_events().contains(&ClientEvent::SignalLate { tick: 4 }));
}

#[test]
fn a_room_without_signals_is_never_fenced() {
    let mut rig = Rig::started();
    rig.bundles(1..=30, 0);
    rig.settle();
    assert_eq!(rig.verified(), 30);
    assert_eq!(rig.client.source_stats().bundles_fenced, 0);
}
