//! The two network run modes: `--host` (a relay server in this process) and `--connect`.
//!
//! A networked window is a relay client on its own thread (`orr_bridge::RelayHost` inside a
//! `Threaded` bridge), so the view code is the same as offline. The server is the plain relay
//! server of `orr_server`; `--host` only runs it on a second thread of the same process.
//! Both sides use WebSocket (`ws://`), which needs no certificate: this is for the same PC and
//! the local network. A server on the internet is `orr_server` with a trusted certificate.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::Duration;

use orr_bridge::{BridgeConfig, RelayHost, RelayHostOptions, StatusFn, Threaded, ThreadedConfig};
use orr_relay_net::{connect, listen, ConnectOptions, DirSink, ListenOptions, TransportKind, Trust};
use orr_server::serve::run_wall_clock;
use orr_server::{RelayServer, RoomConfig};
use orr_session::{RelayClient, RelayClientConfig};
use orr_sim::{build_hash_of, frame_build_id, Game, PlayerSlot};

/// The one room of an in-process server.
const ROOM: u64 = 1;
/// How long a window waits for the other players before it gives up.
const WAIT_FOR_PLAYERS: Duration = Duration::from_secs(600);

/// Identifies the game to the server: the same program on both sides gives the same number,
/// and a window of another game is refused at the handshake.
pub(crate) fn build_id<G: Game>() -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in std::any::type_name::<G>().bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    frame_build_id(hash | 1)
}

/// The room of `--host`: `players` seats, and the game starts when all of them are taken.
pub(crate) fn room_config<G: Game>(players: u8, tick_rate: u32, seed: u64) -> RoomConfig {
    let mut cfg = RoomConfig::new(players, tick_rate, seed, std::mem::size_of::<G::Input>() as u32);
    cfg.build_hash = build_hash_of(build_id::<G>(), 0);
    cfg
}

/// A relay server running on its own thread. Dropping it stops the server.
pub(crate) struct Hosted {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    addr: SocketAddr,
}

impl Hosted {
    /// Starts listening on `bind` and returns once the server is ready for clients.
    pub fn start(bind: SocketAddr, room: RoomConfig, token_seed: u64) -> Result<Self, String> {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let (ready_tx, ready_rx) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("orr-relay-server".to_string())
            .spawn(move || {
                let endpoint = match listen(&ListenOptions::new(bind, TransportKind::Ws)) {
                    Ok(endpoint) => endpoint,
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        return;
                    }
                };
                let addr = endpoint.local_addr();
                let mut server = RelayServer::new(endpoint, token_seed ^ 0x0BAD_5EED_0BAD_5EED);
                server.create_room(ROOM, room);
                let _ = ready_tx.send(Ok(addr));
                run_wall_clock(&mut server, &flag, Duration::from_millis(1), |_, _| {});
            })
            .map_err(|e| format!("start the relay server thread: {e}"))?;
        let addr = ready_rx.recv().map_err(|_| "the relay server thread ended at once".to_string())??;
        Ok(Self { stop, thread: Some(thread), addr })
    }

    /// The address the server listens on.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// The address a client on this PC uses (`0.0.0.0` is not one).
    pub fn client_address(&self) -> String {
        if self.addr.ip().is_unspecified() {
            format!("127.0.0.1:{}", self.addr.port())
        } else {
            self.addr.to_string()
        }
    }
}

impl Drop for Hosted {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Where and as whom a window joins.
pub(crate) struct Join {
    pub addr: String,
    /// The slot to ask for (`None`: any free one).
    pub slot: Option<u8>,
    /// The token of an earlier join, to take its slot back (0: none).
    pub token: u64,
    /// The start data of this window's player (the bytes of `Players::Player`); empty: the default.
    pub player_data: Vec<u8>,
}

/// A window that joined: its bridge, and the seat the server gave it.
pub(crate) struct Joined<G: Game> {
    pub bridge: Threaded<G>,
    pub slot: u8,
    pub players: u8,
    /// Passed back as `--token` (with `--slot`) to take the seat again after the window closed.
    pub token: u64,
}

/// Connects to the server and waits until the game has started. Blocks: with an unfilled room
/// that means until the other players joined.
pub(crate) fn join<G: Game>(join: Join, config: G::Config) -> Result<Joined<G>, String> {
    let (seat_tx, seat_rx) = mpsc::channel();
    let bridge = Threaded::try_spawn(
        move || {
            let link = connect(&ConnectOptions::new(join.addr.clone(), TransportKind::Ws, Trust::InsecureDev))?;
            let mut cfg = RelayClientConfig::new(ROOM, build_id::<G>());
            cfg.want_slot = join.slot.map(PlayerSlot);
            cfg.token = join.token;
            cfg.player_data = join.player_data;
            let mut config = Some(config);
            let client = RelayClient::<G, _>::new(
                cfg,
                link,
                move |_welcome| config.take().expect("the client starts its game once"),
                DirSink::new(PathBuf::from("desync")),
            );
            let options = RelayHostOptions::<G> {
                connect_timeout: WAIT_FOR_PLAYERS,
                on_status: Some(Box::new(|state: &str| eprintln!("[net] {state}")) as StatusFn),
                commands_from_input: None,
                metrics: None,
            };
            let host = RelayHost::connect(client, options).map_err(|e| e.to_string())?;
            let welcome = host.client().welcome().ok_or("the server sent no welcome")?;
            let _ = seat_tx.send((welcome.slot, welcome.player_count, welcome.token));
            Ok(host)
        },
        BridgeConfig::default(),
        ThreadedConfig::default(),
    )?;
    let (slot, players, token) = seat_rx.recv().map_err(|_| "the window did not get a seat".to_string())?;
    Ok(Joined { bridge, slot, players, token })
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use orr_bridge::{Bridge, BridgeEvent, EventStatus, SignalKind};
    use orr_testgame::roster::{Census, Roster, RosterInput};

    use super::*;

    type Seen = Vec<(u64, SignalKind, u8)>;

    fn local_server(players: u8) -> Hosted {
        let bind: SocketAddr = "127.0.0.1:0".parse().unwrap();
        Hosted::start(bind, room_config::<Roster>(players, 60, 7), 7).unwrap()
    }

    fn join_slot(server: &Hosted, slot: u8, token: u64) -> Result<Joined<Roster>, String> {
        join::<Roster>(Join { addr: server.client_address(), slot: Some(slot), token, player_data: Vec::new() }, ())
    }

    /// Plays `window` for `ms` milliseconds of real time and keeps the verified signals it saw.
    fn play(window: &mut Joined<Roster>, ms: u64, seen: &mut Seen) {
        let end = Instant::now() + Duration::from_millis(ms);
        while Instant::now() < end {
            let slot = window.bridge.local_slot();
            let _ = window.bridge.set_input(slot, RosterInput { dx: 1 });
            for event in window.bridge.drain_events() {
                if let BridgeEvent::Player { key, status: EventStatus::Verified(signal) } = event {
                    seen.push((key.tick, signal.kind, signal.slot.0));
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn census(window: &Joined<Roster>) -> Census {
        *window.bridge.snapshot().unwrap().predicted().singleton::<Census>()
    }

    fn tick_of(seen: &Seen, kind: SignalKind, slot: u8) -> Option<u64> {
        seen.iter().find(|s| s.1 == kind && s.2 == slot).map(|s| s.0)
    }

    #[test]
    fn the_build_id_names_the_game() {
        assert_ne!(build_id::<Roster>(), 0);
        assert_ne!(build_id::<Roster>(), build_id::<orr_testgame::Arena>());
    }

    #[test]
    fn a_window_that_cannot_reach_a_server_fails_with_a_message() {
        let nobody = Join { addr: "127.0.0.1:1".to_string(), slot: None, token: 0, player_data: Vec::new() };
        let error = join::<Roster>(nobody, ()).err().unwrap();
        assert!(!error.is_empty());
    }

    #[test]
    fn two_windows_on_a_hosted_server_are_added_and_a_returning_window_is_reconnected() {
        let server = local_server(2);
        // The first window waits for the second one, so it joins from another thread.
        let (mut a, mut b) = std::thread::scope(|scope| {
            let first = scope.spawn(|| join_slot(&server, 0, 0).unwrap());
            let second = join_slot(&server, 1, 0).unwrap();
            (first.join().unwrap(), second)
        });
        assert_eq!((a.slot, b.slot, a.players), (0, 1, 2));
        let (mut seen_a, mut seen_b) = (Seen::new(), Seen::new());
        play(&mut a, 500, &mut seen_a);
        play(&mut b, 100, &mut seen_b);
        assert_eq!(seen_a.iter().filter(|s| s.1 == SignalKind::Added).count(), 2, "{seen_a:?}");
        assert!(seen_a.iter().all(|s| s.0 == 1), "the first players are added in tick 1: {seen_a:?}");

        // Window B closes. A sees the disconnect. B comes back with its token.
        let token = b.token;
        drop(b);
        play(&mut a, 500, &mut seen_a);
        assert!(tick_of(&seen_a, SignalKind::Disconnected, 1).is_some(), "{seen_a:?}");

        let mut b = join_slot(&server, 1, token).unwrap();
        assert_eq!(b.slot, 1);
        let mut seen_b = Seen::new();
        play(&mut a, 600, &mut seen_a);
        play(&mut b, 300, &mut seen_b);
        let back = tick_of(&seen_a, SignalKind::Reconnected, 1).expect("A saw Reconnected");
        assert_eq!(tick_of(&seen_b, SignalKind::Reconnected, 1), Some(back), "B saw it at the same tick: {seen_b:?}");
        assert!(tick_of(&seen_a, SignalKind::Disconnected, 1).unwrap() < back);
        for window in [&a, &b] {
            let c = census(window);
            assert_eq!((c.added, c.disconnected, c.reconnected, c.removed), (2, 1, 1, 0));
        }
    }
}
