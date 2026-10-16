use super::*;
use orr_proto::{Channel, ClientMsg, LinkEvent, ServerMsg, Welcome};
use orr_session::{DumpCollector, RelayClientConfig};
use orr_testgame::{Arena, ArenaConfig, ArenaInput, SpawnBulletCmd};
use std::collections::VecDeque;

struct FakeLink {
    inbox: Arc<Mutex<VecDeque<LinkEvent>>>,
}

impl Link for FakeLink {
    fn send(&mut self, _channel: Channel, bytes: &[u8]) {
        if matches!(ClientMsg::decode(bytes), Ok(ClientMsg::Hello(_))) {
            let welcome = Welcome {
                room: 1,
                slot: 0,
                player_count: 2,
                tick_rate: 1,
                seed: 11,
                build_hash: 0,
                input_size: std::mem::size_of::<ArenaInput>() as u32,
                checksum_interval: 30,
                token: 0,
                running: false,
                t0_us: 0,
                server_time_us: 0,
                finalized_tick: 0,
                config: Vec::new(),
                flags: 0,
            };
            let mut inbox = self.inbox.lock().unwrap();
            for msg in [
                ServerMsg::Welcome(welcome),
                ServerMsg::Start {
                    t0_us: 0,
                    server_time_us: 0,
                },
            ] {
                inbox.push_back(LinkEvent::Message {
                    channel: Channel::Reliable,
                    data: msg.encode(),
                });
            }
        }
    }
    fn poll(&mut self) -> Option<LinkEvent> {
        self.inbox.lock().unwrap().pop_front()
    }
    fn close(&mut self) {
        self.inbox
            .lock()
            .unwrap()
            .push_back(LinkEvent::Disconnected);
    }
}

#[test]
fn relay_reports_unsubmitted_staging_and_stops_growth_after_disconnect() {
    let inbox = Arc::new(Mutex::new(VecDeque::from([LinkEvent::Connected])));
    let link = FakeLink {
        inbox: inbox.clone(),
    };
    let mut cfg = RelayClientConfig::new(1, 0);
    cfg.target_slack_milliticks = 0;
    let client = RelayClient::<Arena, _>::new(
        cfg,
        link,
        |_| ArenaConfig { player_count: 2 },
        DumpCollector::new(),
    );
    let mut host = RelayHost::connect(client, RelayHostOptions::default()).unwrap();
    // Freeze the host's local wall clock at zero without sleeping. The relay
    // has no due send tick, even though advance is called repeatedly.
    host.start = Instant::now() + Duration::from_secs(3600);
    let _ = host.advance(
        ArenaInput::default(),
        vec![SpawnBulletCmd { owner: 0 }, SpawnBulletCmd { owner: 1 }],
    );
    for _ in 0..20 {
        let _ = host.advance(ArenaInput::default(), Vec::new());
        assert_eq!(host.pending_command_count(), 2);
    }
    // Make a send tick due. The complete pending batch is submitted once.
    host.start = Instant::now() - Duration::from_secs(2);
    let _ = host.advance(ArenaInput::default(), Vec::new());
    assert_eq!(host.pending_command_count(), 0);

    host.start = Instant::now() + Duration::from_secs(3600);
    let _ = host.advance(ArenaInput::default(), vec![SpawnBulletCmd { owner: 0 }]);
    assert_eq!(host.pending_command_count(), 1);
    inbox.lock().unwrap().push_back(LinkEvent::Disconnected);
    let _ = host.advance(ArenaInput::default(), Vec::new());
    assert!(!host.accepts_commands());
    assert!(host
        .take_lifecycle()
        .iter()
        .any(|note| matches!(note, Lifecycle::Disconnected)));
    for _ in 0..20 {
        let _ = host.advance(ArenaInput::default(), vec![SpawnBulletCmd { owner: 1 }]);
        assert_eq!(host.pending_command_count(), 1);
    }
    assert!(
        host.take_lifecycle().is_empty(),
        "terminal disconnect is reported once"
    );
}
