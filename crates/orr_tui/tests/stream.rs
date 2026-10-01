//! The terminal viewer against a real host: over a socket (WebSocket and plain TCP) and through
//! the C ABI, it must see exactly what the Rust bridge and the C client see.
#![allow(clippy::disallowed_types)]

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use common::{host, rust_result};
use orr_tui::headless::{self, HeadlessOpts};
use orr_tui::input::{encode, Controls};
use orr_tui::schema::ViewSchema;
use orr_tui::source::{Control, Incoming, Session, SocketSource, Source};
use orr_tui::state::ViewState;
use orr_viewstream::ViewFrame;

fn temp_file(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("orr_tui_{}_{name}", std::process::id()))
}

fn headless_result(url: &str, frames: u32, dump: &Path) -> String {
    let mut src = SocketSource::connect(url, None, 1000, Session::Ensure { run: false }).expect("connect");
    let opts = HeadlessOpts { frames, size: (80, 30), dump: Some(dump.to_path_buf()) };
    headless::run(&mut src, &opts).expect("headless run").to_string()
}

#[test]
fn websocket_headless_result_equals_the_rust_bridge() {
    let h = host();
    let dump = temp_file("ws_dump.txt");
    let line = headless_result(h.url().unwrap(), 120, &dump);
    let expected = rust_result(120);
    println!("orr_tui (ws): {line}\nRust bridge : {expected}");
    assert_eq!(line, expected);
    let text = std::fs::read_to_string(&dump).unwrap();
    assert_eq!(text.matches("# frame ").count(), 120, "one grid per frame");
    assert!(text.contains('#') && text.contains('o') && text.contains('='), "walls, bodies and paddles are drawn");
    let _ = std::fs::remove_file(dump);
}

#[test]
fn plain_tcp_headless_result_equals_the_rust_bridge() {
    let h = host();
    let url = h.url().unwrap().replacen("ws://", "tcp://", 1);
    let dump = temp_file("tcp_dump.txt");
    assert_eq!(headless_result(&url, 120, &dump), rust_result(120));
    let _ = std::fs::remove_file(dump);
}

/// The x of the paddle of `slot` in a frame, found through the schema (kind name, `slot` property).
fn paddle_x(schema: &ViewSchema, f: &ViewFrame, slot: u32) -> f32 {
    let props = f.props_by_entity(|k| schema.props_words(k)).expect("props add up");
    f.entities
        .iter()
        .zip(props)
        .find(|(e, p)| {
            schema.kind_name(e.kind) == "paddle" && schema.prop_index(e.kind, "slot").is_some_and(|i| u32::from_le_bytes(p[i * 4..i * 4 + 4].try_into().unwrap()) == slot)
        })
        .map(|(e, _)| e.cur[0])
        .expect("a paddle of that slot")
}

/// Holds `x` on player 0 for `ticks` and returns the paddle x at the start and at the end.
fn paddle_run(x: i8, ticks: u64) -> (f32, f32) {
    let h = host();
    let mut src = SocketSource::connect(h.url().unwrap(), None, 1000, Session::Ensure { run: false }).unwrap();
    let schema = ViewSchema::parse(src.schema_text()).unwrap();
    let mut state = ViewState::new(schema.clone(), Instant::now());
    fn wait(src: &mut SocketSource, state: &mut ViewState, tick: u64) {
        let end = Instant::now() + Duration::from_secs(20);
        while Instant::now() < end {
            if let Some(m) = src.recv(Duration::from_millis(50)).unwrap() {
                assert!(!matches!(m, Incoming::Error(_)), "{m:?}");
                state.ingest(&m, Instant::now());
                if state.frame.as_ref().is_some_and(|f| f.tick >= tick) {
                    return;
                }
            }
        }
        panic!("no frame of tick {tick}");
    }
    wait(&mut src, &mut state, 0);
    let start = paddle_x(&schema, state.frame.as_ref().unwrap(), 0);
    // The input bytes come from the schema's layout, not from a Rust struct.
    src.set_input(0, &encode(&schema, &Controls { x, ..Controls::default() })).unwrap();
    src.control(Control::Step(ticks as u32)).unwrap();
    wait(&mut src, &mut state, ticks);
    (start, paddle_x(&schema, state.frame.as_ref().unwrap(), 0))
}

#[test]
fn input_built_from_the_schema_layout_moves_the_paddle() {
    let (start, right) = paddle_run(1, 30);
    let (_, left) = paddle_run(-1, 30);
    println!("paddle x: start {start}, after 30 ticks holding right {right}, holding left {left}");
    assert!(right > start + 0.5, "holding right moves the paddle right ({start} -> {right})");
    assert!(left < start - 0.5, "holding left moves it left ({start} -> {left})");
}

#[test]
fn a_refused_call_comes_back_as_an_error_message() {
    let h = host();
    let mut src = SocketSource::connect(h.url().unwrap(), None, 1000, Session::Ensure { run: false }).unwrap();
    // A wrong input size: the host refuses, the viewer is told (and does not crash).
    src.set_input(0, &[1, 2, 3]).unwrap();
    let end = Instant::now() + Duration::from_secs(10);
    while Instant::now() < end {
        if let Some(Incoming::Error(e)) = src.recv(Duration::from_millis(50)).unwrap() {
            assert!(!e.is_empty());
            return;
        }
    }
    panic!("no error came back");
}

// ---- through the C ABI, loaded at run time ----

#[cfg(all(feature = "ffi", target_os = "linux"))]
mod ffi {
    use super::*;

    /// `target/<profile>`, found from this test binary (`target/<profile>/deps/<test>`).
    fn profile_dir() -> PathBuf {
        std::env::current_exe().unwrap().parent().and_then(Path::parent).unwrap().to_path_buf()
    }

    /// `cargo test` does not build the cdylib of `orr_ffi`: build it (same profile and target dir).
    fn ensure_lib() -> PathBuf {
        let lib = profile_dir().join(orr_tui::ffi::lib_file_name());
        if !lib.exists() {
            let mut cmd = Command::new(env!("CARGO"));
            cmd.args(["build", "-p", "orr_ffi", "--lib"]);
            if profile_dir().file_name().is_some_and(|n| n == "release") {
                cmd.arg("--release");
            }
            assert!(cmd.status().unwrap().success(), "cargo build -p orr_ffi failed");
        }
        assert!(lib.exists(), "{} was not built", lib.display());
        lib
    }

    #[test]
    fn ffi_headless_result_equals_the_rust_bridge() {
        let lib = ensure_lib();
        let dump = temp_file("ffi_dump.txt");
        let mut src = orr_tui::ffi::FfiSource::open(&lib, None).expect("open the host through the C ABI");
        let opts = HeadlessOpts { frames: 120, size: (80, 30), dump: Some(dump.clone()) };
        let line = headless::run(&mut src, &opts).unwrap().to_string();
        let expected = rust_result(120);
        println!("orr_tui (ffi): {line}\nRust bridge  : {expected}");
        assert_eq!(line, expected);
        drop(src);
        let _ = std::fs::remove_file(dump);
    }

    #[test]
    fn the_binary_with_ffi_prints_the_same_result_line() {
        let lib = ensure_lib();
        let out = Command::new(env!("CARGO_BIN_EXE_orr_tui"))
            .args(["--ffi", "--headless", "--frames", "120", "--lib"])
            .arg(&lib)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(out.status.success(), "{stdout}\n{}", String::from_utf8_lossy(&out.stderr));
        assert_eq!(stdout.lines().last().unwrap(), rust_result(120));
    }
}

#[test]
fn the_binary_over_a_socket_prints_the_same_result_line() {
    let h = host();
    let dump = temp_file("bin_dump.txt");
    let out = Command::new(env!("CARGO_BIN_EXE_orr_tui"))
        .args(["--connect", h.url().unwrap(), "--headless", "--frames", "120", "--dump"])
        .arg(&dump)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{stdout}\n{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(stdout.lines().last().unwrap(), rust_result(120));
    assert!(dump.exists());
    let _ = std::fs::remove_file(dump);
}

#[test]
fn bad_arguments_fail_with_a_message() {
    let out = Command::new(env!("CARGO_BIN_EXE_orr_tui")).args(["--headless"]).output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("exactly one of"));
    let out = Command::new(env!("CARGO_BIN_EXE_orr_tui")).args(["--connect", "ws://127.0.0.1:1", "--headless"]).output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("cannot connect"));
}
