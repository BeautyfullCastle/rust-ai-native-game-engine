//! Original handcrafted export-contract fixture, not an upstream/model generation.
//! Real package admission and Collect snapshots; this is not GPU-render proof.
#![cfg(all(
    target_os = "linux",
    feature = "collect-sprites",
    feature = "project-create"
))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
use orr_bridge::{Bridge, BridgeConfig, InProc, PlayHost, PlayerSlot, SimControl};
use orr_package::sprite_package::authoring::{import_sprite_gen_package, Request};
use orr_sample::{
    collect_game::{CollectInput, CollectRun, RESTART, WON},
    collect_project::{PreparedProject, ProgressSupport, SpriteSupport},
    project_runtime,
};
use orr_sprite::{PlaybackMode, SpriteDocument};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
const NAME: &str = "handcrafted-sprite-gen";
const MANIFEST: &str = include_str!("fixtures/sprite-gen/manifest.json");
const PNG: &[u8] = include_bytes!("fixtures/sprite-gen/original-atlas.png");
const LICENSE: &[u8] = include_bytes!("fixtures/sprite-gen/LICENSE.txt");
const PIXELS: [u8; 32] = [
    255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 64, 7, 11, 13, 0, 255, 255, 0, 255, 255, 0, 255,
    128, 0, 255, 255, 64, 17, 19, 23, 0,
];
fn rect(document: &SpriteDocument, region: u32) -> [u32; 4] {
    let r = document.region(region).unwrap();
    [r.x, r.y, r.width, r.height]
}
fn verify_contract(document: &SpriteDocument) {
    assert_eq!(document.atlas().image, "atlas.png");
    assert_eq!((document.atlas().width, document.atlas().height), (4, 2));
    assert_eq!(document.regions().len(), 4);
    assert_eq!(
        document.clips().iter().map(|c| c.id()).collect::<Vec<_>>(),
        ["idle", "once"]
    );
    let idle = document.clip("idle").unwrap();
    assert_eq!(idle.mode(), PlaybackMode::Loop);
    assert_eq!(idle.duration_ms(), 600);
    assert_eq!(idle.frames().len(), 3);
    assert_eq!(idle.frames()[0].region, idle.frames()[2].region);
    for (time, x, index) in [
        (0, 2, 0),
        (99, 2, 0),
        (100, 0, 1),
        (299, 0, 1),
        (300, 2, 2),
        (599, 2, 2),
        (600, 2, 0),
    ] {
        let sample = idle.sample(time);
        assert_eq!(sample.frame_index, index);
        assert_eq!(rect(document, sample.region), [x, 0, 2, 1]);
        assert!(!sample.finished);
    }
    let once = document.clip("once").unwrap();
    assert_eq!(once.mode(), PlaybackMode::Once);
    assert_eq!(once.duration_ms(), 400);
    for (time, x, finished) in [
        (0, 2, false),
        (149, 2, false),
        (150, 0, false),
        (399, 0, false),
        (400, 0, true),
        (u64::MAX, 0, true),
    ] {
        let sample = once.sample(time);
        assert_eq!(rect(document, sample.region), [x, 1, 2, 1]);
        assert_eq!(sample.finished, finished);
    }
    assert_eq!(
        document.uv_rect(idle.frames()[0].region),
        Some([0.5, 0.0, 1.0, 0.5])
    );
}
#[test]
fn handcrafted_manifest_preserves_rectangles_order_timing_and_alpha() {
    let document = orr_sprite::sprite_gen::from_json(MANIFEST).unwrap();
    verify_contract(&document);
    assert_eq!(
        orr_sprite::png_import::decode_atlas(PNG, 4, 2).unwrap(),
        PIXELS
    );
    // Ignore object-member order, not playback-array order.
    let reordered =
        serde_json::to_string(&serde_json::from_str::<serde_json::Value>(MANIFEST).unwrap())
            .unwrap();
    assert_eq!(
        document.to_json().unwrap(),
        orr_sprite::sprite_gen::from_json(&reordered)
            .unwrap()
            .to_json()
            .unwrap()
    );
}
struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    request: Request,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let source = root.join("source");
        fs::create_dir(&source).unwrap();
        let request = Request {
            png: source.join("pixels.png"),
            json: source.join("manifest.json"),
            license: source.join("LICENSE"),
            name: NAME.into(),
            version: "1.0.0".into(),
            out: root.join("package"),
        };
        fs::write(&request.png, PNG).unwrap();
        fs::write(&request.json, MANIFEST).unwrap();
        fs::write(&request.license, LICENSE).unwrap();
        Self {
            _temp: temp,
            root,
            request,
        }
    }
}
fn open(root: &Path) -> PreparedProject {
    PreparedProject::open_with_presentation(
        root,
        ProgressSupport::MetadataOnly,
        SpriteSupport::Supported,
    )
    .unwrap()
}
fn exercise_installed_collect(f: &Fixture) {
    assert_eq!(fs::read(f.request.out.join("atlas.png")).unwrap(), PNG);
    assert_eq!(
        fs::read(f.request.out.join("LICENSE.txt")).unwrap(),
        LICENSE
    );
    let root = f.root.join("collect");
    fs::create_dir(&root).unwrap();
    let scene = include_bytes!("../../../scenes/collect_dodge_v1.scene.yaml");
    fs::write(root.join("level.scene.yaml"), scene).unwrap();
    let descriptor = json!({"schema":2,"engine":"*","entry":{"game":"collect-dodge-v1","scene":"level.scene.yaml"}});
    fs::write(
        root.join("orr.project.json"),
        serde_json::to_vec(&descriptor).unwrap(),
    )
    .unwrap();
    let primitive = open(&root);
    let checksum = primitive.scene().frame().checksum();
    orr_package::Project::open_for_install(
        &root,
        project_runtime::compiled_runtime().engine_version,
    )
    .unwrap()
    .install(std::slice::from_ref(&f.request.out))
    .unwrap();
    let mut descriptor = descriptor;
    descriptor["entry"]["sprites"] = json!("view.json");
    fs::write(
        root.join("orr.project.json"),
        serde_json::to_vec(&descriptor).unwrap(),
    )
    .unwrap();
    fs::write(root.join("view.json"), serde_json::to_vec(&json!({"version":2,"scene":"level.scene.yaml","project":".","bindings":{
        "e_00000001":{"package":NAME,"document":"sprites.json","source":{"Clip":"idle"},"units_per_pixel":0.5},
        "e_00000002":{"package":NAME,"document":"sprites.json","source":{"Clip":"once"},"units_per_pixel":0.5}
    }})).unwrap()).unwrap();
    let admitted = open(&root);
    assert_eq!(admitted.scene().frame().checksum(), checksum);
    fs::remove_dir_all(f.root.join("source")).unwrap();
    fs::remove_dir_all(&f.request.out).unwrap();
    let project = open(&root); // Reopen from hash-verified installation, no source fallback.
    let asset = &project.sprites().unwrap().assets[&(NAME.into(), "sprites.json".into())];
    assert_eq!(asset.rgba, PIXELS);
    verify_contract(&asset.document);
    let mut p = project.presentation();
    let mut b = InProc::new(
        PlayHost::new(project.scene().session().unwrap(), PlayerSlot(0)),
        BridgeConfig::default(),
    );
    let mut baseline = InProc::new(
        PlayHost::new(primitive.scene().session().unwrap(), PlayerSlot(0)),
        BridgeConfig::default(),
    );
    fs::remove_dir_all(&root).unwrap(); // Runtime assets must be owned after admission.
    let key = (NAME.into(), "sprites.json".into());
    for tick in 0..=36 {
        if tick > 0 {
            b.step(1);
            baseline.step(1);
        }
        let before = b.snapshot().unwrap().predicted().checksum();
        p.update(b.snapshot().as_ref()).unwrap();
        assert_eq!(b.snapshot().unwrap().predicted().checksum(), before);
        assert_eq!(before, baseline.snapshot().unwrap().predicted().checksum());
        let ms = tick * 1000 / 60;
        assert_eq!(p.sprites().len(), 2);
        for (index, clip) in ["idle", "once"].iter().enumerate() {
            assert_eq!(
                p.sprites()[index].instance.region,
                p.assets()[&key]
                    .document
                    .clip(clip)
                    .unwrap()
                    .sample(ms)
                    .region
            );
        }
    }
    for bridge in [&mut b, &mut baseline] {
        bridge.control(orr_session::ControlOp::Seek(0)).unwrap();
    }
    p.update(b.snapshot().as_ref()).unwrap();
    assert_eq!(
        rect(&p.assets()[&key].document, p.sprites()[0].instance.region),
        [2, 0, 2, 1]
    );
    // Explicit presentation reset also reconstructs the same owned assets.
    p.reset();
    p.update(b.snapshot().as_ref()).unwrap();
    assert_eq!(p.sprites().len(), 2);
    assert_eq!(
        rect(&p.assets()[&key].document, p.sprites()[0].instance.region),
        [2, 0, 2, 1]
    );
    for bridge in [&mut b, &mut baseline] {
        bridge.control(orr_session::ControlOp::Play).unwrap();
        bridge
            .set_input(
                PlayerSlot(0),
                CollectInput {
                    x: orr_fp::FP::ONE,
                    ..Default::default()
                },
            )
            .unwrap();
    }
    for _ in 0..120 {
        b.step(1);
        baseline.step(1);
        p.update(b.snapshot().as_ref()).unwrap();
        assert_eq!(
            b.snapshot().unwrap().predicted().checksum(),
            baseline.snapshot().unwrap().predicted().checksum()
        );
    }
    assert_eq!(
        b.snapshot()
            .unwrap()
            .predicted()
            .singleton::<CollectRun>()
            .phase,
        WON
    );
    assert_eq!(p.sprites().len(), 1);
    for bridge in [&mut b, &mut baseline] {
        bridge
            .set_input(
                PlayerSlot(0),
                CollectInput {
                    buttons: RESTART,
                    ..Default::default()
                },
            )
            .unwrap();
        bridge.step(1);
    }
    p.update(b.snapshot().as_ref()).unwrap();
    assert_eq!(p.sprites().len(), 2);
    assert_eq!(
        rect(&p.assets()[&key].document, p.sprites()[0].instance.region),
        [2, 0, 2, 1]
    );
    assert_eq!(
        b.snapshot().unwrap().predicted().checksum(),
        baseline.snapshot().unwrap().predicted().checksum()
    );
    assert_eq!(project.scene().frame().checksum(), checksum);
}
#[test]
fn imported_package_collect_install_play_seek_win_restart_without_sources() {
    let f = Fixture::new();
    let report = import_sprite_gen_package(&f.request).unwrap();
    assert_eq!((report.width, report.height, report.rgba_bytes), (4, 2, 32));
    exercise_installed_collect(&f);
}
#[test]
#[ignore = "requires actual ORR_SPRITE_GEN_PACKAGE_BIN pointing to Linux sprite-authoring orr_pkg"]
fn actual_sprite_gen_cli_package_runs_in_collect_without_sources() {
    let f = Fixture::new();
    let binary = std::env::var_os("ORR_SPRITE_GEN_PACKAGE_BIN")
        .expect("ORR_SPRITE_GEN_PACKAGE_BIN required");
    assert!(Path::new(&binary).is_absolute() && Path::new(&binary).is_file());
    let mut child = Command::new(binary)
        .arg("create-sprite")
        .arg("--sprite-gen")
        .arg(&f.request.json)
        .arg("--png")
        .arg(&f.request.png)
        .arg("--license")
        .arg(&f.request.license)
        .args(["--name", NAME, "--version", "1.0.0", "--out"])
        .arg(&f.request.out)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            let output = child.wait_with_output().unwrap();
            panic!("CLI timed out: {}", String::from_utf8_lossy(&output.stderr));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    exercise_installed_collect(&f);
}

#[test]
fn sprite_gen_import_rejects_bad_inputs_without_publication_or_source_changes() {
    // Exercise the new input mode through the same bounded package reader.
    for case in [
        "invalid-manifest",
        "missing-manifest",
        "duplicate-state",
        "zero-duration",
        "dimension-mismatch",
        "invalid-png",
        "missing-license",
        "manifest-directory",
        "oversized-manifest",
        "symlink-manifest",
    ] {
        let f = Fixture::new();
        match case {
            "invalid-manifest" => fs::write(&f.request.json, b"not JSON").unwrap(),
            "missing-manifest" => fs::remove_file(&f.request.json).unwrap(),
            "duplicate-state" => {
                let needle = "\"idle\": [";
                assert!(MANIFEST.contains(needle));
                fs::write(
                    &f.request.json,
                    MANIFEST.replacen(needle, "\"idle\": [], \"idle\": [", 1),
                )
                .unwrap();
            }
            "zero-duration" => {
                let mut manifest: serde_json::Value = serde_json::from_str(MANIFEST).unwrap();
                manifest["animation"]["rows"]["idle"]["durations_ms"][0] = json!(0);
                fs::write(&f.request.json, serde_json::to_vec(&manifest).unwrap()).unwrap();
            }
            "dimension-mismatch" => {
                let mut manifest: serde_json::Value = serde_json::from_str(MANIFEST).unwrap();
                manifest["frame_layout"]["sheetWidth"] = json!(5);
                fs::write(&f.request.json, serde_json::to_vec(&manifest).unwrap()).unwrap();
            }
            "invalid-png" => fs::write(&f.request.png, b"not a PNG").unwrap(),
            "missing-license" => fs::remove_file(&f.request.license).unwrap(),
            "manifest-directory" => {
                fs::remove_file(&f.request.json).unwrap();
                fs::create_dir(&f.request.json).unwrap();
                fs::write(f.request.json.join("sentinel"), b"keep this directory").unwrap();
            }
            "oversized-manifest" => {
                fs::write(&f.request.json, vec![b' '; orr_sprite::MAX_JSON_BYTES + 1]).unwrap()
            }
            "symlink-manifest" => {
                fs::rename(&f.request.json, f.root.join("real-manifest.json")).unwrap();
                std::os::unix::fs::symlink(f.root.join("real-manifest.json"), &f.request.json)
                    .unwrap();
            }
            _ => unreachable!(),
        }
        // Record bytes, link identity and directory sentinel, rather than following
        // links in an assertion that might hide an unintended rewrite.
        let input_paths = [&f.request.png, &f.request.json, &f.request.license];
        let before = input_paths.map(|path| {
            let metadata = fs::symlink_metadata(path).ok();
            let kind = metadata
                .as_ref()
                .map(|m| (m.is_file(), m.is_dir(), m.file_type().is_symlink()));
            (kind, fs::read(path).ok(), fs::read_link(path).ok())
        });
        assert!(
            import_sprite_gen_package(&f.request).is_err(),
            "accepted {case}"
        );
        assert!(
            fs::symlink_metadata(&f.request.out).is_err(),
            "published after {case}"
        );
        let after = input_paths.map(|path| {
            let metadata = fs::symlink_metadata(path).ok();
            let kind = metadata
                .as_ref()
                .map(|m| (m.is_file(), m.is_dir(), m.file_type().is_symlink()));
            (kind, fs::read(path).ok(), fs::read_link(path).ok())
        });
        assert_eq!(before, after, "changed source after {case}");
        if case == "manifest-directory" {
            assert_eq!(
                fs::read(f.request.json.join("sentinel")).unwrap(),
                b"keep this directory"
            );
        }
        if case == "symlink-manifest" {
            assert_eq!(
                fs::read(f.root.join("real-manifest.json")).unwrap(),
                MANIFEST.as_bytes()
            );
        }
    }
    let f = Fixture::new();
    fs::create_dir(&f.request.out).unwrap();
    fs::write(f.request.out.join("sentinel"), b"existing output stays").unwrap();
    assert!(import_sprite_gen_package(&f.request).is_err());
    assert_eq!(
        fs::read(f.request.out.join("sentinel")).unwrap(),
        b"existing output stays"
    );
    assert_eq!(fs::read_dir(&f.request.out).unwrap().count(), 1);
    assert_eq!(fs::read(&f.request.png).unwrap(), PNG);
    assert_eq!(fs::read(&f.request.json).unwrap(), MANIFEST.as_bytes());
    assert_eq!(fs::read(&f.request.license).unwrap(), LICENSE);
}
