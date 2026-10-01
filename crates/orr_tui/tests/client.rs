//! `orr_tui --server`: two terminal viewers, as two processes, join a relay game through the C
//! ABI, play with simulated latency and loss, and print a `RESULT client ...` line each. The
//! confirmed state at a fixed verified tick (the `checkpoint=` and `checksum=` parts) must be the
//! same in both, and the lines show that prediction happened (rollbacks, events settled as
//! verified or canceled).
//!
//! The wait is progress based: the viewers stop by themselves when their game is played and fail
//! when no frame arrives for 30 s; the test only has a generous ceiling.
#![allow(clippy::disallowed_types)]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use orr_relay_net::{format_fingerprint, listen, ListenOptions, TransportKind};
use orr_server::presets::{self, Game, PhysicsScene};
use orr_server::serve::run_wall_clock;
use orr_server::RelayServer;

fn profile_dir() -> PathBuf {
    std::env::current_exe().unwrap().parent().and_then(Path::parent).unwrap().to_path_buf()
}

/// `cargo test` does not build the cdylib of `orr_ffi`: build it (same profile and target dir).
fn ensure_lib() -> PathBuf {
    let lib = profile_dir().join(orr_tui::ffi::lib_file_name());
    // Always build: a no-op when current, and a library left by an earlier run never hides a change.
    let mut cmd = Command::new(env!("CARGO"));
    cmd.args(["build", "-p", "orr_ffi", "--lib"]);
    if profile_dir().file_name().is_some_and(|n| n == "release") {
        cmd.arg("--release");
    }
    assert!(cmd.status().unwrap().success(), "cargo build -p orr_ffi failed");
    assert!(lib.exists(), "{} was not built", lib.display());
    lib
}

fn field(line: &str, name: &str) -> String {
    line.split_whitespace().find_map(|w| w.strip_prefix(&format!("{name}="))).unwrap_or_else(|| panic!("no {name}= in {line}")).to_string()
}

fn number(line: &str, name: &str) -> u64 {
    field(line, name).parse().unwrap_or_else(|_| panic!("{name} is not a number in {line}"))
}

struct Viewer {
    child: Option<Child>,
    tag: &'static str,
}

fn spawn_viewer(lib: &Path, addr: &str, fingerprint: &str, tag: &'static str, latency: u32, loss_percent: &str, seed: u32) -> Viewer {
    let child = Command::new(env!("CARGO_BIN_EXE_orr_tui"))
        .args(["--server", addr, "--fingerprint", fingerprint, "--headless", "--ticks", "480", "--check-tick", "300"])
        .args(["--sim-latency", &latency.to_string(), "--sim-jitter", "5", "--sim-loss", loss_percent, "--sim-seed", &seed.to_string()])
        .arg("--lib")
        .arg(lib)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start orr_tui");
    Viewer { child: Some(child), tag }
}

#[test]
fn two_terminal_viewers_agree_on_the_confirmed_state() {
    let lib = ensure_lib();
    let ep = listen(&ListenOptions::new("127.0.0.1:0".parse().unwrap(), TransportKind::Quic)).expect("listen");
    let (addr, fp) = (ep.local_addr().to_string(), format_fingerprint(&ep.cert_sha256().expect("self-signed")));
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

    // About 70 and 90 ms of round trip, some loss on the second.
    let mut viewers = [
        spawn_viewer(&lib, &addr, &fp, "tui-a", 30, "0", 21),
        spawn_viewer(&lib, &addr, &fp, "tui-b", 40, "2", 22),
    ];
    let started = Instant::now();
    let mut results: Vec<Option<String>> = vec![None, None];
    while results.iter().any(Option::is_none) {
        for (i, v) in viewers.iter_mut().enumerate() {
            if v.child.as_mut().is_some_and(|c| c.try_wait().unwrap().is_some()) {
                let out = v.child.take().unwrap().wait_with_output().unwrap();
                let (stdout, stderr) = (String::from_utf8_lossy(&out.stdout).to_string(), String::from_utf8_lossy(&out.stderr).to_string());
                assert!(out.status.success(), "{} failed: {stdout}\n{stderr}", v.tag);
                results[i] = Some(stdout.lines().find(|l| l.starts_with("RESULT client")).unwrap_or_else(|| panic!("{}: no RESULT line in:\n{stdout}\n{stderr}", v.tag)).to_string());
            }
        }
        if started.elapsed() > Duration::from_secs(240) {
            for v in viewers.iter_mut().filter_map(|v| v.child.as_mut()) {
                let _ = v.kill();
            }
            panic!("the viewers did not finish in 240 s");
        }
        thread::sleep(Duration::from_millis(50));
    }
    stop.store(true, Ordering::Relaxed);
    let room = server_thread.join().unwrap();

    let lines: Vec<String> = results.into_iter().flatten().collect();
    for l in &lines {
        println!("{l}");
    }
    // The part about the confirmed state is identical.
    let part = |l: &str| format!("checkpoint={} checksum={}", field(l, "checkpoint"), field(l, "checksum"));
    assert_eq!(part(&lines[0]), part(&lines[1]), "the two viewers disagree on the confirmed state");
    assert_eq!(field(&lines[0], "checkpoint"), "300");
    assert_ne!(field(&lines[0], "checksum"), "0x0000000000000000");
    // They played different slots of the same two-player room.
    let mut slots = [number(&lines[0], "slot"), number(&lines[1], "slot")];
    slots.sort_unstable();
    assert_eq!(slots, [0, 1]);
    for l in &lines {
        assert_eq!(number(l, "players"), 2);
        assert!(number(l, "ticks") >= 480);
        // Prediction reached the view: rollbacks, and events that were settled one way or the other.
        assert!(number(l, "rolled_back_frames") > 0, "{l}");
        assert!(number(l, "max_depth") >= 1, "{l}");
        assert!(number(l, "predicted") > 0 && number(l, "verified") > 0, "{l}");
        assert!(number(l, "rtt_ms") >= 30, "{l}");
    }
    assert_eq!(room.desyncs, 0);
}

#[test]
fn joining_nothing_fails_with_a_message() {
    let lib = ensure_lib();
    let out = Command::new(env!("CARGO_BIN_EXE_orr_tui"))
        .args(["--server", "127.0.0.1:9", "--insecure", "--connect-timeout", "2", "--headless"])
        .arg("--lib")
        .arg(&lib)
        .output()
        .unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("could not join"), "{err}");
    // QUIC without a way to trust the server is refused before anything is sent.
    let out = Command::new(env!("CARGO_BIN_EXE_orr_tui")).args(["--server", "127.0.0.1:9", "--headless"]).output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("--fingerprint"));
}
