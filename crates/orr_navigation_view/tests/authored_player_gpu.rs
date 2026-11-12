//! Actual offscreen player process/readback. Software Vulkan is permitted and
//! reported explicitly; this is not physical-GPU, window-input or sandbox proof.
#![cfg(all(target_os = "linux", feature = "authored", feature = "gpu"))]
use orr_navigation_view::authored::AuthoredPlayer;
use serde_json::Value;
use std::{fs, io::BufReader, path::Path, process::Command};
const SCENE: &str = include_str!("../../../scenes/navigation_point.scene.yaml");
const TERRAIN: &[u8] = include_bytes!("../../../scenes/navigation/point_demo.orrt");
fn capture(scene: &Path, output: &Path, ticks: u64, checksum: u64) -> (Value, Vec<u8>) {
    let process = Command::new(env!("CARGO_BIN_EXE_navigation_player"))
        .arg("--scene")
        .arg(scene)
        .arg("--ticks")
        .arg(ticks.to_string())
        .arg("--restart-check")
        .arg("--capture")
        .arg(output)
        .output()
        .unwrap();
    assert!(
        process.status.success(),
        "player failed: stdout={} stderr={}",
        String::from_utf8_lossy(&process.stdout),
        String::from_utf8_lossy(&process.stderr)
    );
    let receipt: Value = serde_json::from_slice(&process.stdout).unwrap();
    assert_eq!(receipt["tick"].as_u64(), Some(ticks));
    assert_eq!(receipt["checksum"].as_u64(), Some(checksum));
    assert_eq!(receipt["restart_checked"], true);
    let evidence = &receipt["capture"];
    assert!(!evidence["adapter"].as_str().unwrap().trim().is_empty());
    assert!(evidence["software"].is_boolean());
    assert_eq!(evidence["offscreen"], true);
    assert_eq!(evidence["width"], 1000);
    assert_eq!(evidence["height"], 750);
    assert_eq!(evidence["rgba_bytes"], 1000 * 750 * 4);
    eprintln!(
        "actual navigation capture adapter={} software={} tick={ticks}; child stderr={}",
        evidence["adapter"],
        evidence["software"],
        String::from_utf8_lossy(&process.stderr)
    );
    let mut reader = png::Decoder::new(BufReader::new(fs::File::open(output).unwrap()))
        .read_info()
        .unwrap();
    assert_eq!((reader.info().width, reader.info().height), (1000, 750));
    let size = reader.output_buffer_size().unwrap();
    assert_eq!(size, 1000 * 750 * 4);
    let mut pixels = vec![0; size];
    let info = reader.next_frame(&mut pixels).unwrap();
    assert_eq!(info.color_type, png::ColorType::Rgba);
    assert_eq!(info.bit_depth, png::BitDepth::Eight);
    assert_eq!(info.buffer_size(), pixels.len());
    assert_eq!(
        evidence["png_bytes"].as_u64(),
        Some(fs::metadata(output).unwrap().len())
    );
    (receipt, pixels)
}
fn features(pixels: &[u8]) -> (u64, u64, u64) {
    let (mut orange, mut sum_x, mut sum_y, mut green, mut cyan) =
        (0u64, 0u64, 0u64, 0usize, 0usize);
    for (index, p) in pixels.chunks_exact(4).enumerate() {
        assert_eq!(p[3], 255);
        if p[0] > 180
            && p[1] > 40
            && p[1] < 210
            && p[2] < 150
            && u16::from(p[0]) > u16::from(p[1]) + 30
        {
            orange += 1;
            sum_x += (index % 1000) as u64;
            sum_y += (index / 1000) as u64;
        }
        if p[1] > 70 && u16::from(p[1]) > u16::from(p[0]) + 20 && p[1] > p[2] {
            green += 1;
        }
        if p[2] > 160 && p[1] > 130 && u16::from(p[1]) > u16::from(p[0]) + 30 {
            cyan += 1;
        }
    }
    assert!(green > 1000, "missing rendered terrain: {green} pixels");
    assert!(cyan > 10, "missing rendered path: {cyan} pixels");
    assert!(orange > 30, "missing rendered agent: {orange} pixels");
    (orange, sum_x, sum_y)
}
#[test]
#[ignore = "requires actual offscreen GPU/Vulkan adapter; software adapter allowed and reported"]
fn actual_authored_player_capture_moves_agent_and_preserves_existing_output() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    fs::create_dir_all(root.join("navigation")).unwrap();
    fs::write(root.join("scene.yaml"), SCENE).unwrap();
    fs::write(root.join("navigation/point_demo.orrt"), TERRAIN).unwrap();
    let mut oracle = AuthoredPlayer::open(&root.join("scene.yaml")).unwrap();
    let initial = oracle.checksum();
    // Actual ordinary-directory relocation, not a hidden-mount/bwrap claim.
    let moved = temp.path().join("relocated");
    fs::rename(&root, &moved).unwrap();
    let scene = moved.join("scene.yaml");
    let first = temp.path().join("tick0.png");
    let (second_receipt, pixels0) = capture(&scene, &first, 0, initial);
    let agent0 = features(&pixels0);
    oracle.step().unwrap();
    let expected = oracle.checksum();
    let second = temp.path().join("tick1.png");
    let (receipt1, pixels1) = capture(&scene, &second, 1, expected);
    let agent1 = features(&pixels1);
    assert_ne!(second_receipt["position_raw"], receipt1["position_raw"]);
    assert_ne!(pixels0, pixels1);
    let denominator = i128::from(agent0.0) * i128::from(agent1.0);
    let dx = (i128::from(agent0.1) * i128::from(agent1.0)
        - i128::from(agent1.1) * i128::from(agent0.0))
    .abs();
    let dy = (i128::from(agent0.2) * i128::from(agent1.0)
        - i128::from(agent1.2) * i128::from(agent0.0))
    .abs();
    assert!(
        dx + dy > denominator * 2,
        "orange centroid did not move by more than two pixels"
    );
    // Existing output refusal must preserve its exact bytes, not merely return failure.
    let sentinel = temp.path().join("existing.png");
    fs::write(&sentinel, b"existing foreign output").unwrap();
    let failure = Command::new(env!("CARGO_BIN_EXE_navigation_player"))
        .arg("--scene")
        .arg(&scene)
        .arg("--ticks")
        .arg("1")
        .arg("--capture")
        .arg(&sentinel)
        .output()
        .unwrap();
    assert!(!failure.status.success());
    assert!(String::from_utf8_lossy(&failure.stderr).contains("already exists"));
    assert_eq!(fs::read(&sentinel).unwrap(), b"existing foreign output");
    // The already admitted CPU oracle can restart after files disappear. This
    // does not assert a fresh process can reopen absent authoring sources.
    fs::rename(moved, temp.path().join("hidden-authoring-inputs")).unwrap();
    oracle.restart().unwrap();
    assert_eq!(oracle.checksum(), initial);
    oracle.step().unwrap();
    assert_eq!(oracle.checksum(), expected);
}
