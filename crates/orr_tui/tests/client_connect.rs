//! `orr_tui --connect` against hosts in client mode (`orr_remote_host --join`): two hosts (in
//! process, the same code as the binary) play on an in-process `orr_server` with simulated latency
//! and loss; two headless terminal viewers connect to them over WebSocket, play their slots through
//! `sim.input` and print a `RESULT client ...` line each. The confirmed state at a fixed verified
//! tick (`checkpoint=` and `checksum=`) must be the same in both.
#![allow(clippy::disallowed_types)]

use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::channel;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use orr_edit::EditorDoc;
use orr_relay_net::{listen, ListenOptions, SimConditions, TransportKind};
use orr_remote::sample::{join_phys_client, phys_types};
use orr_remote::{Auth, ErpServer, Host, ServerConfig};
use orr_sample::net_client::NetArgs;
use orr_sample::physics_game::PhysGame;
use orr_server::presets::{self, Game, PhysicsScene};
use orr_server::serve::run_wall_clock;
use orr_server::RelayServer;
use orr_sim::Simulation;

struct JoinedHost {
    url: String,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl JoinedHost {
    fn start(args: NetArgs) -> JoinedHost {
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = channel();
        let s = stop.clone();
        let thread = thread::spawn(move || {
            let mut cfg = ServerConfig::new(Auth::DevNoAuth);
            if let Err(e) = join_phys_client(&mut cfg.limits, args) {
                tx.send(Err(e)).unwrap();
                return;
            }
            let doc = EditorDoc::from_yaml("schema: orr.scene/1\nentities: {}\n", phys_types(), Simulation::<PhysGame>::build_registry(), 7).unwrap();
            let server = ErpServer::start(cfg).unwrap();
            tx.send(Ok(server.url())).unwrap();
            Host::<PhysGame>::new(doc, server).run(&s, Duration::from_micros(500));
        });
        let url = rx.recv_timeout(Duration::from_secs(120)).expect("the host did not join").expect("join failed");
        JoinedHost { url, stop, thread: Some(thread) }
    }
}

impl Drop for JoinedHost {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn field(line: &str, name: &str) -> String {
    line.split_whitespace().find_map(|w| w.strip_prefix(&format!("{name}="))).unwrap_or_else(|| panic!("no {name}= in {line}")).to_string()
}

fn number(line: &str, name: &str) -> u64 {
    field(line, name).parse().unwrap_or_else(|_| panic!("{name} is not a number in {line}"))
}

fn args(addr: std::net::SocketAddr, name: &str, latency_ms: u32, loss: f32, seed: u64) -> NetArgs {
    let mut a = NetArgs {
        connect: Some(addr.to_string()),
        kind: TransportKind::Ws,
        name: name.to_string(),
        quiet: true,
        desync_dir: std::env::temp_dir().join("orr_tui_connect_desync"),
        ..NetArgs::default()
    };
    a.sim = SimConditions { latency_ms: latency_ms.into(), jitter_ms: 5, loss, seed };
    a.sim_seed = Some(seed);
    a
}

fn spawn_tui(url: &str) -> Child {
    Command::new(env!("CARGO_BIN_EXE_orr_tui"))
        .args(["--connect", url, "--headless", "--ticks", "480", "--check-tick", "300"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start orr_tui")
}

#[test]
fn two_terminal_viewers_on_client_mode_hosts_agree_on_the_confirmed_state() {
    let ep = listen(&ListenOptions::new("127.0.0.1:0".parse().unwrap(), TransportKind::Ws)).expect("listen");
    let addr = ep.local_addr();
    let mut server = RelayServer::new(ep, 3);
    let scene = PhysicsScene { bodies: 300, mode: 1, ..PhysicsScene::default() };
    server.create_room(1, presets::room_config(Game::Physics, 2, 60, presets::SAMPLE_SEED, scene));
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let server_thread = thread::spawn(move || {
        run_wall_clock(&mut server, &flag, Duration::from_millis(1), |s, _| {
            let _ = s.drain_notes();
        });
        server.room_stats(1).expect("room")
    });

    let (a_args, b_args) = (args(addr, "host-a", 30, 0.0, 21), args(addr, "host-b", 40, 0.02, 22));
    let ta = thread::spawn(move || JoinedHost::start(a_args));
    let tb = thread::spawn(move || JoinedHost::start(b_args));
    let (host_a, host_b) = (ta.join().unwrap(), tb.join().unwrap());

    let mut viewers = [(spawn_tui(&host_a.url), "tui-a"), (spawn_tui(&host_b.url), "tui-b")].map(|(c, t)| (Some(c), t));
    let started = Instant::now();
    let mut results: Vec<Option<String>> = vec![None, None];
    while results.iter().any(Option::is_none) {
        for (i, (child, tag)) in viewers.iter_mut().enumerate() {
            if child.as_mut().is_some_and(|c| c.try_wait().unwrap().is_some()) {
                let out = child.take().unwrap().wait_with_output().unwrap();
                let (stdout, stderr) = (String::from_utf8_lossy(&out.stdout).to_string(), String::from_utf8_lossy(&out.stderr).to_string());
                assert!(out.status.success(), "{tag} failed: {stdout}\n{stderr}");
                results[i] = Some(stdout.lines().find(|l| l.starts_with("RESULT client")).unwrap_or_else(|| panic!("{tag}: no RESULT line in:\n{stdout}\n{stderr}")).to_string());
            }
        }
        if started.elapsed() > Duration::from_secs(240) {
            for c in viewers.iter_mut().filter_map(|(c, _)| c.as_mut()) {
                let _ = c.kill();
            }
            panic!("the viewers did not finish in 240 s");
        }
        thread::sleep(Duration::from_millis(50));
    }
    drop((host_a, host_b));
    stop.store(true, Ordering::Relaxed);
    let room = server_thread.join().unwrap();

    let lines: Vec<String> = results.into_iter().flatten().collect();
    for l in &lines {
        println!("{l}");
    }
    let part = |l: &str| format!("checkpoint={} checksum={}", field(l, "checkpoint"), field(l, "checksum"));
    assert_eq!(part(&lines[0]), part(&lines[1]), "the two viewers disagree on the confirmed state");
    assert_eq!(field(&lines[0], "checkpoint"), "300");
    assert_ne!(field(&lines[0], "checksum"), "0x0000000000000000");
    let mut slots = [number(&lines[0], "slot"), number(&lines[1], "slot")];
    slots.sort_unstable();
    assert_eq!(slots, [0, 1]);
    for l in &lines {
        assert_eq!(number(l, "players"), 2);
        assert!(number(l, "ticks") >= 480);
        assert!(number(l, "rolled_back_frames") > 0, "{l}");
        assert!(number(l, "predicted") > 0 && number(l, "verified") > 0, "{l}");
        assert!(number(l, "rtt_ms") >= 30, "{l}");
    }
    assert_eq!(room.desyncs, 0);
}
