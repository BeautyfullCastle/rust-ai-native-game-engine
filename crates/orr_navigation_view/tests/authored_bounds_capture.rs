//! Actual authored CLI/offscreen acceptance, not window, physical input or hardware-only proof.
#![cfg(all(target_os = "linux", feature = "authored", feature = "gpu"))]
use orr_fp::FP;
use orr_navigation::{AgentProfile, TerrainGraph};
use orr_navigation_view::{authored::AuthoredPlayer, gpu, PathDisplay};
use orr_remote::navigation_yard3d::{NavigationAgentSpec, NavigationScenePin};
use orr_render::orr_rhi::{Rhi, Wgpu, WgpuOptions};
use orr_terrain::Terrain;
use serde_json::Value;
use std::{
    fs,
    io::BufReader,
    path::{Path, PathBuf},
    process::Command,
};
const SIZE: (u32, u32) = (1000, 750);
fn fixture(root: &Path, origin: [i32; 2], spacing: i32, height: i32) -> PathBuf {
    fs::create_dir(root).unwrap();
    let terrain = Terrain::new(
        "bounds/authored".into(),
        9,
        9,
        origin.map(FP::from_int),
        FP::from_int(spacing),
        vec![FP::from_int(height); 81],
        vec![false; 64],
    )
    .unwrap();
    let profile = AgentProfile {
        max_slope: FP::ONE,
        radius: FP::ZERO,
        headroom: FP::ZERO,
        max_step: FP::ZERO,
    };
    let graph = TerrainGraph::build(&terrain, profile).unwrap();
    let spec = NavigationAgentSpec {
        start: [
            FP::from_int(origin[0] + 2 * spacing),
            FP::from_int(origin[1] + 6 * spacing),
        ],
        goal: [
            FP::from_int(origin[0] + 6 * spacing),
            FP::from_int(origin[1] + 2 * spacing),
        ],
        max_slope: FP::ONE,
        distance_per_tick: FP::from_raw(i64::from(spacing) * 16384),
    };
    let pin = NavigationScenePin::new("authored.orrt", &terrain, &graph, spec).unwrap();
    fs::write(root.join("authored.orrt"), terrain.cook()).unwrap();
    // Arrays are emitted from the actual pin, including full admitted revisions.
    // Integer authored coordinates are exact YAML fixed-point values.
    let text=format!("schema: orr.scene/1\nentities: {{}}\nsingletons:\n  NavigationScenePin:\n    source_len: {}\n    identity_len: {}\n    source_path: {:?}\n    asset_id: {:?}\n    terrain_revision: {:?}\n    graph_revision: {:?}\n    agent:\n      start: [{}, {}]\n      goal: [{}, {}]\n      max_slope: 1\n      distance_per_tick: {}\n",pin.source_len,pin.identity_len,pin.source_path.as_slice(),pin.asset_id.as_slice(),pin.terrain_revision,pin.graph_revision,origin[0]+2*spacing,origin[1]+6*spacing,origin[0]+6*spacing,origin[1]+2*spacing,if spacing==1 {"0.25"} else {"2"});
    let path = root.join("scene.yaml");
    fs::write(&path, text).unwrap();
    AuthoredPlayer::open(&path).unwrap();
    path
}
#[derive(Debug)]
struct Features {
    green: usize,
    cyan: usize,
    orange: u64,
    x: u64,
    y: u64,
    bounds: [usize; 4],
    border: usize,
}
fn features(pixels: &[u8]) -> Features {
    assert_eq!(pixels.len(), (SIZE.0 * SIZE.1 * 4) as usize);
    let mut f = Features {
        green: 0,
        cyan: 0,
        orange: 0,
        x: 0,
        y: 0,
        bounds: [1000, 750, 0, 0],
        border: 0,
    };
    for (i, p) in pixels.chunks_exact(4).enumerate() {
        assert_eq!(p[3], 255);
        let (x, y) = (i % 1000, i / 1000);
        let green = p[1] > 110
            && u16::from(p[1]) > u16::from(p[0]) + 30
            && u16::from(p[1]) > u16::from(p[2]) + 20;
        let cyan = p[0] < 100 && p[1] > 160 && p[2] > 220;
        let orange = p[0] > 220 && p[1] > 60 && p[1] < 120 && p[2] < 50;
        if green {
            f.green += 1;
            f.bounds[0] = f.bounds[0].min(x);
            f.bounds[1] = f.bounds[1].min(y);
            f.bounds[2] = f.bounds[2].max(x);
            f.bounds[3] = f.bounds[3].max(y);
        }
        if cyan {
            f.cyan += 1;
        }
        if orange {
            f.orange += 1;
            f.x += x as u64;
            f.y += y as u64;
        }
        if (!(5..995).contains(&x) || !(5..745).contains(&y)) && (green || cyan || orange) {
            f.border += 1;
        }
    }
    f
}
fn framed(f: &Features) -> bool {
    f.green > 20000
        && f.cyan > 10
        && f.orange > 30
        && f.border == 0
        && f.bounds[0] > 5
        && f.bounds[1] > 5
        && f.bounds[2] < 995
        && f.bounds[3] < 745
        && f.bounds[2].saturating_sub(f.bounds[0]) > 300
        && f.bounds[3].saturating_sub(f.bounds[1]) > 300
}
fn capture(scene: &Path, output: &Path, ticks: u64, oracle: &AuthoredPlayer) -> (Value, Vec<u8>) {
    let child = Command::new(env!("CARGO_BIN_EXE_navigation_player"))
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
        child.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&child.stdout),
        String::from_utf8_lossy(&child.stderr)
    );
    let receipt: Value = serde_json::from_slice(&child.stdout).unwrap();
    assert_eq!(receipt["tick"].as_u64(), Some(ticks));
    assert_eq!(receipt["checksum"].as_u64(), Some(oracle.checksum()));
    assert_eq!(receipt["restart_checked"], true);
    assert_eq!(receipt["capture"]["offscreen"], true);
    assert_eq!(receipt["capture"]["width"], 1000);
    assert_eq!(receipt["capture"]["height"], 750);
    assert!(receipt["capture"]["software"].is_boolean());
    assert!(!receipt["capture"]["adapter"].as_str().unwrap().is_empty());
    eprintln!("bounds capture {}", receipt);
    let mut decoder = png::Decoder::new(BufReader::new(fs::File::open(output).unwrap()))
        .read_info()
        .unwrap();
    assert_eq!((decoder.info().width, decoder.info().height), SIZE);
    let mut pixels = vec![0; decoder.output_buffer_size().unwrap()];
    let info = decoder.next_frame(&mut pixels).unwrap();
    assert_eq!(info.color_type, png::ColorType::Rgba);
    assert_eq!(info.bit_depth, png::BitDepth::Eight);
    assert_eq!(info.buffer_size(), pixels.len());
    (receipt, pixels)
}
#[test]
#[ignore = "requires actual GPU/Vulkan CLI captures; software adapter allowed and reported, no silent skip"]
fn moved_scaled_high_authored_terrain_is_framed_without_changing_simulation() {
    let temp = tempfile::tempdir().unwrap();
    let output = std::env::var_os("ORR_NAV_BOUNDS_CAPTURE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| temp.path().join("captures"));
    fs::create_dir(&output).expect("capture evidence directory must be new");
    let device =
        Wgpu::headless(WgpuOptions::default()).expect("actual negative-control GPU required");
    eprintln!(
        "negative-control adapter={} software={}",
        device.adapter_name(),
        device.is_software()
    );
    for (name, origin, spacing, height) in [
        ("moved", [128, -96], 1, 0),
        ("scaled", [-32, -32], 8, 0),
        ("high", [128, -96], 8, 1024),
    ] {
        let scene = fixture(&temp.path().join(name), origin, spacing, height);
        let scene_before = fs::read(&scene).unwrap();
        let terrain_path = scene.parent().unwrap().join("authored.orrt");
        let terrain_before = fs::read(&terrain_path).unwrap();
        let mut oracle = AuthoredPlayer::open(&scene).unwrap();
        let initial = oracle.frame().to_bytes();
        let view = oracle.view().unwrap();
        let mut old = gpu::renderer(
            &device,
            &view.terrain,
            view.navigator.path(),
            view.navigator.position(),
            PathDisplay::Current,
        )
        .unwrap();
        let negative = gpu::capture(&mut old, &gpu::top_camera(), &gpu::lighting(), SIZE).unwrap();
        let negative_features = features(&negative);
        assert!(
            !framed(&negative_features),
            "old fixed camera unexpectedly satisfies {name}: {negative_features:?}"
        );
        gpu::save_png(
            &output.join(format!("{name}-old-camera.png")),
            &negative,
            SIZE,
        )
        .unwrap();
        assert_eq!(
            oracle.frame().to_bytes(),
            initial,
            "view/negative capture changed Frame"
        );
        let (receipt0, pixels0) = capture(
            &scene,
            &output.join(format!("{name}-tick0.png")),
            0,
            &oracle,
        );
        let f0 = features(&pixels0);
        assert!(framed(&f0), "{name} initial: {f0:?}");
        assert!(
            f0.x < f0.orange * 500 && f0.y > f0.orange * 375,
            "literal initial lower-left quadrant: {name} {f0:?}"
        );
        assert_eq!(
            oracle.frame().to_bytes(),
            initial,
            "CLI capture changed CPU oracle"
        );
        for _ in 0..8 {
            oracle.step().unwrap();
        }
        let stepped = oracle.frame().to_bytes();
        let (receipt8, pixels8) = capture(
            &scene,
            &output.join(format!("{name}-tick8.png")),
            8,
            &oracle,
        );
        let f8 = features(&pixels8);
        assert!(framed(&f8), "{name} tick8: {f8:?}");
        assert_ne!(receipt0["position_raw"], receipt8["position_raw"]);
        let dx = (i128::from(f8.x) * i128::from(f0.orange)
            - i128::from(f0.x) * i128::from(f8.orange))
        .abs();
        let dy = (i128::from(f8.y) * i128::from(f0.orange)
            - i128::from(f0.y) * i128::from(f8.orange))
        .abs();
        assert!(
            dx + dy > i128::from(f0.orange) * i128::from(f8.orange) * 2,
            "literal agent centroid must move"
        );
        assert_eq!(oracle.frame().to_bytes(), stepped);
        oracle.restart().unwrap();
        assert_eq!(oracle.frame().to_bytes(), initial);
        assert_eq!(fs::read(scene).unwrap(), scene_before);
        assert_eq!(fs::read(terrain_path).unwrap(), terrain_before);
    }
}
