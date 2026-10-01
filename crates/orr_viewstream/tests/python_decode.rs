//! `tools/viewstream_client.py` decodes the 3D frame (format version 2) to the
//! same values the Rust encoder wrote. Skips when no `python3` is installed.

use orr_viewstream::*;
use std::process::Command;

#[test]
fn python_decodes_a_3d_frame_like_rust() {
    let frame = ViewFrame3 {
        flags: FLAG_DISCONTINUITY,
        tick: 41,
        verified_tick: 39,
        seq: 8,
        rollback: None,
        entities: vec![
            EntityRecord3 {
                id: 0x0000_0003_0000_0007,
                kind: 2,
                shape: SHAPE3_CAPSULE,
                mode: MODE_PREDICTION,
                size: [0.25, 0.75, 0.0],
                rgba: [10, 20, 30, 255],
                roughness: 128,
                metallic: 255,
                style_flags: STYLE_CHECKER,
                prev: Pose3 { pos: [1.0, 2.0, 3.0], rot: [0.0, 0.0, 0.0, 1.0] },
                cur: Pose3 { pos: [1.5, 2.5, -3.5], rot: [0.0, 1.0, 0.0, 0.0] },
            },
            EntityRecord3 {
                id: 9,
                kind: 0,
                shape: SHAPE3_PLANE,
                mode: MODE_NONE,
                size: [30.0, 0.0, 20.0],
                rgba: [200, 200, 200, 255],
                roughness: 255,
                metallic: 0,
                style_flags: 0,
                prev: Pose3::IDENTITY,
                cur: Pose3::IDENTITY,
            },
        ],
        props: vec![],
    };
    let hex: String = frame.encode().iter().map(|b| format!("{b:02x}")).collect();
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tools/viewstream_client.py");
    let code = format!(
        "import importlib.util, sys\n\
         spec = importlib.util.spec_from_file_location('vsc', r'{script}')\n\
         m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m)\n\
         f = m.decode_message(bytes.fromhex('{hex}'))\n\
         e = f['entities']\n\
         print(f['dimensions'], f['tick'], f['verified_tick'], f['seq'], f['flags'], len(e))\n\
         print(e[0]['index'], e[0]['version'], e[0]['kind'], e[0]['shape'], e[0]['mode'], e[0]['size'], e[0]['rgba'], e[0]['roughness'], e[0]['metallic'], e[0]['checker'])\n\
         print(e[0]['prev'], e[0]['cur'])\n\
         print(e[1]['shape'], e[1]['mode'], e[1]['size'], e[1]['checker'])\n"
    );
    let out = match Command::new("python3").arg("-c").arg(&code).output() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("SKIP: no python3 ({e})");
            return;
        }
    };
    assert!(out.status.success(), "python failed: {}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "3 41 39 8 2 2");
    assert_eq!(lines[1], "7 3 2 capsule prediction (0.25, 0.75, 0.0) (10, 20, 30, 255) 128 255 True");
    assert_eq!(lines[2], "(1.0, 2.0, 3.0, 0.0, 0.0, 0.0, 1.0) (1.5, 2.5, -3.5, 0.0, 1.0, 0.0, 0.0)");
    assert_eq!(lines[3], "plane none (30.0, 0.0, 20.0) False");
}
