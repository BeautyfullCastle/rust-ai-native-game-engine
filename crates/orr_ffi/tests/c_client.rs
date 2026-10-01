//! The C ABI, proven from the outside: a C program (`tests/c/view_client.c`)
//! is compiled against `include/orrery.h`, linked to the built cdylib and run.
//! It plays 120 ticks with two players and prints a checksum of every entity
//! record it read; this test runs the same scenario through the Rust bridge and
//! compares. A second test opens the host with a socket and shows that a
//! WebSocket client receives the same bytes as the C ABI.
//!
//! The C program needs a C compiler (found by the `cc` crate: gcc/clang, or
//! MSVC's `cl` on Windows). Without one the test prints why and is skipped
//! unless `ORR_REQUIRE_C_COMPILER=1` (CI sets it).
#![allow(clippy::disallowed_types)] // tests wait on the wall clock

use std::ffi::{CStr, CString};
use std::process::Command;
use std::time::{Duration, Instant};

use orr_bridge::{Bridge, BridgeConfig, InProc, PlayHost, PlayerSlot};
use orr_ffi::*;
use orr_remote::ErpClient;
use orr_sample::physics_game::{PhysGame, PhysInput, TICK_RATE};
use orr_session::PlaySession;
use orr_viewstream::{ViewFrame, HEADER_LEN, RECORD_LEN};
use serde_json::json;

mod common;

use common::{compile_c, ensure_lib, skip_or_fail};

const DEMO_SCENE: &str = include_str!("../../../scenes/physics_demo.scene.yaml");

// ---- the scenario, as the C program plays it ----

fn scenario_input(p: i32, t: i32) -> PhysInput {
    PhysInput::new((t / 20 + p) % 3 - 1, (t / 30 + 2 * p) % 3 - 1, (t / 25 + p) % 3 - 1, p == 0 && t % 40 < 5)
}

fn fnv(mut h: u64, bytes: &[u8]) -> u64 {
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// The same 120 ticks through the Rust bridge and the view stream source.
fn rust_result() -> (usize, usize, u64) {
    let doc = orr_remote::sample::phys_doc(DEMO_SCENE).unwrap();
    let mut cfg = doc.play_config(2, TICK_RATE);
    cfg.start_paused = false;
    let session = PlaySession::<PhysGame>::from_frame(cfg, doc.frame()).unwrap();
    // Slot 0 is the local player (set before each step); slot 1 comes from the bot, for tick `tick`.
    let host = PlayHost::new(session, PlayerSlot(0)).with_bot(|_slot, tick| scenario_input(1, tick as i32));
    let mut bridge = InProc::new(host, BridgeConfig::default());
    let mut source = orr_sample::physics_stream::phys_stream_source(0, 2);
    let (mut hash, mut frames, mut count) = (0xcbf2_9ce4_8422_2325u64, 0, 0);
    for t in 1..=120 {
        bridge.set_input(PlayerSlot(0), scenario_input(0, t)).unwrap();
        bridge.step(1);
        let frame = source.pump(&mut bridge).frame.expect("one frame per tick");
        let bytes = frame.encode();
        count = frame.entities.len();
        hash = fnv(hash, &frame.tick.to_le_bytes());
        hash = fnv(hash, &bytes[HEADER_LEN..HEADER_LEN + count * RECORD_LEN]);
        frames += 1;
    }
    (count, frames, hash)
}

#[test]
fn c_program_sees_what_the_rust_bridge_sees() {
    let Some(lib) = ensure_lib() else {
        return skip_or_fail("the orr_ffi shared library is not there and `cargo build -p orr_ffi` did not make it");
    };
    let dir = std::env::temp_dir().join(format!("orr_ffi_c_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let exe = match compile_c(&dir, &lib, "view_client") {
        Ok(e) => e,
        Err(why) if why.starts_with("no C compiler") || why.starts_with("cannot run the C compiler") => return skip_or_fail(&why),
        Err(why) => panic!("{why}"),
    };
    let path_var = std::env::var_os("PATH").unwrap_or_default();
    let mut paths = vec![lib.dir.clone()];
    paths.extend(std::env::split_paths(&path_var));
    let out = Command::new(&exe)
        .env("PATH", std::env::join_paths(paths).unwrap())
        .env("LD_LIBRARY_PATH", &lib.dir)
        .env("DYLD_LIBRARY_PATH", &lib.dir)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(out.status.success(), "the C client failed ({:?}):\n{stdout}\n{stderr}", out.status);
    let line = stdout.lines().find(|l| l.starts_with("RESULT")).unwrap_or_else(|| panic!("no RESULT line in:\n{stdout}")).to_string();
    let (count, frames, hash) = rust_result();
    let expected = format!("RESULT entities={count} frames={frames} fnv=0x{hash:016x}");
    println!("C program: {line}\nRust bridge: {expected}");
    assert_eq!(line, expected, "the C client and the Rust bridge disagree");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- the C functions called from Rust: the same error rules, and the socket ----

fn last_error() -> String {
    unsafe { CStr::from_ptr(orr_last_error()) }.to_string_lossy().into_owned()
}

fn wait_for_frame(host: *mut OrrHost, tick: u64) -> Vec<u8> {
    let end = Instant::now() + Duration::from_secs(20);
    let mut buf = vec![0u8; 1 << 16];
    while Instant::now() < end {
        let mut written = 0usize;
        match unsafe { orr_view_poll(host, buf.as_mut_ptr(), buf.len(), &mut written) } {
            ORR_OK => {
                let f = ViewFrame::decode(&buf[..written]).unwrap();
                if f.tick >= tick {
                    return buf[..written].to_vec();
                }
            }
            ORR_NO_FRAME => std::thread::sleep(Duration::from_millis(1)),
            other => panic!("orr_view_poll returned {other}: {}", last_error()),
        }
    }
    panic!("no frame of tick {tick}");
}

#[test]
fn error_paths_return_codes_not_crashes() {
    unsafe {
        let mut written = 99usize;
        assert_eq!(orr_view_poll(std::ptr::null_mut(), std::ptr::null_mut(), 0, &mut written), ORR_ERR_NULL);
        assert!(last_error().contains("null"));
        assert_eq!(orr_set_input(std::ptr::null_mut(), 0, std::ptr::null(), 0), ORR_ERR_NULL);
        assert_eq!(orr_control(std::ptr::null_mut(), ORR_CTL_PLAY, 0), ORR_ERR_NULL);
        assert_eq!(orr_schema_json(std::ptr::null_mut(), std::ptr::null_mut(), 0), 0);
        orr_host_close(std::ptr::null_mut());
        let bad = CString::new("/definitely/not/a/scene.yaml").unwrap();
        assert!(orr_host_open(bad.as_ptr(), std::ptr::null()).is_null());
        assert!(last_error().contains("cannot read scene"));
        let cfg = OrrHostConfig { struct_size: 0, flags: 0, listen_port: 0 };
        assert!(orr_host_open(std::ptr::null(), &cfg).is_null());
        let cfg = OrrHostConfig { struct_size: 12, flags: ORR_HOST_LISTEN, listen_port: 70000 };
        assert!(orr_host_open(std::ptr::null(), &cfg).is_null());
        assert!(last_error().contains("listen_port"));

        let host = orr_host_open(std::ptr::null(), std::ptr::null());
        assert!(!host.is_null(), "{}", last_error());
        // Buffers: too small reports the size and writes nothing.
        let needed = orr_schema_json(host, std::ptr::null_mut(), 0);
        assert!(needed > 100);
        let mut small = [1 as std::ffi::c_char; 4];
        assert_eq!(orr_schema_json(host, small.as_mut_ptr(), small.len()), needed);
        assert_eq!(small[0], 0);
        assert_eq!(orr_view_poll(host, std::ptr::null_mut(), 5, &mut written), ORR_ERR_NULL);
        let f = wait_for_frame(host, 0);
        assert_eq!(&f[..4], b"OVS1");
        // Inputs: wrong length, unknown player, control args.
        let input = [0u8; 24];
        assert_eq!(orr_set_input(host, 0, input.as_ptr(), 23), ORR_ERR_ARG);
        assert_eq!(orr_set_input(host, 2, input.as_ptr(), 24), ORR_ERR_ARG);
        assert_eq!(orr_set_input(host, 1, input.as_ptr(), 24), ORR_OK);
        assert_eq!(orr_control(host, ORR_CTL_STEP, -5), ORR_ERR_ARG);
        assert_eq!(orr_control(host, 1234, 0), ORR_ERR_ARG);
        // Commands: the demo has a 4-byte no-op command.
        let cmd = [0u8; 4];
        assert_eq!(orr_send_command(host, 0, cmd.as_ptr(), 4), ORR_OK);
        assert_eq!(orr_send_command(host, 0, cmd.as_ptr(), 3), ORR_ERR_RPC);
        // ERP in process, and its buffer rule.
        let req = CString::new(r#"{"method":"sim.state"}"#).unwrap();
        let mut needed_answer = 0usize;
        assert_eq!(orr_erp_call(host, req.as_ptr(), std::ptr::null_mut(), 0, &mut needed_answer), ORR_ERR_BUFFER);
        let mut answer = vec![0 as std::ffi::c_char; needed_answer];
        assert_eq!(orr_erp_call(host, req.as_ptr(), answer.as_mut_ptr(), answer.len(), &mut needed_answer), ORR_OK);
        let text = CStr::from_ptr(answer.as_ptr()).to_str().unwrap();
        let v: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(v["result"]["mode"], "play");
        orr_host_close(host);
    }
}

#[test]
fn a_socket_client_receives_the_same_bytes_as_the_c_abi() {
    unsafe {
        let cfg = OrrHostConfig { struct_size: std::mem::size_of::<OrrHostConfig>() as u32, flags: ORR_HOST_LISTEN, listen_port: 0 };
        let host = orr_host_open(std::ptr::null(), &cfg);
        assert!(!host.is_null(), "{}", last_error());
        let mut url = vec![0 as std::ffi::c_char; 128];
        assert!(orr_host_url(host, url.as_mut_ptr(), url.len()) > 0);
        let url = CStr::from_ptr(url.as_ptr()).to_str().unwrap().to_string();
        assert!(url.starts_with("ws://127.0.0.1:"), "{url}");

        // An out-of-process view would do exactly this, over the network.
        let mut remote = ErpClient::connect(&url, None).unwrap();
        remote.call("watch.subscribe", json!({"topics": ["viewstream"], "max_fps": 1000})).unwrap();
        let schema = remote.wait_notification("watch.viewstream.schema", Duration::from_secs(10)).unwrap().expect("the schema comes first");
        assert_eq!(schema["params"]["game"], "PhysGame");
        assert_eq!(schema["params"]["input"]["size"], 24);
        // The C ABI hands out the same schema text.
        let mut buf = vec![0 as std::ffi::c_char; orr_schema_json(host, std::ptr::null_mut(), 0)];
        orr_schema_json(host, buf.as_mut_ptr(), buf.len());
        let ffi_schema: serde_json::Value = serde_json::from_str(CStr::from_ptr(buf.as_ptr()).to_str().unwrap()).unwrap();
        assert_eq!(ffi_schema, schema["params"]);

        assert_eq!(orr_control(host, ORR_CTL_STEP, 5), ORR_OK);
        let via_ffi = wait_for_frame(host, 5);
        let via_socket = loop {
            let bytes = remote.wait_frame(Duration::from_secs(20)).unwrap().expect("a frame over the socket");
            if ViewFrame::decode(&bytes).unwrap().tick == 5 {
                break bytes;
            }
        };
        assert_eq!(via_ffi, via_socket, "the socket carries exactly the bytes the C ABI gives");
        orr_host_close(host);
    }
}
