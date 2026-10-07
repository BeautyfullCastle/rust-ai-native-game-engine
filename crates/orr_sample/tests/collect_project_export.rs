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
    let trace_number = std::cell::Cell::new(0u32);
    let run_no_profile = |c: &mut Command| {
        c.env("XDG_DATA_HOME", &data)
            .env("HOME", w.root.path().join("isolated home"));
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
        assert!(
            trace.contains("execve("),
            "syscall observation did not execute"
        );
        assert!(
            !trace.contains(data.to_str().unwrap()) && !trace.contains("isolated progress data"),
            "noninteractive route touched progress root: {trace}"
        );
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
