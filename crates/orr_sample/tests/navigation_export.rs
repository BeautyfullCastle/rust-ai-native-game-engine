//! Explicit source-hidden production-binary acceptance. No native-window claim.
#![cfg(all(
    feature = "navigation-project",
    feature = "project-create",
    feature = "project-export",
    target_os = "linux",
    target_arch = "x86_64"
))]
#![allow(clippy::disallowed_types)]
use orr_sample::project_create::{create, CreateOptions, NAVIGATION_TEMPLATE};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Output},
};
fn hash(path: &Path) -> String {
    let mut file = fs::File::open(path).unwrap();
    let mut digest = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let n = file.read(&mut buffer).unwrap();
        if n == 0 {
            break;
        }
        digest.update(&buffer[..n]);
    }
    digest
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn tree(root: &Path) -> BTreeMap<String, String> {
    fn walk(root: &Path, path: &Path, out: &mut BTreeMap<String, String>) {
        for e in fs::read_dir(path).unwrap() {
            let e = e.unwrap();
            let p = e.path();
            let t = e.file_type().unwrap();
            assert!(!t.is_symlink());
            if t.is_dir() {
                walk(root, &p, out)
            } else {
                assert!(t.is_file());
                out.insert(
                    p.strip_prefix(root).unwrap().to_str().unwrap().into(),
                    hash(&p),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}
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
fn required(name: &str) -> PathBuf {
    let p = PathBuf::from(
        std::env::var_os(name).unwrap_or_else(|| panic!("explicit acceptance requires {name}")),
    );
    assert!(p.is_absolute() && p.is_file());
    p.canonicalize().unwrap()
}
struct Lab {
    temp: tempfile::TempDir,
    workspace: PathBuf,
    project: PathBuf,
    runtime: PathBuf,
    exporter: PathBuf,
    originals: [PathBuf; 2],
}
impl Lab {
    fn new() -> Self {
        assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
        assert_eq!(
            std::env::var("ORR_REQUIRE_PROJECT_ISOLATION").as_deref(),
            Ok("1")
        );
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path();
        for name in [
            "tools",
            "empty",
            "home",
            "xdg-data",
            "xdg-config",
            "xdg-cache",
            "xdg-runtime",
            "captures",
            "readonly",
        ] {
            fs::create_dir(base.join(name)).unwrap();
        }
        let originals = [
            required("ORR_NAVIGATION_RUNTIME"),
            required("ORR_NAVIGATION_EXPORTER"),
        ];
        let runtime = base.join("tools/navigation_playground");
        let exporter = base.join("tools/orr_export_navigation");
        for (from, to) in originals.iter().zip([&runtime, &exporter]) {
            fs::copy(from, to).unwrap();
            assert_eq!(hash(from), hash(to));
        }
        let project = base.join("source-project");
        create(&CreateOptions {
            output: project.clone(),
            template: NAVIGATION_TEMPLATE.into(),
            seed: "export-detour".into(),
        })
        .unwrap();
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap();
        Self {
            temp,
            workspace,
            project,
            runtime,
            exporter,
            originals,
        }
    }
    fn isolated(&self, executable: &Path, bundle: Option<&Path>) -> Command {
        let base = self.temp.path();
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
        .arg(base)
        .arg(base)
        .arg("--tmpfs")
        .arg(&self.workspace)
        .arg("--ro-bind")
        .arg(base.join("readonly"))
        .arg(base.join("readonly"));
        for original in &self.originals {
            if !original.starts_with(&self.workspace) {
                c.args(["--ro-bind", "/dev/null"]).arg(original);
            }
        }
        if let Some(bundle) = bundle {
            c.arg("--tmpfs")
                .arg(&self.project)
                .arg("--tmpfs")
                .arg(base.join("tools"))
                .arg("--ro-bind")
                .arg(bundle)
                .arg(bundle);
        } else {
            c.arg("--ro-bind").arg(&self.project).arg(&self.project);
        }
        c.arg("--");
        if bundle.is_some() {
            c.args(["/bin/sh","-c","test ! -e \"$1\" && test ! -e \"$2\" && test ! -e \"$3\" && test ! -s \"$4\" && test ! -s \"$5\" || exit 91; shift 5; exec \"$@\"","sh"])
                .arg(self.workspace.join("Cargo.toml")).arg(self.project.join("orr.project.json")).arg(&self.runtime).arg(&self.originals[0]).arg(&self.originals[1]);
        }
        c.arg(executable)
            .current_dir(base.join("empty"))
            .env("HOME", base.join("home"))
            .env("XDG_DATA_HOME", base.join("xdg-data"))
            .env("XDG_CONFIG_HOME", base.join("xdg-config"))
            .env("XDG_CACHE_HOME", base.join("xdg-cache"))
            .env("XDG_RUNTIME_DIR", base.join("xdg-runtime"))
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY");
        c
    }
    fn runtime(
        &self,
        bundle: Option<&Path>,
        tick: u32,
        label: &str,
        restart: bool,
    ) -> (String, PathBuf) {
        let exe = bundle.map_or_else(
            || self.runtime.clone(),
            |b| b.join("run-navigation-playground"),
        );
        let mut c = self.isolated(&exe, bundle);
        if bundle.is_none() {
            c.arg("--project").arg(&self.project);
        }
        let capture = self
            .temp
            .path()
            .join("captures")
            .join(format!("{label}.png"));
        c.args(["--headless", "--ticks", &tick.to_string(), "--capture"])
            .arg(&capture)
            .args(["--capture-size", "960x720", "--software-gpu"]);
        if restart {
            c.args(["--restart-after", "37"]);
        }
        let output = c.output().unwrap();
        let state_lines = String::from_utf8_lossy(&output.stderr)
            .lines()
            .filter(|line| line.starts_with("navigation state checksum: "))
            .map(str::to_owned)
            .collect::<Vec<_>>();
        assert_eq!(
            state_lines.len(),
            1,
            "one authoritative navigation state digest"
        );
        let stdout = good(output);
        let adapters = stdout
            .lines()
            .filter(|line| line.starts_with("navigation capture adapter: "))
            .collect::<Vec<_>>();
        assert_eq!(
            adapters.len(),
            1,
            "production binary reports one actual capture adapter"
        );
        assert!(
            adapters[0].ends_with(" software: true"),
            "export/source capture must use a software adapter: {}",
            adapters[0]
        );
        (format!("{stdout}{}\n", state_lines[0]), capture)
    }
}
#[test]
#[ignore = "explicit prebuilt runtime/exporter, mandatory GPU and source isolation"]
fn navigation_real_export_source_hidden_readonly_gpu_parity_restart_and_tamper() {
    let lab = Lab::new();
    let base = lab.temp.path();
    let expected = orr_sample::navigation_project::PreparedProject::open(&lab.project).unwrap();
    let expected_initial = expected.scene().frame().checksum();
    let mut source = Vec::new();
    for (tick, label) in [(0, "start"), (24, "mid"), (300, "reached")] {
        let (stdout, png) = lab.runtime(None, tick, &format!("source-{label}"), false);
        assert!(stdout.contains(&format!(
            "navigation initial checksum: 0x{expected_initial:016x}"
        )));
        source.push((tick, label, stdout, fs::read(png).unwrap()));
    }
    let output = base.join("built-bundle");
    let mut c = lab.isolated(&lab.exporter, None);
    c.arg("--project")
        .arg(&lab.project)
        .arg("--runtime")
        .arg(&lab.runtime)
        .arg("--runtime-sha256")
        .arg(hash(&lab.runtime))
        .arg("--output")
        .arg(&output)
        .arg("--trusted-runtime");
    if let Ok(sha) = std::env::var("NAVIGATION_SOURCE_SHA") {
        c.arg("--source-revision").arg(sha);
    }
    good(c.output().unwrap());
    let bundle = base.join("readonly/relocated navigation");
    fs::rename(output, &bundle).unwrap();
    let before = tree(&bundle);
    assert!(before.contains_key("project/terrain.orrt"));
    assert!(!before
        .keys()
        .any(|name| name.contains("sprites") || name.contains("models")));
    for (tick, label, stdout, png) in source {
        let (relocated, capture) =
            lab.runtime(Some(&bundle), tick, &format!("export-{label}"), false);
        assert_eq!(
            relocated, stdout,
            "source/export checksum and adapter output"
        );
        assert_eq!(
            fs::read(&capture).unwrap(),
            png,
            "source/export production image at {tick}"
        );
    }
    let (restart, capture) = lab.runtime(Some(&bundle), 24, "export-restarted", true);
    assert!(restart.contains(&format!(
        "navigation initial checksum: 0x{expected_initial:016x}"
    )));
    assert_eq!(
        fs::read(capture).unwrap(),
        fs::read(base.join("captures/export-mid.png")).unwrap(),
        "restart restores admitted Frame and camera"
    );
    assert_eq!(
        tree(&bundle),
        before,
        "read-only execution changes no bundle bytes"
    );
    let terrain = bundle.join("project/terrain.orrt");
    let original = fs::read(&terrain).unwrap();
    for tamper in [true, false] {
        if tamper {
            fs::write(&terrain, b"tampered terrain").unwrap();
        } else {
            fs::remove_file(&terrain).unwrap();
        }
        let mut c = lab.isolated(&bundle.join("run-navigation-playground"), Some(&bundle));
        c.args(["--headless", "--ticks", "0"]);
        let out = c.output().unwrap();
        assert!(!out.status.success());
        assert!(out.stdout.is_empty(), "reject before initial App output");
        assert!(!out.stderr.is_empty());
        fs::write(&terrain, &original).unwrap();
    }
    assert_eq!(tree(&bundle), before);
    if let Some(directory) = std::env::var_os("NAVIGATION_EXPORT_CAPTURE_DIR") {
        let directory = PathBuf::from(directory);
        fs::create_dir_all(&directory).unwrap();
        for e in fs::read_dir(base.join("captures")).unwrap() {
            let e = e.unwrap();
            let target = directory.join(e.file_name());
            assert!(!target.exists());
            fs::copy(e.path(), target).unwrap();
        }
        for (name, bytes) in [
            ("bundle-hashes-before.json", serde_json::to_vec_pretty(&before).unwrap()),
            ("bundle-hashes-after.json", serde_json::to_vec_pretty(&tree(&bundle)).unwrap()),
            ("orr.export.json", fs::read(bundle.join("orr.export.json")).unwrap()),
            ("result.json", serde_json::to_vec_pretty(&serde_json::json!({"source_sha":std::env::var("NAVIGATION_SOURCE_SHA").ok(),"initial_checksum":format!("0x{expected_initial:016x}"),"source_hidden":true,"readonly":true,"frame_and_navigation_state_parity_ticks":[0,24,300],"restart_after":37,"restart_observed_tick":24,"tamper_rejected_before_app":true,"missing_rejected_before_app":true,"bundle_hashes_unchanged":true})).unwrap()),
        ] { let path=directory.join(name); assert!(!path.exists()); fs::write(path,bytes).unwrap(); }
    }
}
