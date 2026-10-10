//! Deployment manifests preserve the exact game identity through native clients.
#![allow(clippy::disallowed_types)]

use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use orr_proto::RejectReason;
use orr_relay_net::{ListenOptions, TransportKind, listen};
use orr_sample::net_client::{NetArgs, run_arena_bot};
use orr_sample::physics_game::TICK_RATE;
use orr_sample::relay_view::RelayView;
use orr_server::RelayServer;
use orr_server::presets::{self, Game, PhysicsScene};
use orr_server::serve::run_wall_clock;
use orr_session::ClientState;

const GAME_CODE_ID: u64 = (1 << 53) + 1;
const OTHER_GAME_CODE_ID: u64 = (1 << 53) + 3;
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

struct Live {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Live {
    fn start(game: Game, build_id: u64) -> Self {
        let endpoint = listen(&ListenOptions::new(
            "127.0.0.1:0".parse().unwrap(),
            TransportKind::Ws,
        ))
        .unwrap();
        let addr = endpoint.local_addr();
        let mut server = RelayServer::new(endpoint, 17);
        let mut room = presets::room_config(
            game,
            1,
            TICK_RATE,
            presets::SAMPLE_SEED,
            PhysicsScene::default(),
        );
        room.build_hash = orr_sim::build_hash_of(build_id, 0);
        room.min_players_to_start = 1;
        server.create_room(1, room);
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let thread = thread::spawn(move || {
            run_wall_clock(&mut server, &flag, Duration::from_millis(1), |s, _| {
                drop(s.drain_notes())
            });
        });
        Self {
            addr,
            stop,
            thread: Some(thread),
        }
    }

    fn stop(&mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn python() -> std::ffi::OsString {
    std::env::var_os("ORR_PYTHON").unwrap_or_else(|| {
        if cfg!(windows) {
            "python".into()
        } else {
            "python3".into()
        }
    })
}

fn manifest_id(game: &str, code_id: u64) -> u64 {
    let dir = loop {
        let candidate = std::env::temp_dir().join(format!(
            "orr-deploy-identity-{}-{game}-{code_id}-{}",
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        match std::fs::create_dir(&candidate) {
            Ok(()) => break candidate,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => panic!("create temporary manifest directory: {error}"),
        }
    };
    let path = dir.join("manifest.json");
    let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let script = repo_root.join("tools/deployment_manifest.py");
    let output = Command::new(python())
        .arg(&script)
        .arg("create")
        .arg("--game")
        .arg(game)
        .arg("--game-code-identity")
        .arg(format!("test-{game}"))
        .arg("--game-code-id")
        .arg(code_id.to_string())
        .arg("--output")
        .arg(&path)
        .current_dir(&repo_root)
        .output()
        .expect("start Python deployment manifest tool");
    assert!(
        output.status.success(),
        "manifest creation failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let id_output = Command::new(python())
        .arg(&script)
        .arg("id")
        .arg(&path)
        .current_dir(&repo_root)
        .output()
        .expect("read deployment manifest id");
    assert!(
        id_output.status.success(),
        "manifest id failed: {}",
        String::from_utf8_lossy(&id_output.stderr)
    );
    let id_text = String::from_utf8(id_output.stdout).unwrap();
    let id = id_text
        .trim()
        .parse()
        .expect("manifest id is an exact decimal u64");
    let json = std::fs::read_to_string(&path).unwrap();
    assert!(
        json.contains(&format!("\"game_code_id\": \"{code_id}\"")),
        "game code id is a canonical string: {json}"
    );
    assert!(
        json.contains(&format!("\"build_id\": \"{id}\"")),
        "derived build id is a canonical string: {json}"
    );
    assert!(json.contains(&format!(
        "\"frame_format_version\": {}",
        orr_ecs::FRAME_FORMAT_VERSION
    )));
    assert_eq!(id, orr_sim::frame_build_id(code_id));
    std::fs::remove_dir_all(dir).unwrap();
    id
}

fn args_for(live: &Live, build_id: u64) -> NetArgs {
    let mut args = NetArgs {
        connect: Some(live.addr.to_string()),
        kind: TransportKind::Ws,
        ..NetArgs::default()
    };
    let build_id_text = build_id.to_string();
    let mut next = |_: &str| Ok(build_id_text.clone());
    assert!(args.parse_option("--build-id", &mut next).unwrap());
    args.connect_timeout = Duration::from_secs(5);
    args.quiet = true;
    args
}

#[test]
fn manifest_id_is_used_by_native_client_for_matching_and_mismatching_rooms() {
    let build_id = manifest_id("arena", GAME_CODE_ID);
    let other_build_id = manifest_id("arena", OTHER_GAME_CODE_ID);

    let mut matching = Live::start(Game::Arena, build_id);
    let report =
        run_arena_bot(&args_for(&matching, build_id), 0.05).expect("matching native client");
    assert_eq!(report.state, ClientState::Playing);
    matching.stop();

    let mut mismatch = Live::start(Game::Arena, build_id);
    let report = run_arena_bot(&args_for(&mismatch, other_build_id), 0.0)
        .expect("mismatching native client report");
    assert_eq!(
        report.state,
        ClientState::Rejected(RejectReason::BuildHashMismatch {
            server: orr_sim::build_hash_of(build_id, 0),
            client: orr_sim::build_hash_of(other_build_id, 0),
        })
    );
    mismatch.stop();
}

#[test]
fn physics_relay_view_schema_uses_the_selected_manifest_id() {
    let build_id = manifest_id("physics", GAME_CODE_ID);
    let mut live = Live::start(Game::Physics, build_id);
    let args = args_for(&live, build_id);
    let (_view, ready) =
        RelayView::join(args).expect("physics RelayView matching manifest identity");
    assert_eq!(ready.schema.build_id, build_id);
    drop(_view);
    live.stop();
}

#[test]
fn explicit_zero_build_id_keeps_the_untracked_native_contract() {
    let mut live = Live::start(Game::Arena, 0);
    let report = run_arena_bot(&args_for(&live, 0), 0.05).expect("untracked native client");
    assert_eq!(report.state, ClientState::Playing);
    live.stop();
}

#[test]
fn net_args_parses_max_u64_and_keeps_explicit_zero() {
    fn parse(value: &str) -> Result<NetArgs, String> {
        let mut args = NetArgs::default();
        let mut next = |_: &str| Ok(value.to_string());
        assert!(args.parse_option("--build-id", &mut next)?);
        Ok(args)
    }
    assert_eq!(
        parse("18446744073709551615").unwrap().build_id,
        Some(u64::MAX)
    );
    assert_eq!(parse("0").unwrap().build_id, Some(0));
    assert!(parse("18446744073709551616").is_err());
}

#[test]
fn manifest_derivation_preserves_u64_max() {
    assert_eq!(
        manifest_id("arena", u64::MAX),
        orr_sim::frame_build_id(u64::MAX)
    );
}

#[test]
fn python_manifest_unit_suite_is_a_mandatory_check() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new(python())
        .arg("-m")
        .arg("unittest")
        .arg("tools.test_deployment_manifest")
        .current_dir(repo_root)
        .output()
        .expect("start Python deployment manifest tests");
    assert!(
        output.status.success(),
        "Python manifest tests failed:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn manifest_derivation_matches_pinned_raw_id_vector() {
    assert_eq!(manifest_id("arena", 1), 2_372_055_272_327_396_894);
}
