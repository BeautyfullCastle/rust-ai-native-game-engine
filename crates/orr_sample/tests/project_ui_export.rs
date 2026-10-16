//! Reproducible exports preserve and run an admitted Korean UI package closure.
//! `ORR_REQUIRE_PROJECT_ISOLATION=1` makes bwrap source hiding mandatory;
//! `ORR_REQUIRE_GPU=1` makes the generated runtime's UI capture mandatory.
#![cfg(all(
    feature = "project-export",
    target_os = "linux",
    target_arch = "x86_64"
))]
#![allow(clippy::disallowed_types)]

use std::{
    fs,
    io::Read,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

#[cfg(feature = "game-ui")]
use orr_sample::project_export::{export, ExportManifest, ExportOptions};
use sha2::{Digest, Sha256};
#[cfg(feature = "game-ui")]
use std::{
    collections::{BTreeMap, BTreeSet},
    process::Output,
};

#[path = "common/project.rs"]
mod project_fixture;
use project_fixture::ProjectFixture;

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);
const FONT_FILES: [&str; 5] = [
    "COPYRIGHT.txt",
    "OFL.txt",
    "OrreryKoreanUI.otf",
    "corpus.txt",
    "font-manifest.json",
];

struct Fixture {
    root: PathBuf,
    project: ProjectFixture,
    runtime: PathBuf,
    runtime_sha256: String,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "orr authored project Korean UI export {} {}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("empty cwd")).unwrap();
        fs::create_dir(root.join("supplied runtime with spaces")).unwrap();

        let mut project = ProjectFixture::new();
        let project_root = root.join("saved project with Korean UI");
        fs::rename(&project.root, &project_root).unwrap();
        project.root = project_root;
        install_saved_ui(&project.root);

        let runtime = root.join("supplied runtime with spaces/arena");
        fs::copy(env!("CARGO_BIN_EXE_arena"), &runtime).unwrap();
        let runtime_sha256 = hash_file(&runtime);
        Self {
            root,
            project,
            runtime,
            runtime_sha256,
        }
    }

    #[cfg(feature = "game-ui")]
    fn options(&self, name: &str) -> ExportOptions {
        ExportOptions {
            project: self.project.root.clone(),
            runtime: self.runtime.clone(),
            runtime_sha256: self.runtime_sha256.clone(),
            output: self.root.join(name),
            trusted_runtime: true,
            source_revision: Some("declared-ui-acceptance-revision".into()),
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn install_saved_ui(project: &Path) {
    let package = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/game_ui_font")
        .canonicalize()
        .unwrap();
    let installer = orr_package::Project::open_for_install(
        project,
        orr_package::Runtime::content_only().engine_version,
    )
    .unwrap();
    let lock = installer.install(&[package]).unwrap();
    assert_eq!(
        lock.direct.len(),
        2,
        "UI is an ordinary locked package selection"
    );
    assert!(lock.direct.contains_key("sample-sprites"));
    assert!(lock.direct.contains_key("korean-game-ui"));

    let manifest_path = project.join("orr.project.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest["entry"]["ui"] = serde_json::json!({
        "profile": "arena-korean-v1",
        "font": {
            "package": "korean-game-ui",
            "asset": "OrreryKoreanUI.otf"
        }
    });
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let opened =
        orr_package::Project::open(project, orr_sample::project_runtime::compiled_runtime())
            .unwrap();
    let active = opened.verify().unwrap();
    assert_eq!(active.direct.len(), 2);
    assert_eq!(active.packages.len(), 2);
    for asset in FONT_FILES {
        opened.read_asset("korean-game-ui", asset).unwrap();
    }
}

fn hash_file(path: &Path) -> String {
    let mut file = fs::File::open(path).unwrap();
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let bytes = file.read(&mut buffer).unwrap();
        if bytes == 0 {
            break;
        }
        hash.update(&buffer[..bytes]);
    }
    hash.finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(feature = "game-ui")]
fn file_paths(root: &Path) -> Vec<String> {
    fn visit(root: &Path, path: &Path, files: &mut Vec<String>) {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            let kind = entry.file_type().unwrap();
            assert!(
                !kind.is_symlink(),
                "unexpected link {}",
                entry.path().display()
            );
            if kind.is_dir() {
                visit(root, &entry.path(), files);
            } else {
                assert!(kind.is_file());
                files.push(
                    entry
                        .path()
                        .strip_prefix(root)
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .into(),
                );
            }
        }
    }
    let mut files = Vec::new();
    visit(root, root, &mut files);
    files.sort();
    files
}

#[cfg(feature = "game-ui")]
fn expected_project_files(project: &Path) -> Vec<String> {
    let opened =
        orr_package::Project::open(project, orr_sample::project_runtime::compiled_runtime())
            .unwrap();
    let lock = opened.verify().unwrap();
    let mut paths: BTreeSet<String> = [
        "orr.project.json".into(),
        "orr.packages.lock.json".into(),
        "arena.scene.yaml".into(),
        "arena.sprites.json".into(),
    ]
    .into();
    for package in lock.packages.values() {
        let object = format!(".orr/packages/objects/{}/", package.digest);
        paths.insert(format!("{object}orr.package.json"));
        for file in &package.manifest.files {
            paths.insert(format!("{object}{file}"));
        }
    }
    paths.into_iter().collect()
}

#[cfg(feature = "game-ui")]
fn success(output: Output) -> Output {
    assert!(
        output.status.success(),
        "status {}\nstdout {}\nstderr {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn independent(a: &Path, b: &Path) {
    let a = fs::metadata(a).unwrap();
    let b = fs::metadata(b).unwrap();
    assert_ne!((a.dev(), a.ino()), (b.dev(), b.ino()));
}

fn copy_project_only_runtime(fixture: &Fixture, source: &Path) -> PathBuf {
    let copied = fixture
        .root
        .join("copied trusted project-only runtime with spaces/arena");
    fs::create_dir_all(copied.parent().unwrap()).unwrap();
    fs::copy(source, &copied).unwrap();
    assert_eq!(hash_file(&copied), hash_file(source));
    independent(source, &copied);
    copied
}

fn assert_project_only_runtime_rejects_ui(fixture: &Fixture, copied: &Path) -> String {
    let output = Command::new(copied)
        .arg("--project")
        .arg(&fixture.project.root)
        .args(["--headless", "--ticks", "0"])
        .current_dir(fixture.root.join("empty cwd"))
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        stderr.contains("saved project declares UI") || stderr.contains("game-ui-enabled host"),
        "copied project-only runtime should reject the declared UI clearly: {stderr}"
    );
    stderr
}

#[cfg(feature = "game-ui")]
fn source_hiding_supported() -> bool {
    Command::new("bwrap")
        .args(["--ro-bind", "/", "/", "--", "/bin/true"])
        .output()
        .is_ok_and(|output| output.status.success())
}

#[cfg(feature = "game-ui")]
fn runtime_command(fixture: &Fixture, bundle: &Path) -> Command {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let hide_root = std::env::var_os("ORR_EXPORT_HIDE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| repository.clone())
        .canonicalize()
        .unwrap();
    assert!(repository.starts_with(&hide_root));
    assert!(!fixture.root.starts_with(&hide_root));

    let require_isolation = std::env::var_os("ORR_REQUIRE_PROJECT_ISOLATION").is_some();
    let use_bwrap = source_hiding_supported();
    assert!(
        !require_isolation || use_bwrap,
        "ORR_REQUIRE_PROJECT_ISOLATION=1 but bwrap source hiding is unavailable"
    );
    let launcher = bundle.join("run-arena");
    let mut command = if use_bwrap {
        let original_binary = Path::new(env!("CARGO_BIN_EXE_arena"))
            .canonicalize()
            .unwrap();
        let mut command = Command::new("bwrap");
        command
            .args(["--ro-bind", "/", "/", "--bind"])
            .arg(&fixture.root)
            .arg(&fixture.root)
            .arg("--tmpfs")
            .arg(&hide_root)
            .arg("--tmpfs")
            .arg(&fixture.project.root)
            .arg("--tmpfs")
            .arg(fixture.runtime.parent().unwrap());
        if !original_binary.starts_with(&hide_root) {
            command
                .arg("--tmpfs")
                .arg(original_binary.parent().unwrap());
        }
        command
            .args([
                "--",
                "/bin/sh",
                "-c",
                "test ! -e \"$1\" && test ! -e \"$2\" && test ! -e \"$3\" && test ! -e \"$4\" || exit 91; shift 4; exec \"$@\"",
                "sh",
            ])
            .arg(repository.join("Cargo.toml"))
            .arg(fixture.project.root.join("orr.project.json"))
            .arg(&fixture.runtime)
            .arg(&original_binary)
            .arg(&launcher);
        command
    } else {
        eprintln!("bwrap source hiding unavailable; direct relocated execution used");
        Command::new(launcher)
    };
    command
        .current_dir(fixture.root.join("empty cwd"))
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY");
    command
}

#[cfg(feature = "game-ui")]
fn run_bundle(fixture: &Fixture, bundle: &Path, capture: Option<&Path>) -> Output {
    let mut command = runtime_command(fixture, bundle);
    command.args(["--headless", "--ticks", "0"]);
    if let Some(capture) = capture {
        command.arg("--capture").arg(capture);
    }
    success(command.output().unwrap())
}

#[cfg(feature = "game-ui")]
fn retain_capture(source: &Path, destination_dir: &Path, name: &str) {
    assert!(
        destination_dir.is_dir(),
        "capture directory must already exist"
    );
    let mut source = fs::File::open(source).unwrap();
    let destination = destination_dir.join(name);
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&destination)
        .unwrap_or_else(|error| panic!("create capture {}: {error}", destination.display()));
    std::io::copy(&mut source, &mut output).unwrap();
}

#[cfg(feature = "game-ui")]
#[test]
fn generated_ui_exports_are_exact_relocatable_and_reproducible() {
    use orr_render::orr_rhi::{Wgpu, WgpuOptions};

    let fixture = Fixture::new();
    let expected_project = expected_project_files(&fixture.project.root);
    assert_eq!(expected_project.len(), 15);
    let source_paths = file_paths(&fixture.project.root);
    let source_before: BTreeMap<_, _> = source_paths
        .iter()
        .map(|path| {
            (
                path.clone(),
                fs::read(fixture.project.root.join(path)).unwrap(),
            )
        })
        .collect();
    assert!(expected_project
        .iter()
        .all(|path| source_before.contains_key(path)));
    let a = export(&fixture.options("first bundle with spaces")).unwrap();
    let b = export(&fixture.options("second bundle with spaces")).unwrap();
    assert_eq!(a.project_files, 15);
    assert_eq!(b.project_files, 15);
    assert_eq!(a.content_digest, b.content_digest);
    assert_eq!(a.manifest, b.manifest);
    assert_eq!(
        fs::read(a.output.join("orr.export.json")).unwrap(),
        fs::read(b.output.join("orr.export.json")).unwrap()
    );

    let manifest_bytes = fs::read(a.output.join("orr.export.json")).unwrap();
    let manifest: ExportManifest = serde_json::from_slice(&manifest_bytes).unwrap();
    let ui = manifest
        .payload
        .entry
        .ui
        .as_ref()
        .expect("export entry retains UI");
    assert_eq!(ui.profile, orr_package::ProjectUiProfile::ArenaKoreanV1);
    assert_eq!(ui.font.package, "korean-game-ui");
    assert_eq!(ui.font.asset, "OrreryKoreanUI.otf");
    assert_eq!(manifest.payload.packages.len(), 2);
    assert_eq!(
        manifest.payload.packages["korean-game-ui"],
        orr_package::Project::open(
            &fixture.project.root,
            orr_sample::project_runtime::compiled_runtime()
        )
        .unwrap()
        .verify()
        .unwrap()
        .packages["korean-game-ui"]
            .digest
    );

    let exported_project = a.output.join("project");
    let actual_project = file_paths(&exported_project);
    assert_eq!(actual_project, expected_project);
    let listed: Vec<_> = manifest
        .payload
        .files
        .iter()
        .map(|file| file.path.clone())
        .collect();
    assert!(listed.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(
        listed,
        file_paths(&a.output)
            .into_iter()
            .filter(|path| path != "orr.export.json")
            .collect::<Vec<_>>()
    );
    for file in &manifest.payload.files {
        let path = a.output.join(&file.path);
        assert_eq!(file.sha256, hash_file(&path), "{}", file.path);
        assert_eq!(file.bytes, fs::metadata(&path).unwrap().len());
        independent(&path, &b.output.join(&file.path));
    }

    let font_object = manifest.payload.packages["korean-game-ui"].clone();
    let font_prefix = format!(".orr/packages/objects/{font_object}/");
    for file in [
        "orr.package.json",
        FONT_FILES[0],
        FONT_FILES[1],
        FONT_FILES[2],
        FONT_FILES[3],
        FONT_FILES[4],
    ] {
        let relative = format!("{font_prefix}{file}");
        assert!(
            actual_project.contains(&relative),
            "missing exact font closure member {relative}"
        );
        let installed = fixture.project.root.join(&relative);
        let exported = exported_project.join(&relative);
        assert_eq!(
            fs::read(&installed).unwrap(),
            fs::read(&exported).unwrap(),
            "{relative}"
        );
    }
    for path in &expected_project {
        let bytes = &source_before[path];
        assert_eq!(
            fs::read(fixture.project.root.join(path)).unwrap(),
            *bytes,
            "export changed authoritative source bytes at {path}"
        );
        assert_eq!(
            fs::read(exported_project.join(path)).unwrap(),
            *bytes,
            "export payload differs from source bytes at {path}"
        );
    }
    assert_eq!(file_paths(&fixture.project.root), source_paths);
    for (path, bytes) in source_before {
        assert_eq!(
            fs::read(fixture.project.root.join(&path)).unwrap(),
            bytes,
            "export modified source path {path}"
        );
    }

    let relocated_a = fixture.root.join("relocated first game with spaces");
    let relocated_b = fixture.root.join("relocated second game with spaces");
    fs::rename(&a.output, &relocated_a).unwrap();
    fs::rename(&b.output, &relocated_b).unwrap();
    assert!(!a.output.exists());
    assert!(!b.output.exists());

    // Run the actual copied/trusted runtime twice from an unrelated empty cwd.
    // When software GPU is available, capture goes through the same project
    // compositor and Korean overlay callback; otherwise the simulation smoke
    // still verifies both relocated outputs and the require flag makes missing
    // render hardware fatal on the acceptance lane.
    let software_gpu = Wgpu::headless(WgpuOptions {
        force_software: true,
        ..Default::default()
    });
    let require_gpu = std::env::var_os("ORR_REQUIRE_GPU").is_some();
    let capture_dir = std::env::var_os("ORR_PROJECT_UI_CAPTURE_DIR").map(PathBuf::from);
    assert!(
        !(require_gpu || capture_dir.is_some()) || software_gpu.is_ok(),
        "GPU capture was required but software GPU is unavailable"
    );
    let do_capture = software_gpu.is_ok();
    if !do_capture {
        eprintln!(
            "software GPU unavailable; relocated headless UI runtime still runs twice without capture"
        );
    }
    drop(software_gpu);

    let capture_a = fixture.root.join("capture first with spaces.png");
    let capture_b = fixture.root.join("capture second with spaces.png");
    let first = run_bundle(
        &fixture,
        &relocated_a,
        do_capture.then_some(capture_a.as_path()),
    );
    let second = run_bundle(
        &fixture,
        &relocated_b,
        do_capture.then_some(capture_b.as_path()),
    );
    assert_eq!(first.stdout, second.stdout);
    assert!(String::from_utf8_lossy(&first.stdout).contains("project tick: 0 checksum:"));
    if do_capture {
        assert!(capture_a.is_file());
        assert!(capture_b.is_file());
        let image_a = fs::read(&capture_a).unwrap();
        let image_b = fs::read(&capture_b).unwrap();
        assert_eq!(
            image_a, image_b,
            "relocated UI captures must be deterministic"
        );
        let mut reader = png17::Decoder::new(fs::File::open(&capture_a).unwrap())
            .read_info()
            .unwrap();
        let mut pixels = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut pixels).unwrap();
        assert_eq!((info.width, info.height), (512, 512));
        assert_eq!(info.color_type, png17::ColorType::Rgba);
        assert_eq!(info.bit_depth, png17::BitDepth::Eight);
        // The fixed-time headless sizing passes must finish the title fade.
        // These three label rows are left of both authored sprites, so sprites
        // cannot make a nearly transparent menu pass this readability check.
        let readable_button_pixels = (260..335)
            .flat_map(|y| (90..175).map(move |x| (x, y)))
            .filter(|(x, y)| {
                let pixel = &pixels[(y * 512 + x) * 4..][..3];
                pixel.iter().all(|value| *value > 110)
                    && pixel.iter().max().unwrap() - pixel.iter().min().unwrap() < 12
            })
            .count();
        assert!(
            readable_button_pixels > 100,
            "title buttons must finish fading in: {readable_button_pixels} readable pixels"
        );
        if let Some(directory) = &capture_dir {
            retain_capture(&capture_a, directory, "project-ui-export-first.png");
            retain_capture(&capture_b, directory, "project-ui-export-second.png");
        }
        eprintln!(
            "relocated exported Korean UI capture: {} bytes",
            image_a.len()
        );
    }
}

#[cfg(feature = "game-ui")]
#[test]
fn independently_built_project_only_runtime_rejects_a_valid_saved_ui_project() {
    let Some(source) = std::env::var_os("ORR_PROJECT_ONLY_RUNTIME").map(PathBuf::from) else {
        assert!(
            std::env::var_os("ORR_REQUIRE_PROJECT_ONLY_RUNTIME").is_none(),
            "ORR_REQUIRE_PROJECT_ONLY_RUNTIME=1 requires ORR_PROJECT_ONLY_RUNTIME to name an independently built game-ui-free Arena binary"
        );
        eprintln!(
            "project-only runtime negative skipped; set ORR_PROJECT_ONLY_RUNTIME to a separately built game-ui-free Arena binary"
        );
        return;
    };
    let fixture = Fixture::new();
    let copied = copy_project_only_runtime(&fixture, &source);
    assert_project_only_runtime_rejects_ui(&fixture, &copied);
    eprintln!(
        "independently built and copied project-only runtime rejected the valid saved UI project"
    );
}

#[cfg(feature = "game-ui")]
#[test]
fn exporter_smoke_rejects_a_copied_project_only_runtime_for_saved_ui() {
    let Some(source) = std::env::var_os("ORR_PROJECT_ONLY_RUNTIME").map(PathBuf::from) else {
        assert!(
            std::env::var_os("ORR_REQUIRE_PROJECT_ONLY_RUNTIME").is_none(),
            "ORR_REQUIRE_PROJECT_ONLY_RUNTIME=1 requires ORR_PROJECT_ONLY_RUNTIME to name an independently built game-ui-free Arena binary"
        );
        eprintln!(
            "project-only exporter smoke negative skipped; set ORR_PROJECT_ONLY_RUNTIME to a separately built game-ui-free Arena binary"
        );
        return;
    };
    let fixture = Fixture::new();
    let copied = copy_project_only_runtime(&fixture, &source);
    let mut options = fixture.options("must not publish project-only UI bundle");
    options.runtime_sha256 = hash_file(&copied);
    options.runtime = copied;
    let error = export(&options).unwrap_err();
    assert!(!options.output.exists());
    assert!(
        error.contains("saved project declares UI") || error.contains("game-ui-enabled host"),
        "the copied trusted runtime smoke should reject this valid UI project: {error}"
    );
    eprintln!(
        "export smoke ran the copied game-ui-free binary and refused to publish its UI project"
    );
}

#[cfg(not(feature = "game-ui"))]
#[test]
fn copied_project_only_runtime_rejects_saved_ui_before_launch() {
    let fixture = Fixture::new();
    assert_eq!(hash_file(&fixture.runtime), fixture.runtime_sha256);
    let copied = copy_project_only_runtime(&fixture, &fixture.runtime);
    assert_project_only_runtime_rejects_ui(&fixture, &copied);
    eprintln!("copied trusted project-only runtime correctly rejected the saved UI entry");
}
