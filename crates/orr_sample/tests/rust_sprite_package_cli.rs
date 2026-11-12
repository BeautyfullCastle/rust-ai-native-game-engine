//! Explicit real-binary differential proof; Python is a baseline, not a product dependency.
//! CPU install/reopen and millisecond sampling only, not rendered pixels or license rights.
#![cfg(all(target_os = "linux", feature = "project-create"))]
#![allow(clippy::float_arithmetic)]
use orr_sample::{project::PreparedProject, project_runtime};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};
const PACKAGE: &str = "original-test-sprites";
const PIXELS: [u8; 16] = [255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 64, 7, 11, 13, 0];
const LICENSE: &[u8] =
    b"Original fixture; user supplied license, not rights verification.\r\nCC0-1.0\r\n";
struct Fixture {
    temp: tempfile::TempDir,
    png: PathBuf,
    json: PathBuf,
    license: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let f = Self {
            temp,
            png: root.join("input.png"),
            json: root.join("input.json"),
            license: root.join("LICENSE"),
        };
        fs::write(&f.png, png(false)).unwrap();
        fs::write(&f.json, serde_json::to_vec_pretty(&source()).unwrap()).unwrap();
        fs::write(&f.license, LICENSE).unwrap();
        f
    }
    fn output(&self, name: &str) -> PathBuf {
        self.temp.path().join(name)
    }
}
fn binary(name: &str) -> std::ffi::OsString {
    let value = std::env::var_os(name).unwrap_or_else(|| panic!("{name} required"));
    assert!(Path::new(&value).is_absolute() && Path::new(&value).is_file());
    value
}
#[derive(Clone, Copy, Debug)]
enum Frontend {
    Rust,
    Native,
    Python,
}
fn command(f: &Fixture, frontend: Frontend, out: &Path, explicit: bool) -> Command {
    let mut c = match frontend {
        Frontend::Rust => {
            let mut c = Command::new(binary("ORR_RUST_PACKAGE_BIN"));
            c.arg("create-sprite");
            c
        }
        Frontend::Native => Command::new(binary("ORR_SPRITE_PACKAGE_BIN")),
        Frontend::Python => {
            let mut c = Command::new("python3");
            c.arg("-B")
                .arg(
                    Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("../../tools/create_sprite_package.py"),
                )
                .arg("--packer")
                .arg(binary("ORR_SPRITE_PACKAGE_BIN"));
            c
        }
    };
    c.arg("--png")
        .arg(&f.png)
        .arg("--license")
        .arg(&f.license)
        .args(["--name", PACKAGE, "--version", "1.2.3", "--out"])
        .arg(out);
    if explicit {
        c.args([
            "--atlas-size",
            "2",
            "2",
            "--region",
            "7",
            "0",
            "0",
            "1",
            "2",
            "--region",
            "9",
            "1",
            "0",
            "1",
            "2",
            "--clip",
            "walk",
            "loop",
            "7:30,9:70",
            "--clip",
            "once",
            "once",
            "9:20,7:40",
        ]);
    } else {
        c.arg("--json").arg(&f.json);
    }
    c
}
#[expect(
    clippy::disallowed_types,
    reason = "bounded actual CLI timeout detects FIFO blocking"
)]
fn execute(mut c: Command) -> Output {
    use std::time::{Duration, Instant};
    let mut child = c
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if child.try_wait().unwrap().is_some() {
            return child.wait_with_output().unwrap();
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            let o = child.wait_with_output().unwrap();
            panic!("CLI timed out: {}", String::from_utf8_lossy(&o.stderr));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn success(c: Command) {
    let o = execute(c);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
}
fn rejects(f: &Fixture) {
    let before = [&f.png, &f.json, &f.license].map(|p| fs::read(p).unwrap());
    for (i, frontend) in [Frontend::Native, Frontend::Rust].into_iter().enumerate() {
        let out = f.output(&format!("reject-{i}"));
        let o = execute(command(f, frontend, &out, false));
        assert!(!o.status.success(), "{frontend:?} unexpectedly accepted");
        assert!(
            fs::symlink_metadata(out).is_err(),
            "admission failure published"
        );
    }
    assert_eq!(
        before,
        [&f.png, &f.json, &f.license].map(|p| fs::read(p).unwrap())
    );
}
fn source() -> Value {
    json!({
        "format":"orr_sprite", "version":1,
        "atlas":{"image":"atlas.png", "width":2, "height":2},
        "regions":[
            {"id":7,"x":0,"y":0,"width":1,"height":2},
            {"id":9,"x":1,"y":0,"width":1,"height":2}
        ],
        "clips":[
            {"id":"walk","mode":"loop","frames":[{"region":7,"duration_ms":30},{"region":9,"duration_ms":70}]},
            {"id":"once","mode":"once","frames":[{"region":9,"duration_ms":20},{"region":7,"duration_ms":40}]}
        ]
    })
}
fn png(animated: bool) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png17::Encoder::new(&mut bytes, 2, 2);
        encoder.set_color(png17::ColorType::Rgba);
        encoder.set_depth(png17::BitDepth::Eight);
        if animated {
            encoder.set_animated(1, 0).unwrap();
        }
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&PIXELS).unwrap();
        writer.finish().unwrap();
    }
    bytes
}
fn tree(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, files: &mut BTreeMap<String, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            assert!(!entry.file_type().unwrap().is_symlink());
            if entry.file_type().unwrap().is_dir() {
                walk(root, &entry.path(), files);
            } else {
                files.insert(
                    entry
                        .path()
                        .strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                    fs::read(entry.path()).unwrap(),
                );
            }
        }
    }
    let mut files = BTreeMap::new();
    walk(root, root, &mut files);
    files
}

#[test]
#[ignore = "requires actual ORR_RUST_PACKAGE_BIN and original ORR_SPRITE_PACKAGE_BIN"]
fn exact_python_native_rust_bytes_and_installed_consumer_without_sources() {
    let fixture = Fixture::new();
    let py = fixture.output("python");
    let native = fixture.output("native");
    let package = fixture.output("rust");
    success(command(&fixture, Frontend::Python, &py, true));
    // The native baseline and new direct JSON mode must preserve these exact bytes.
    fs::write(&fixture.json, fs::read(py.join("sprites.json")).unwrap()).unwrap();
    success(command(&fixture, Frontend::Native, &native, false));
    success(command(&fixture, Frontend::Rust, &package, true));
    let direct = fixture.output("direct");
    success(command(&fixture, Frontend::Rust, &direct, false));
    let expected = tree(&py);
    assert_eq!(expected.len(), 4);
    for path in [&native, &package, &direct] {
        assert_eq!(tree(path), expected);
        assert_eq!(
            orr_package::inspect_for_install(
                path,
                project_runtime::compiled_runtime().engine_version
            )
            .unwrap(),
            orr_package::inspect_for_install(
                &py,
                project_runtime::compiled_runtime().engine_version
            )
            .unwrap()
        );
    }
    assert_eq!(expected["atlas.png"], fs::read(&fixture.png).unwrap());
    assert_eq!(expected["LICENSE.txt"], LICENSE);
    // Additional formatter parity for UTF-8 and control escaping; no fps or pivot schema exists.
    for (i, id) in [
        "걷기",
        "quote\"slash\\tab\tnewline\n",
        "back\u{8}form\u{c}\u{1}",
    ]
    .iter()
    .enumerate()
    {
        let mut p = command(
            &fixture,
            Frontend::Python,
            &fixture.output(&format!("unicode-p-{i}")),
            true,
        );
        p.args(["--clip", id, "loop", "7:1"]);
        let mut r = command(
            &fixture,
            Frontend::Rust,
            &fixture.output(&format!("unicode-r-{i}")),
            true,
        );
        r.args(["--clip", id, "loop", "7:1"]);
        success(p);
        success(r);
        assert_eq!(
            tree(&fixture.output(&format!("unicode-p-{i}"))),
            tree(&fixture.output(&format!("unicode-r-{i}")))
        );
    }
    // Hide every alternate source package as well as the installed package below.
    for path in [&py, &native, &direct] {
        fs::rename(path, path.with_extension("hidden")).unwrap();
    }
    let project = fixture
        .temp
        .path()
        .canonicalize()
        .unwrap()
        .join("arena-project");
    fs::create_dir(&project).unwrap();
    let scene = fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/saved_arena_project/arena.scene.yaml"),
    )
    .unwrap();
    fs::write(project.join("arena.scene.yaml"), &scene).unwrap();
    fs::write(project.join("orr.project.json"), serde_json::to_vec(&json!({"schema":2,"engine":"^0.0.1","entry":{"game":"arena","scene":"arena.scene.yaml","sprites":"arena.sprites.json"}})).unwrap()).unwrap();
    fs::write(project.join("arena.sprites.json"), serde_json::to_vec(&json!({"version":1,"scene":"arena.scene.yaml","project":".","bindings":{"e_00000001":{"package":PACKAGE.to_string(),"document":"sprites.json","source":{"Clip":"walk"},"units_per_pixel":1.0}}})).unwrap()).unwrap();
    let installer = orr_package::Project::open_for_install(
        &project,
        project_runtime::compiled_runtime().engine_version,
    )
    .unwrap();
    let lock = installer.install(std::slice::from_ref(&package)).unwrap();
    assert_eq!(lock.packages.len(), 1);
    let admitted = PreparedProject::open(&project, project_runtime::compiled_runtime()).unwrap();
    let checksum = project_runtime::PreparedRuntime::open(&project)
        .unwrap()
        .initial_frame()
        .checksum();
    let snapshot = tree(&project);
    for path in [&fixture.png, &fixture.json, &fixture.license, &package] {
        let hidden = path.with_file_name(format!(
            "hidden-{}",
            path.file_name().unwrap().to_string_lossy()
        ));
        fs::rename(path, hidden).unwrap();
        assert!(!path.exists());
    }
    let reopened = PreparedProject::open(&project, project_runtime::compiled_runtime()).unwrap();
    for prepared in [&admitted, &reopened] {
        assert_eq!(prepared.scene().text().as_bytes(), scene);
        let sprites = prepared.sprites().unwrap();
        assert_eq!(sprites.assets.len(), 1);
        let asset = &sprites.assets[&(PACKAGE.to_string(), "sprites.json".into())];
        assert_eq!(asset.rgba, PIXELS);
        assert_eq!(asset.document.atlas().image, "atlas.png");
        assert_eq!(asset.document.regions().len(), 2);
        assert_eq!(asset.document.region(9).unwrap().x, 1);
        assert_eq!(asset.document.clips().len(), 2);
        assert_eq!(
            serde_json::from_str::<Value>(&asset.document.to_json().unwrap()).unwrap(),
            source(),
            "exact atlas, region and clip metadata survived installation"
        );
        assert_eq!(asset.document.clip("walk").unwrap().duration_ms(), 100);
        let binding = &sprites.document.bindings["e_00000001"];
        for (time, region) in [
            (0, 7),
            (29, 7),
            (30, 9),
            (99, 9),
            (100, 7),
            (130, 9),
            (u64::MAX, 7),
        ] {
            assert_eq!(
                binding.region(&asset.document, time).unwrap(),
                region,
                "time={time}"
            );
        }
        let once = asset.document.clip("once").unwrap();
        assert_eq!(once.duration_ms(), 60);
        for (time, region) in [(0, 9), (19, 9), (20, 7), (59, 7), (60, 7), (u64::MAX, 7)] {
            assert_eq!(once.sample(time).region, region);
            assert_eq!(once.sample(time).finished, time >= 60);
        }
    }
    assert_eq!(
        project_runtime::PreparedRuntime::open(&project)
            .unwrap()
            .initial_frame()
            .checksum(),
        checksum
    );
    assert_eq!(
        tree(&project),
        snapshot,
        "read-only admission changed installed project"
    );
}

#[test]
#[ignore = "requires actual old and new sprite authoring binaries"]
fn differential_malformed_inputs_bounds_and_duration_semantics() {
    let edits: &[fn(&mut Value)] = &[
        |v| v["atlas"]["width"] = json!(3),
        |v| v["atlas"]["width"] = json!(0),
        |v| v["atlas"]["width"] = json!(2049),
        |v| v["regions"][0]["width"] = json!(0),
        |v| v["regions"][0]["x"] = json!(2),
        |v| v["regions"][0]["id"] = json!(-1),
        |v| v["regions"][0]["id"] = json!(4294967296u64),
        |v| v["regions"][1]["id"] = json!(7),
        |v| v["clips"][1]["id"] = json!("walk"),
        |v| v["clips"][0]["mode"] = json!("ping_pong"),
        |v| v["clips"][0]["frames"][0]["region"] = json!(999),
        |v| v["clips"][0]["frames"][0]["duration_ms"] = json!(0),
        |v| v["clips"][0]["frames"][0]["duration_ms"] = json!(-1),
        |v| v["clips"][0]["frames"][0]["duration_ms"] = json!(u64::MAX),
        |v| v["clips"][0]["frames"][0]["duration_ms"] = json!(1.5),
        |v| v["clips"][0]["fps"] = json!(30),
        |v| v["regions"][0]["pivot"] = json!([0, 0]),
        |v| v["unknown"] = json!(true),
        |v| v["atlas"]["image"] = json!("../atlas.png"),
        |v| v["atlas"]["image"] = json!("CON.png"),
        |v| v["regions"] = json!(vec![v["regions"][0].clone(); 4097]),
        |v| v["clips"] = json!(vec![v["clips"][0].clone(); 257]),
        |v| v["clips"][0]["frames"] = json!(vec![v["clips"][0]["frames"][0].clone(); 4097]),
    ];
    for edit in edits {
        let f = Fixture::new();
        let mut v = source();
        edit(&mut v);
        fs::write(&f.json, serde_json::to_vec(&v).unwrap()).unwrap();
        rejects(&f);
    }
    for bytes in [
        b"{".to_vec(),
        vec![255],
        b"{\"version\":1,\"version\":1}".to_vec(),
        vec![b' '; 4 * 1024 * 1024 + 1],
    ] {
        let f = Fixture::new();
        fs::write(&f.json, bytes).unwrap();
        rejects(&f);
    }
    for bytes in [
        b"not PNG".to_vec(),
        png(false)[..20].to_vec(),
        png(true),
        vec![0; 16 * 1024 * 1024 + 1],
    ] {
        let f = Fixture::new();
        fs::write(&f.png, bytes).unwrap();
        rejects(&f);
    }
    for bytes in [Vec::new(), vec![255], vec![b'a'; 64 * 1024 + 1]] {
        let f = Fixture::new();
        fs::write(&f.license, bytes).unwrap();
        rejects(&f);
    }
    // u64::MAX is valid for ONE frame; reject aggregate overflow, not large durations alone.
    let f = Fixture::new();
    let mut v = source();
    v["clips"][0]["frames"] = json!([{"region":7,"duration_ms":u64::MAX}]);
    fs::write(&f.json, serde_json::to_vec_pretty(&v).unwrap()).unwrap();
    for (i, front) in [Frontend::Native, Frontend::Rust].into_iter().enumerate() {
        success(command(&f, front, &f.output(&format!("max-{i}")), false));
    }
    assert_eq!(tree(&f.output("max-0")), tree(&f.output("max-1")));
    // Actual explicit CLI semantic rejections; the 4MiB argv ceiling is a parser-unit concern
    // because the OS may reject an oversized exec before either CLI can inspect it.
    for extra in [
        vec!["--region", "7", "0", "0", "1", "1"],
        vec!["--clip", "bad", "loop", "7:0"],
        vec!["--clip", "bad", "loop", "7:18446744073709551616"],
        vec!["--clip", "bad", "loop", "7:18446744073709551615,9:1"],
        vec!["--region", "4294967296", "0", "0", "1", "1"],
    ] {
        for front in [Frontend::Python, Frontend::Rust] {
            let f = Fixture::new();
            let out = f.output("bad");
            let mut c = command(&f, front, &out, true);
            c.args(&extra);
            let o = execute(c);
            assert!(!o.status.success());
            assert!(!out.exists());
        }
    }
}

#[test]
#[ignore = "requires actual old and new sprite authoring binaries"]
fn differential_symlinks_nonregular_inputs_and_existing_output_preservation() {
    use std::os::unix::fs::symlink;
    for front in [Frontend::Native, Frontend::Rust] {
        for input in 0..3 {
            let f = Fixture::new();
            let path = [&f.png, &f.json, &f.license][input];
            let target = path.with_extension("real");
            fs::rename(path, &target).unwrap();
            symlink(&target, path).unwrap();
            let out = f.output("out");
            assert!(!execute(command(&f, front, &out, false)).status.success());
            assert!(!out.exists());
            assert!(fs::symlink_metadata(path).unwrap().file_type().is_symlink());
        }
        for fifo in [false, true] {
            let f = Fixture::new();
            fs::remove_file(&f.png).unwrap();
            if fifo {
                assert!(Command::new("mkfifo")
                    .arg(&f.png)
                    .status()
                    .unwrap()
                    .success());
            } else {
                fs::create_dir(&f.png).unwrap();
            }
            let out = f.output("out");
            assert!(!execute(command(&f, front, &out, false)).status.success());
            assert!(!out.exists());
        }
        for kind in 0..3 {
            let f = Fixture::new();
            let out = f.output("out");
            match kind {
                0 => {
                    fs::create_dir(&out).unwrap();
                    fs::write(out.join("keep"), b"sentinel").unwrap();
                }
                1 => fs::write(&out, b"sentinel").unwrap(),
                _ => symlink("missing", &out).unwrap(),
            }
            assert!(!execute(command(&f, front, &out, false)).status.success());
            match kind {
                0 => assert_eq!(fs::read(out.join("keep")).unwrap(), b"sentinel"),
                1 => assert_eq!(fs::read(out).unwrap(), b"sentinel"),
                _ => assert_eq!(fs::read_link(out).unwrap(), Path::new("missing")),
            }
        }
        let f = Fixture::new();
        let actual = f.output("actual");
        fs::create_dir(&actual).unwrap();
        let link = f.output("linked");
        symlink(&actual, &link).unwrap();
        assert!(!execute(command(&f, front, &link.join("out"), false))
            .status
            .success());
        assert_eq!(fs::read_dir(actual).unwrap().count(), 0);
    }
    // These assertions cover prepublication admission failures, not crash atomicity or
    // rollback of a partially published directory after a real write/fsync failure.
}

#[test]
#[ignore = "requires actual in-process Rust sprite authoring binary"]
fn rust_rejects_external_tool_and_unsupported_or_ambiguous_flags() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    let sentinel = f.output("executed");
    let tool = f.output("must-not-run");
    fs::write(
        &tool,
        format!("#!/bin/sh\ntouch '{}'\n", sentinel.display()),
    )
    .unwrap();
    fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).unwrap();
    for args in [
        vec!["--packer".to_owned(), tool.to_string_lossy().into_owned()],
        vec!["--fps".into(), "30".into()],
        vec!["--pivot".into(), "0,0".into()],
        vec!["--unknown".into(), "x".into()],
        vec!["--name".into(), "duplicate".into()],
        vec!["--atlas-size".into(), "2".into(), "2".into()],
        vec![
            "--region".into(),
            "1".into(),
            "0".into(),
            "0".into(),
            "1".into(),
            "1".into(),
        ],
        vec!["--clip".into(), "x".into(), "loop".into(), "7:1".into()],
    ] {
        let out = f.output("out");
        let mut c = command(&f, Frontend::Rust, &out, false);
        c.args(args);
        let result = execute(c);
        assert_eq!(
            result.status.code(),
            Some(2),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(!out.exists());
        assert!(!sentinel.exists());
    }
}
