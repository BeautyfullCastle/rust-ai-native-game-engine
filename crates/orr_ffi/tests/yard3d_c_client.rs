//! Yard3D from a compiled C consumer of the public header and shared library.
//! The C process validates the v2 wire contract, drives deterministic inputs,
//! exercises timeline controls, and hashes the raw records and properties. This test builds
//! the same session with the existing Yard3D document factory and compares
//! the C record stream and ERP checksum to an independent Rust simulation.
#![allow(clippy::disallowed_types)] // The C process is a real child process.

use std::path::Path;
use std::process::Command;

use orr_sample::yard3d_game::{NoCommand, Yard3D, YardConfig, YardInput};
use orr_sample::yard3d_stream::Yard3dStreamProducer;
use orr_sim::{PlayerSlot, TickInputs};
use orr_viewstream::{FrameMeta, StreamProducer, HEADER_LEN, RECORD3D_LEN};

mod common;

use common::{compile_c, ensure_lib, skip_or_fail};

const TICKS: u64 = 16;

fn input(player: u32, tick: u32) -> YardInput {
    let buttons = match (player, tick % 4) {
        (0, 0) => orr_sample::yard3d_game::SPAWN_BALL,
        (1, 1) => orr_sample::yard3d_game::SHOOT,
        _ => 0,
    };
    YardInput {
        buttons,
        _pad: 0,
        origin: [0, 1000, 2000],
        dir: [0, -447, -894],
    }
}

fn fnv(mut h: u64, bytes: &[u8]) -> u64 {
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

fn save_evidence(dir: &Path, exe: &Path, lib_dir: &Path) {
    std::fs::create_dir_all(dir).expect("create C ABI evidence directory");
    let shared_name = if cfg!(windows) {
        "orr_ffi.dll"
    } else if cfg!(target_os = "macos") {
        "liborr_ffi.dylib"
    } else {
        "liborr_ffi.so"
    };
    let artifacts = [
        ("yard3d_view_client", exe.to_path_buf()),
        (
            "orrery.h",
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("include/orrery.h"),
        ),
        (shared_name, lib_dir.join(shared_name)),
    ];
    let mut manifest = String::new();
    for (name, source) in artifacts {
        let target = dir.join(name);
        std::fs::copy(&source, &target)
            .unwrap_or_else(|e| panic!("copy evidence {}: {e}", source.display()));
        let size = std::fs::metadata(&target).unwrap().len();
        manifest.push_str(&format!("{name}\t{size}\t{}\n", source.display()));
    }
    std::fs::write(dir.join("manifest.tsv"), manifest).expect("write evidence manifest");
}

/// Replays the exact two-player raw-input sequence sent by C against the same
/// built-in Yard3D document, then hashes tick + unmodified record/property bytes.
fn rust_result() -> (usize, u64, u64) {
    assert_eq!(
        orr_sample::yard3d_stream::yard3d_view_schema(0, 2).dimensions,
        3
    );
    let doc =
        orr_remote::yard3d::yard3d_doc(YardConfig::new(24)).expect("built-in Yard3D document");
    let mut cfg = doc.play_config(2, orr_remote::yard3d::YARD_TICK_RATE);
    cfg.game_id = "Yard3D".into();
    cfg.build_id = orr_remote::yard3d::YARD_BUILD_ID;
    let mut sim =
        orr_sim::Simulation::<Yard3D>::from_frame(doc.frame(), cfg.tick_rate, cfg.build_id)
            .expect("baked Yard3D frame");
    let mut producer = Yard3dStreamProducer::new(orr_remote::yard3d::YARD_BUILD_ID, 2);
    let mut previous = doc.frame().clone();
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    let mut count = 0;
    for t in 1..=TICKS {
        let mut inputs = TickInputs::<YardInput, NoCommand>::new(t, 2);
        inputs.set_input(PlayerSlot(0), input(0, t as u32));
        inputs.set_input(PlayerSlot(1), input(1, t as u32));
        sim.step(&inputs);
        let bytes = producer.encode_frame(
            sim.frame(),
            Some(&previous),
            FrameMeta {
                tick: t,
                verified_tick: t,
                ..FrameMeta::default()
            },
        );
        assert_eq!(&bytes[..4], b"OVS1");
        assert_eq!(u16::from_le_bytes([bytes[4], bytes[5]]), 2);
        assert_eq!(bytes[6], 3);
        count = u32::from_le_bytes(bytes[48..52].try_into().unwrap()) as usize;
        let props = u32::from_le_bytes(bytes[52..56].try_into().unwrap()) as usize;
        assert_eq!(bytes.len(), HEADER_LEN + count * RECORD3D_LEN + props);
        hash = fnv(hash, &bytes[8..16]);
        hash = fnv(hash, &bytes[HEADER_LEN..]);
        previous = sim.frame().clone();
    }
    (count, hash, sim.checksum())
}

#[test]
fn compiled_c_client_reads_and_drives_yard3d_v2() {
    let lib = ensure_lib();
    let dir = std::env::temp_dir().join(format!("orr_ffi_yard3d_c_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let exe = match compile_c(&dir, &lib, "yard3d_view_client") {
        Ok(e) => e,
        Err(why)
            if why.starts_with("no C compiler") || why.starts_with("cannot run the C compiler") =>
        {
            return skip_or_fail(&why)
        }
        Err(why) => panic!("{why}"),
    };
    let evidence_dir = std::env::var_os("ORR_YARD3D_C_EVIDENCE_DIR").map(std::path::PathBuf::from);
    if let Some(path) = &evidence_dir {
        save_evidence(path, &exe, &lib.dir);
    }
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
    if let Some(path) = &evidence_dir {
        std::fs::write(path.join("stdout.txt"), &stdout).expect("save C stdout");
        std::fs::write(path.join("stderr.txt"), &stderr).expect("save C stderr");
    }
    assert!(
        out.status.success(),
        "the Yard3D C client failed ({:?}):\n{stdout}\n{stderr}",
        out.status
    );
    let line = stdout
        .lines()
        .find(|l| l.starts_with("RESULT"))
        .unwrap_or_else(|| panic!("no RESULT line in:\n{stdout}"));
    let (count, hash, checksum) = rust_result();
    let expected = format!("RESULT records={count} fnv=0x{hash:016x} checksum=0x{checksum:016x}");
    println!("C Yard3D: {line}\nRust Yard3D: {expected}");
    assert_eq!(
        line, expected,
        "C raw record/property stream or ERP replay checksum differs from Rust Yard3D"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
