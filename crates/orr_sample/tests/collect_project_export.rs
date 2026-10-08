//! Mandatory explicit acceptance for built CollectDodge tools; no silent skips.
//! ORR_COLLECT_RUNTIME=/abs/collect_dodge ORR_COLLECT_EXPORTER=/abs/orr_export_collect
//! cargo test -p orr_sample --features collect-dodge,project-export --test collect_project_export -- --ignored
#![cfg(all(
    feature = "collect-dodge",
    feature = "project-export",
    target_os = "linux",
    target_arch = "x86_64"
))]
#![allow(clippy::disallowed_types)]
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};
fn good(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}\n{}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}
fn snapshot(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn visit(root: &Path, path: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for e in fs::read_dir(path).unwrap() {
            let e = e.unwrap();
            let t = e.file_type().unwrap();
            assert!(!t.is_symlink());
            if t.is_dir() {
                visit(root, &e.path(), out);
            } else {
                assert!(t.is_file());
                out.insert(
                    e.path()
                        .strip_prefix(root)
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .into(),
                    fs::read(e.path()).unwrap(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    visit(root, root, &mut out);
    out
}
struct Work {
    root: tempfile::TempDir,
    project: PathBuf,
    runtime: PathBuf,
    exporter: PathBuf,
    workspace: PathBuf,
}
impl Work {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let base = root.path().canonicalize().unwrap();
        let project = base.join("edited project");
        fs::create_dir(&project).unwrap();
        fs::write(
            project.join("orr.project.json"),
            include_bytes!("../../../assets/collect_dodge_project/orr.project.json"),
        )
        .unwrap();
        // Deliberately authored variation, not the unedited checked-in fixture.
        let scene = include_str!("../../../scenes/collect_dodge_v1.scene.yaml")
            .replace("position: [10, 0]", "position: [12, 0]");
        fs::write(project.join("level.scene.yaml"), scene).unwrap();
        fs::create_dir(base.join("tools")).unwrap();
        fs::create_dir(base.join("empty")).unwrap();
        fs::create_dir(base.join("captures")).unwrap();
        let mut copied = Vec::new();
        for (key, name) in [
            ("ORR_COLLECT_RUNTIME", "collect_dodge"),
            ("ORR_COLLECT_EXPORTER", "orr_export_collect"),
        ] {
            let source = PathBuf::from(
                std::env::var_os(key).unwrap_or_else(|| panic!("mandatory acceptance needs {key}")),
            )
            .canonicalize()
            .unwrap();
            let dest = base.join("tools").join(name);
            fs::copy(&source, &dest).unwrap();
            copied.push(dest);
        }
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../..")
            .canonicalize()
            .unwrap();
        assert!(!base.starts_with(&workspace));
        Self {
            root,
            project,
            runtime: copied[0].clone(),
            exporter: copied[1].clone(),
            workspace,
        }
    }
    fn isolated(&self, executable: &Path, bundle: Option<&Path>) -> Command {
        let base = self.root.path().canonicalize().unwrap();
        let mut c = Command::new("bwrap");
        c.args([
            "--die-with-parent",
            "--ro-bind",
            "/",
            "/",
            "--dev-bind",
            "/dev/null",
            "/dev/null",
            "--bind",
        ])
        .arg(&base)
        .arg(&base)
        .arg("--tmpfs")
        .arg(&self.workspace);
        if let Some(bundle) = bundle {
            c.arg("--tmpfs")
                .arg(&self.project)
                .arg("--tmpfs")
                .arg(base.join("tools"))
                .arg("--ro-bind")
                .arg(bundle)
                .arg(bundle);
        }
        c.arg("--")
            .arg(executable)
            .current_dir(base.join("empty"))
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY");
        c
    }
    fn run(
        &self,
        exe: &Path,
        bundle: Option<&Path>,
        ticks: u32,
        held: &str,
        capture: Option<&Path>,
    ) -> String {
        let mut c = self.isolated(exe, bundle);
        if bundle.is_none() {
            c.arg("--project").arg(&self.project);
        }
        c.args(["--headless", "--ticks", &ticks.to_string()]);
        if !held.is_empty() {
            c.args(["--hold", held]);
        }
        if let Some(path) = capture {
            c.arg("--capture").arg(path);
        }
        good(c.output().unwrap())
    }
    fn export(&self) -> PathBuf {
        let bundle = self.root.path().join("relocated bundle");
        let hash = hex(&fs::read(&self.runtime).unwrap());
        good(
            self.isolated(&self.exporter, None)
                .arg("--project")
                .arg(&self.project)
                .arg("--runtime")
                .arg(&self.runtime)
                .args(["--runtime-sha256", &hash, "--trusted-runtime", "--output"])
                .arg(&bundle)
                .output()
                .unwrap(),
        );
        bundle
    }
}
#[test]
#[ignore = "requires explicit built runtime/exporter and Linux namespace isolation"]
fn collect_real_export_cpu_workflow() {
    let w = Work::new();
    let before = snapshot(&w.project);
    for (ticks, held, status) in [
        (0, "", "PLAYING | score 0/2"),
        (8, "right", "PLAYING | score 1/2"),
        (16, "right", "WON | score 2/2"),
        (8, "up", "LOST: hazard"),
        (600, "", "LOST: time"),
        (17, "right,restart", "WON | score 2/2"),
    ] {
        assert!(w.run(&w.runtime, None, ticks, held, None).contains(status));
    }
    let bundle = w.export();
    let bytes = snapshot(&bundle);
    let exe = bundle.join("run-collect-dodge");
    for (ticks, held) in [
        (0, ""),
        (8, "right"),
        (16, "right"),
        (8, "up"),
        (600, ""),
        (17, "right,restart"),
    ] {
        assert_eq!(
            w.run(&w.runtime, None, ticks, held, None),
            w.run(&exe, Some(&bundle), ticks, held, None)
        );
    }
    assert_eq!(snapshot(&w.project), before);
    assert_eq!(snapshot(&bundle), bytes);
    assert!(bytes.contains_key("bin/collect_dodge"));
    assert!(!bytes.keys().any(|k| k.contains("arena")));
    let hash = hex(&fs::read(&w.runtime).unwrap());
    let rejected = w
        .isolated(&w.exporter, None)
        .arg("--project")
        .arg(&w.project)
        .arg("--runtime")
        .arg(&w.runtime)
        .args(["--runtime-sha256", &hash, "--trusted-runtime", "--output"])
        .arg(&bundle)
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    assert_eq!(snapshot(&bundle), bytes);
}
fn counts(path: &Path) -> [usize; 3] {
    let mut reader = png::Decoder::new(std::io::BufReader::new(fs::File::open(path).unwrap()))
        .read_info()
        .unwrap();
    let mut bytes = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut bytes).unwrap();
    assert_eq!(info.color_type, png::ColorType::Rgba);
    let mut n = [0; 3];
    for p in bytes[..info.buffer_size()].chunks_exact(4) {
        if p[2] > 200 && p[0] < 150 && p[1] < 200 {
            n[0] += 1;
        }
        if p[0] > 200 && p[1] > 200 && p[2] < 120 {
            n[1] += 1;
        }
        if p[0] > 200 && p[1] < 150 && p[2] < 150 {
            n[2] += 1;
        }
    }
    n
}
#[test]
#[ignore = "requires explicit built tools, namespace isolation and real software GPU"]
fn collect_real_export_gpu_workflow() {
    let w = Work::new();
    let bundle = w.export();
    let before = snapshot(&bundle);
    for (name, ticks, held, coins) in [
        ("initial", 0, "", true),
        ("won", 16, "right", false),
        ("hazard", 8, "up", true),
    ] {
        let a = w
            .root
            .path()
            .join("captures")
            .join(format!("{name}-source.png"));
        let b = w
            .root
            .path()
            .join("captures")
            .join(format!("{name}-export.png"));
        let text = w.run(&w.runtime, None, ticks, held, Some(&a));
        assert!(text.contains("software: true"));
        w.run(
            &bundle.join("run-collect-dodge"),
            Some(&bundle),
            ticks,
            held,
            Some(&b),
        );
        assert_eq!(fs::read(&a).unwrap(), fs::read(&b).unwrap());
        let n = counts(&a);
        assert!(
            n[0] > 5 && n[2] > 5,
            "missing actual player/hazard pixels: {n:?}"
        );
        assert_eq!(n[1] > 5, coins, "collectible visibility: {n:?}");
        if let Some(dest) = std::env::var_os("ORR_COLLECT_CAPTURES") {
            let dest = PathBuf::from(dest);
            fs::create_dir_all(&dest).unwrap();
            fs::copy(&a, dest.join(a.file_name().unwrap())).unwrap();
            fs::copy(&b, dest.join(b.file_name().unwrap())).unwrap();
        }
        let bytes = fs::read(&a).unwrap();
        let failed = w
            .isolated(&w.runtime, None)
            .arg("--project")
            .arg(&w.project)
            .args(["--headless", "--ticks", "0", "--capture"])
            .arg(&a)
            .output()
            .unwrap();
        assert!(!failed.status.success());
        assert_eq!(fs::read(&a).unwrap(), bytes);
    }
    assert_eq!(snapshot(&bundle), before);
}

fn hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[cfg(feature = "collect-progress")]
#[test]
#[ignore = "requires built progress runtime, exporter and exact test binary; namespace isolation"]
fn collect_progress_source_hidden_relaunch_and_no_headless_writes() {
    let w = Work::new();
    let manifest_path = w.project.join("orr.project.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest["schema"] = 3.into();
    manifest["progress"] = serde_json::json!({"schema":1,"game_id":"12345678-1234-4234-8234-123456789abc","profile":"collect-dodge-highscore-v1"});
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    let before = snapshot(&w.project);
    let data = w.root.path().join("isolated progress data");
    let run = |c: &mut Command| {
        c.env("XDG_DATA_HOME", &data)
            .env("HOME", w.root.path().join("isolated home"));
        good(c.output().unwrap())
    };
    for ticks in [0, 20, 600] {
        let mut c = w.isolated(&w.runtime, None);
        c.arg("--project").arg(&w.project).args([
            "--headless",
            "--ticks",
            &ticks.to_string(),
            "--hold",
            "right",
        ]);
        run(&mut c);
        assert!(!data.exists(), "headless created profile data");
    }
    let bundle = w.root.path().join("progress export");
    let hash = hex(&fs::read(&w.runtime).unwrap());
    let mut c = w.isolated(&w.exporter, None);
    c.arg("--project")
        .arg(&w.project)
        .arg("--runtime")
        .arg(&w.runtime)
        .args(["--runtime-sha256", &hash, "--trusted-runtime", "--output"])
        .arg(&bundle);
    run(&mut c);
    assert!(!data.exists(), "export smoke created profile data");
    assert_eq!(snapshot(&w.project), before);
    assert_eq!(
        fs::read(bundle.join("project/orr.project.json")).unwrap(),
        fs::read(&manifest_path).unwrap()
    );
    let bundle_before = snapshot(&bundle);
    let test_binary = PathBuf::from(
        std::env::var_os("ORR_PROGRESS_TEST_BIN").expect("exact built sample test binary"),
    );
    assert!(test_binary.is_absolute() && test_binary.is_file());
    let harness_dir = w.root.path().join("test-harness");
    fs::create_dir(&harness_dir).unwrap();
    let harness = harness_dir.join("progress-test-harness");
    fs::copy(test_binary, &harness).unwrap();
    for mode in ["win", "relaunch"] {
        let mut c = w.isolated(&harness, Some(&bundle));
        c.args([
            "--ignored",
            "--exact",
            "collect_progress::tests::isolated_window_progress_child",
            "--nocapture",
        ])
        .env("ORR_PROGRESS_TEST_PROJECT", bundle.join("project"))
        .env("ORR_PROGRESS_TEST_MODE", mode);
        let output = run(&mut c);
        assert!(
            output.contains("1 passed"),
            "explicit helper did not execute: {output}"
        );
    }
    assert!(data.join("orrery/games").is_dir());
    assert_eq!(snapshot(&bundle), bundle_before);
    assert_eq!(snapshot(&w.project), before);
}

/// The two directory names are reserved identities in this fixture. Checking
/// their names as well as absolute paths also observes relative path arguments,
/// decoded `-yy` dirfds/AT_FDCWD and chdir into either root. Do not infer profile
/// access from generic names such as `.local/share`, `orrery` or `highscore.json`.
/// This is a guard for this fixture, not a general strace path resolver.
#[cfg(feature = "collect-progress")]
fn check_collect_progress_trace(trace: &str, data: &Path, home: &Path) -> Result<(), &'static str> {
    if !trace.contains("execve(") {
        return Err("syscall observation did not execute");
    }
    for root in [data, home] {
        if trace.contains(root.to_str().unwrap())
            || trace.contains(root.file_name().unwrap().to_str().unwrap())
        {
            return Err("noninteractive route touched progress root");
        }
    }
    Ok(())
}

#[cfg(feature = "collect-progress")]
#[test]
fn collect_progress_syscall_trace_guard_rejects_profile_access() {
    let data = Path::new("/proof/isolated progress data");
    let home = Path::new("/proof/isolated home");
    let exec = "11 execve(\"/proof/tools/collect_dodge\", [\"collect_dodge\", \"--headless\", \"--ticks\", \"20\"], 0x0 /* 20 vars */) = 0\n";
    for calls in [
        "",
        "11 openat(AT_FDCWD, \"/etc/ld.so.cache\", O_RDONLY|O_CLOEXEC) = 3</etc/ld.so.cache>\n",
        "11 openat(4</proof/edited project>, \"level.scene.yaml\", O_RDONLY) = 5</proof/edited project/level.scene.yaml>\n",
        "11 openat(AT_FDCWD</proof/empty>, \"unrelated/highscore.json\", O_RDONLY) = -1 ENOENT (No such file or directory)\n",
        "11 openat(3</proof/unrelated>, \".local/share/orrery/highscore.json\", O_RDONLY) = -1 ENOENT (No such file or directory)\n",
        "11 chdir(\"/proof/empty\") = 0\n11 openat(AT_FDCWD, \"unrelated\", O_RDONLY) = -1 ENOENT (No such file or directory)\n",
    ] {
        let trace = format!("{exec}{calls}");
        assert_eq!(check_collect_progress_trace(&trace, data, home), Ok(()), "{trace}");
    }
    for root in ["isolated progress data", "isolated home"] {
        for calls in [
            format!("11 openat(AT_FDCWD, \"/proof/{root}/.local/share\", O_RDONLY|O_DIRECTORY) = -1 ENOENT (No such file or directory)\n"),
            // Original HOME counterexample: failed access creates no XDG directory.
            format!("11 openat(3</proof>, \"{root}/.local/share\", O_RDONLY|O_DIRECTORY) = -1 ENOENT (No such file or directory)\n"),
            format!("11 openat(3</proof/{root}>, \".local/share\", O_RDONLY|O_DIRECTORY) = -1 ENOENT (No such file or directory)\n"),
            format!("11 openat(AT_FDCWD</proof/{root}>, \".local/share\", O_RDONLY|O_DIRECTORY) = -1 ENOENT (No such file or directory)\n"),
            format!("11 openat(AT_FDCWD</proof/empty>, \"../{root}/.local/share\", O_RDONLY|O_DIRECTORY) = -1 ENOENT (No such file or directory)\n"),
            format!("11 chdir(\"/proof/{root}\") = 0\n11 openat(AT_FDCWD, \".local/share\", O_RDONLY|O_DIRECTORY) = -1 ENOENT (No such file or directory)\n"),
        ] {
            let trace = format!("{exec}{calls}");
            assert_eq!(
                check_collect_progress_trace(&trace, data, home),
                Err("noninteractive route touched progress root"),
                "{trace}"
            );
        }
    }
    for trace in [
        "",
        "11 openat(AT_FDCWD, \"/etc/ld.so.cache\", O_RDONLY) = 3</etc/ld.so.cache>\n",
    ] {
        assert_eq!(
            check_collect_progress_trace(trace, data, home),
            Err("syscall observation did not execute")
        );
    }
}

/// Additional OS syscall observation. Kept separate so environments denying
/// ptrace report this check blocked without disguising independent acceptance.
#[cfg(feature = "collect-progress")]
#[test]
#[ignore = "requires strace/ptrace permission plus built progress runtime and exporter"]
fn collect_progress_noninteractive_syscall_isolation() {
    let w = Work::new();
    let path = w.project.join("orr.project.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    manifest["schema"] = 3.into();
    manifest["progress"] = serde_json::json!({"schema":1,"game_id":"12345678-1234-4234-8234-123456789abc","profile":"collect-dodge-highscore-v1"});
    fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let data = w.root.path().join("isolated progress data");
    let home = w.root.path().join("isolated home");
    let trace_number = std::cell::Cell::new(0u32);
    let run_no_profile = |c: &mut Command| {
        c.env("XDG_DATA_HOME", &data).env("HOME", &home);
        let n = trace_number.get();
        trace_number.set(n + 1);
        let trace_path = w.root.path().join(format!("profile-file-syscalls-{n}.log"));
        let mut traced = Command::new("strace");
        traced
            .args(["-f", "-yy", "-s", "4096", "-e", "trace=%file", "-o"])
            .arg(&trace_path)
            .arg(c.get_program())
            .args(c.get_args());
        if let Some(dir) = c.get_current_dir() {
            traced.current_dir(dir);
        }
        for (key, value) in c.get_envs() {
            if let Some(value) = value {
                traced.env(key, value);
            } else {
                traced.env_remove(key);
            }
        }
        let output = good(traced.output().unwrap());
        let trace = fs::read_to_string(trace_path).unwrap();
        if let Err(reason) = check_collect_progress_trace(&trace, &data, &home) {
            panic!("{reason}: {trace}");
        }
        output
    };

    let mut c = w.isolated(&w.runtime, None);
    c.arg("--project")
        .arg(&w.project)
        .args(["--headless", "--ticks", "20", "--hold", "right"]);
    run_no_profile(&mut c);
    let hash = hex(&fs::read(&w.runtime).unwrap());
    let mut c = w.isolated(&w.exporter, None);
    c.arg("--project")
        .arg(&w.project)
        .arg("--runtime")
        .arg(&w.runtime)
        .args(["--runtime-sha256", &hash, "--trusted-runtime", "--output"])
        .arg(w.root.path().join("traced export"));
    run_no_profile(&mut c);
    assert!(!data.exists());
}

#[cfg(feature = "collect-sprites")]
#[path = "common/collect_sprites.rs"]
mod sprite_fixture;
#[cfg(feature = "collect-sprites")]
#[test]
#[ignore = "requires exact built sprite runtime/exporter, source-hidden read-only namespace and software GPU"]
fn collect_sprite_source_hidden_export_matches_actual_atlas() {
    let w = Work::new();
    sprite_fixture::fixture(&w.project);
    let before = snapshot(&w.project);
    let bundle = w.export();
    let bytes = snapshot(&bundle);
    assert!(bytes.keys().any(|k| k.ends_with("view.json")));
    assert!(bytes.keys().any(|k| k.ends_with("lantern_keeper.png")));
    for (name, ticks, held) in [
        ("sprite-initial", 0, ""),
        ("sprite-idle", 36, ""),
        ("sprite-won", 16, "right"),
    ] {
        let a = w
            .root
            .path()
            .join("captures")
            .join(format!("{name}-source.png"));
        let b = w
            .root
            .path()
            .join("captures")
            .join(format!("{name}-export.png"));
        let text = w.run(&w.runtime, None, ticks, held, Some(&a));
        assert!(text.contains("software: true"));
        assert_eq!(
            text,
            w.run(
                &bundle.join("run-collect-dodge"),
                Some(&bundle),
                ticks,
                held,
                Some(&b)
            )
        );
        assert_eq!(fs::read(&a).unwrap(), fs::read(&b).unwrap());
        let mut reader = png::Decoder::new(std::io::BufReader::new(fs::File::open(&a).unwrap()))
            .read_info()
            .unwrap();
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut pixels).unwrap();
        // Unique opaque lantern texels never appear in the primitive-square palette.
        let atlas_pixels = pixels[..info.buffer_size()]
            .chunks_exact(4)
            .filter(|p| {
                p[0].abs_diff(54) <= 3 && p[1].abs_diff(154) <= 3 && p[2].abs_diff(165) <= 3
            })
            .count();
        assert!(atlas_pixels > 10, "installed atlas pixels absent: {name}");
        if let Some(dest) = std::env::var_os("ORR_COLLECT_CAPTURES") {
            let dest = PathBuf::from(dest);
            fs::create_dir_all(&dest).unwrap();
            fs::copy(&a, dest.join(a.file_name().unwrap())).unwrap();
            fs::copy(&b, dest.join(b.file_name().unwrap())).unwrap();
        }
    }
    assert_eq!(snapshot(&bundle), bytes);
    assert_eq!(snapshot(&w.project), before);
}

#[cfg(feature = "collect-ui")]
#[test]
#[ignore = "requires exact UI-enabled built runtime/exporter, namespace isolation and real software GPU"]
fn collect_authored_ui_export_gpu_workflow() {
    let w = Work::new();
    let manifest = w.project.join("orr.project.json");
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    value["schema"] = 3.into();
    value["progress"] = serde_json::json!({"schema":1,"game_id":"12345678-1234-4234-8234-123456789abc","profile":"collect-dodge-highscore-v1"});
    value["entry"]["ui"] = serde_json::json!({"profile":"collect-authored-v1","document":"hud.json","font":{"package":"korean-game-ui","asset":"OrreryKoreanUI.otf"}});
    fs::write(&manifest, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    let mut document = orr_sample::authored_ui::Document::default_collect();
    // Authored variation survives exact same-document exported presentation.
    if let orr_sample::authored_ui::Kind::Label { text, .. } = &mut document
        .nodes
        .iter_mut()
        .find(|node| node.id == "score")
        .unwrap()
        .kind
    {
        *text = "게임 메뉴".into();
    }
    fs::write(w.project.join("hud.json"), document.to_bytes().unwrap()).unwrap();
    orr_package::Project::open(&w.project, orr_package::Runtime::content_only())
        .unwrap()
        .install(&[Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/game_ui_font")
            .canonicalize()
            .unwrap()])
        .unwrap();
    let before = snapshot(&w.project);
    let no_ui_source = PathBuf::from(
        std::env::var_os("ORR_COLLECT_NO_UI_RUNTIME")
            .expect("mandatory UI acceptance requires separately built no-UI runtime"),
    );
    let no_ui = w.root.path().join("tools/no-ui-collect");
    fs::copy(no_ui_source, &no_ui).unwrap();
    let rejected = w
        .isolated(&no_ui, None)
        .arg("--project")
        .arg(&w.project)
        .args(["--headless", "--ticks", "0"])
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("does not yet support"));
    let bundle = w.export();
    let bundle_before = snapshot(&bundle);
    assert_eq!(bundle_before["project/hud.json"], before["hud.json"]);
    assert!(bundle_before.keys().any(|p| p.ends_with("/OFL.txt")));
    assert!(bundle_before.keys().any(|p| p.ends_with("/COPYRIGHT.txt")));
    for (name, ticks, held) in [("playing", 0, ""), ("won", 16, "right"), ("lost", 600, "")] {
        let a = w.root.path().join(format!("{name}-ui-source.png"));
        let b = w.root.path().join(format!("{name}-ui-export.png"));
        w.run(&w.runtime, None, ticks, held, Some(&a));
        w.run(
            &bundle.join("run-collect-dodge"),
            Some(&bundle),
            ticks,
            held,
            Some(&b),
        );
        assert_eq!(fs::read(&a).unwrap(), fs::read(&b).unwrap());
        if let Some(dest) = std::env::var_os("ORR_COLLECT_CAPTURES") {
            fs::create_dir_all(&dest).unwrap();
            fs::copy(
                &b,
                PathBuf::from(dest).join(format!("{name}-authored-ui-export.png")),
            )
            .unwrap();
        }
    }
    let test_source = PathBuf::from(
        std::env::var_os("ORR_PROGRESS_TEST_BIN")
            .expect("mandatory actual UI restart/progress child test binary"),
    );
    fs::create_dir(w.root.path().join("test-tools")).unwrap();
    let child = w.root.path().join("test-tools/ui-progress-test");
    fs::copy(test_source, &child).unwrap();
    let data = w.root.path().join("user-data");
    fs::create_dir(&data).unwrap();
    for (mode, root, relocated) in [
        ("win", w.project.clone(), false),
        ("relaunch", bundle.join("project"), true),
    ] {
        let text = good(
            w.isolated(&child, if relocated { Some(&bundle) } else { None })
                .args([
                    "--ignored",
                    "--exact",
                    "collect_progress::tests::isolated_window_progress_child",
                    "--nocapture",
                ])
                .env("ORR_PROGRESS_TEST_PROJECT", root)
                .env("ORR_PROGRESS_TEST_MODE", mode)
                .env("XDG_DATA_HOME", &data)
                .env("HOME", w.root.path().join("home"))
                .output()
                .unwrap(),
        );
        assert!(
            text.contains("1 passed"),
            "actual window constructor/UI restart/progress hook must execute"
        );
    }
    assert!(data.join("orrery/games").is_dir());
    let ui_manifest = fs::read(&manifest).unwrap();
    value["entry"].as_object_mut().unwrap().remove("ui");
    fs::write(&manifest, serde_json::to_vec(&value).unwrap()).unwrap();
    let plain = w.root.path().join("plain.png");
    w.run(&w.runtime, None, 0, "", Some(&plain));
    let decode = |path: &Path| {
        let mut reader = png::Decoder::new(std::io::BufReader::new(fs::File::open(path).unwrap()))
            .read_info()
            .unwrap();
        let mut bytes = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut bytes).unwrap();
        bytes.truncate(info.buffer_size());
        bytes
    };
    let plain = decode(&plain);
    let drawn = decode(&w.root.path().join("playing-ui-source.png"));
    assert!(
        plain
            .chunks_exact(4)
            .zip(drawn.chunks_exact(4))
            .filter(|(a, b)| a != b)
            .count()
            > 100,
        "actual authored UI must change target pixels"
    );
    fs::write(&manifest, ui_manifest).unwrap();
    assert_eq!(snapshot(&w.project), before);
    assert_eq!(snapshot(&bundle), bundle_before);
}
