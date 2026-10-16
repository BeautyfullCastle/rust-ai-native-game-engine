//! Exported folders are verified as delivered, through their generated launcher.
//! ORR_REQUIRE_PROJECT_ISOLATION=1 requires Linux bubblewrap source hiding.
//! ORR_EXPORT_HIDE_ROOT may broaden repository hiding to the whole workspace.
//! ORR_REQUIRE_GPU=1 makes the relocated runtime's real --capture path mandatory.
#![cfg(all(
    feature = "project-export",
    target_os = "linux",
    target_arch = "x86_64"
))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};

use orr_sample::project_export::{export, ExportManifest, ExportOptions};
use orr_sample::project_runtime::PreparedRuntime;
use sha2::{Digest, Sha256};

#[path = "common/project.rs"]
mod project_fixture;
use project_fixture::{copy_tree, ProjectFixture};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);
const OBJECT: &str = "952838c26cb40890a4c9b1bb9a3c25c5e2430bca3b33b4064b9d79efd7432f86";

struct Fixture {
    root: PathBuf,
    project: ProjectFixture,
    runtime: PathBuf,
    runtime_sha256: String,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "orr export acceptance {} {}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let project = ProjectFixture::new();
        fs::create_dir(root.join("supplied runtime")).unwrap();
        fs::create_dir(root.join("empty cwd")).unwrap();
        let runtime = root.join("supplied runtime/arena");
        fs::copy(env!("CARGO_BIN_EXE_arena"), &runtime).unwrap();
        let runtime_sha256 = hash_file(&runtime);
        Self {
            root,
            project,
            runtime,
            runtime_sha256,
        }
    }
    fn options(&self, name: &str) -> ExportOptions {
        ExportOptions {
            project: self.project.root.clone(),
            runtime: self.runtime.clone(),
            runtime_sha256: self.runtime_sha256.clone(),
            output: self.root.join(name),
            trusted_runtime: true,
            source_revision: Some("declared-test-revision".into()),
        }
    }
    fn cli(&self, output: &Path) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_orr_export_arena"));
        command
            .arg("--project")
            .arg(&self.project.root)
            .arg("--runtime")
            .arg(&self.runtime)
            .arg("--runtime-sha256")
            .arg(&self.runtime_sha256)
            .arg("--output")
            .arg(output)
            .arg("--trusted-runtime")
            .current_dir(self.root.join("empty cwd"))
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY");
        command
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
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
fn snapshot(root: &Path) -> BTreeMap<String, Vec<u8>> {
    file_paths(root)
        .into_iter()
        .map(|path| {
            let bytes = fs::read(root.join(&path)).unwrap();
            (path, bytes)
        })
        .collect()
}
fn expected_project_files() -> Vec<String> {
    let mut paths: Vec<String> = [
        "orr.project.json",
        "orr.packages.lock.json",
        "arena.scene.yaml",
        "arena.sprites.json",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    for asset in [
        "orr.package.json",
        "LICENSE.txt",
        "lantern_keeper.png",
        "lantern_keeper.rgba",
        "sprites.json",
    ] {
        paths.push(format!(".orr/packages/objects/{OBJECT}/{asset}"));
    }
    paths.sort();
    paths
}
fn success(output: Output) -> String {
    assert!(
        output.status.success(),
        "status {}\nstdout {}\nstderr {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}
fn error(output: Output) -> String {
    assert!(
        !output.status.success(),
        "unexpected success: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    String::from_utf8(output.stderr).unwrap()
}
fn assert_independent(a: &Path, b: &Path) {
    let a = fs::metadata(a).unwrap();
    let b = fs::metadata(b).unwrap();
    assert_ne!(
        (a.dev(), a.ino()),
        (b.dev(), b.ino()),
        "export must copy to an independent inode"
    );
}

#[test]
fn export_is_an_exact_reproducible_independent_active_closure() {
    let fixture = Fixture::new();
    for relative in [
        ".orr/packages/cache/unused",
        ".orr/packages/objects/inactive/sentinel",
        ".orr/editor/session.json",
        "root-sentinel",
    ] {
        let path = fixture.project.root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"not active project content").unwrap();
    }
    let before = snapshot(&fixture.project.root);
    let a = export(&fixture.options("first bundle")).unwrap();
    let b = export(&fixture.options("second bundle")).unwrap();
    assert_eq!(a.project_files, 9);
    assert_eq!(b.project_files, 9);
    assert_eq!(a.manifest, b.manifest);
    assert_eq!(a.content_digest, b.content_digest);
    assert_eq!(a.content_digest, a.manifest.content_digest);
    assert_eq!(
        fs::read(a.output.join("orr.export.json")).unwrap(),
        fs::read(b.output.join("orr.export.json")).unwrap()
    );
    let manifest: ExportManifest =
        serde_json::from_slice(&fs::read(a.output.join("orr.export.json")).unwrap()).unwrap();
    assert_eq!(manifest, a.manifest);
    assert_eq!(manifest.schema, 1);
    let mut content_hash = Sha256::new();
    content_hash.update(b"orrery.arena.export.content.v1\0");
    content_hash.update(serde_json::to_vec(&manifest.payload).unwrap());
    assert_eq!(
        manifest.content_digest,
        content_hash
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );
    assert_eq!(manifest.payload.runtime.sha256, fixture.runtime_sha256);
    assert_eq!(
        manifest.payload.runtime.bytes,
        fs::metadata(&fixture.runtime).unwrap().len()
    );
    assert_eq!(
        manifest.payload.declared.source_revision.as_deref(),
        Some("declared-test-revision")
    );
    assert_eq!(
        manifest.payload.initial_checksum,
        format!(
            "0x{:016x}",
            PreparedRuntime::open(&fixture.project.root)
                .unwrap()
                .initial_frame()
                .checksum()
        )
    );
    assert_eq!(manifest.payload.packages.len(), 1);
    assert_eq!(manifest.payload.packages["sample-sprites"], OBJECT);
    assert_eq!(
        file_paths(&a.output.join("project")),
        expected_project_files()
    );
    let paths = file_paths(&a.output);
    assert_eq!(paths.len(), 12);
    assert_eq!(manifest.payload.files.len(), 11);
    let listed: Vec<_> = manifest
        .payload
        .files
        .iter()
        .map(|file| file.path.clone())
        .collect();
    assert_eq!(
        listed,
        paths
            .iter()
            .filter(|path| path.as_str() != "orr.export.json")
            .cloned()
            .collect::<Vec<_>>()
    );
    for file in &manifest.payload.files {
        let path = a.output.join(&file.path);
        assert_eq!(file.sha256, hash_file(&path), "{}", file.path);
        assert_eq!(file.bytes, fs::metadata(&path).unwrap().len());
        assert_eq!(
            file.mode,
            fs::metadata(&path).unwrap().permissions().mode() & 0o777
        );
        assert_eq!(
            file.mode,
            if file.path == "bin/arena" || file.path == "run-arena" {
                0o755
            } else {
                0o644
            }
        );
        assert_independent(&path, &b.output.join(&file.path));
    }
    for path in expected_project_files() {
        assert_eq!(
            fs::read(a.output.join("project").join(&path)).unwrap(),
            before[&path]
        );
        assert_independent(
            &fixture.project.root.join(&path),
            &a.output.join("project").join(path),
        );
    }
    assert_independent(&fixture.runtime, &a.output.join("bin/arena"));
    assert_eq!(
        hash_file(&a.output.join("bin/arena")),
        fixture.runtime_sha256
    );
    assert_eq!(hash_file(&fixture.runtime), fixture.runtime_sha256);
    assert_eq!(snapshot(&fixture.project.root), before);
    let json = String::from_utf8(fs::read(a.output.join("orr.export.json")).unwrap()).unwrap();
    for absolute in [
        &fixture.root,
        &fixture.project.root,
        Path::new(env!("CARGO_MANIFEST_DIR")),
    ] {
        assert!(
            !json.contains(absolute.to_str().unwrap()),
            "manifest must not depend on host paths"
        );
    }
}

#[test]
fn cli_exports_and_rejects_existing_destinations_without_changes() {
    let fixture = Fixture::new();
    let output = fixture.root.join("CLI bundle with spaces");
    let stdout = success(fixture.cli(&output).output().unwrap());
    assert!(stdout.contains("export project files: 9"));
    assert!(stdout.contains("export content digest: "));
    let before = snapshot(&output);
    assert!(!error(fixture.cli(&output).output().unwrap()).is_empty());
    assert_eq!(snapshot(&output), before);
    for name in ["existing empty directory", "existing file"] {
        let options = fixture.options(name);
        if name.ends_with("directory") {
            fs::create_dir(&options.output).unwrap();
        } else {
            fs::write(&options.output, b"destination sentinel").unwrap();
        }
        assert!(export(&options).is_err());
        if options.output.is_file() {
            assert_eq!(fs::read(options.output).unwrap(), b"destination sentinel");
        } else {
            assert!(file_paths(&options.output).is_empty());
        }
    }
}

#[test]
fn cli_usage_requires_explicit_trust_and_unambiguous_options() {
    let cwd = Path::new(env!("CARGO_MANIFEST_DIR"));
    let help = success(
        Command::new(env!("CARGO_BIN_EXE_orr_export_arena"))
            .arg("--help")
            .current_dir(cwd)
            .output()
            .unwrap(),
    );
    assert!(help.contains("hash and smoke run do not establish trust"));
    for args in [
        vec![],
        vec![
            "--project",
            "missing",
            "--runtime",
            "missing",
            "--runtime-sha256",
            "0",
            "--output",
            "missing",
        ],
        vec!["--trusted-runtime", "--unknown"],
        vec!["--trusted-runtime", "--trusted-runtime"],
        vec!["--trusted-runtime", "--project"],
        vec!["--trusted-runtime", "--project", "--runtime"],
        vec![
            "--trusted-runtime",
            "--project",
            "missing",
            "--project",
            "missing-again",
        ],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_orr_export_arena"))
            .args(&args)
            .current_dir(cwd)
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(2),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!error(output).is_empty());
    }
}

#[test]
fn scene_only_export_preserves_absent_lock_without_installation() {
    let fixture = Fixture::new();
    let manifest = fixture.project.root.join("orr.project.json");
    let mut project: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    project["entry"].as_object_mut().unwrap().remove("sprites");
    fs::write(&manifest, serde_json::to_vec_pretty(&project).unwrap()).unwrap();
    fs::remove_file(fixture.project.root.join("orr.packages.lock.json")).unwrap();
    fs::remove_dir_all(fixture.project.root.join(".orr")).unwrap();
    let before = snapshot(&fixture.project.root);
    let report = export(&fixture.options("scene only")).unwrap();
    assert_eq!(report.project_files, 2);
    assert!(report.manifest.payload.packages.is_empty());
    assert_eq!(
        file_paths(&report.output.join("project")),
        ["arena.scene.yaml", "orr.project.json"]
    );
    assert!(!report
        .output
        .join("project/orr.packages.lock.json")
        .exists());
    assert_eq!(snapshot(&fixture.project.root), before);
    let stdout = success(
        Command::new(report.output.join("run-arena"))
            .args(["--headless", "--ticks", "0"])
            .current_dir(fixture.root.join("empty cwd"))
            .output()
            .unwrap(),
    );
    assert!(stdout.contains("project tick: 0 checksum:"));
}

#[test]
fn admission_hash_and_target_failures_never_publish_a_folder() {
    let fixture = Fixture::new();
    for (name, hash) in [
        ("wrong hash", "0".repeat(64)),
        ("malformed hash", "not-a-sha256".into()),
    ] {
        let mut options = fixture.options(name);
        options.runtime_sha256 = hash;
        assert!(export(&options).is_err());
        assert!(!options.output.exists());
    }
    let mut untrusted = fixture.options("no explicit trust");
    untrusted.trusted_runtime = false;
    assert!(export(&untrusted).is_err());
    assert!(!untrusted.output.exists());
    let malformed = fixture.root.join("malformed ELF");
    // This is deliberately non-executable data. Admission must reject it before
    // spawning; all binaries actually executed by this suite are built Arena.
    fs::write(&malformed, b"not an ELF executable").unwrap();
    fs::set_permissions(&malformed, fs::Permissions::from_mode(0o755)).unwrap();
    let mut options = fixture.options("invalid ELF output");
    options.runtime = malformed;
    options.runtime_sha256 = hash_file(&options.runtime);
    assert!(export(&options).unwrap_err().contains("ELF"));
    assert!(!options.output.exists());
    let scene = fixture.project.root.join("arena.scene.yaml");
    fs::write(&scene, "schema: orr.scene/1\nentities: [broken").unwrap();
    let before = snapshot(&fixture.project.root);
    let options = fixture.options("invalid scene output");
    assert!(export(&options).is_err());
    assert!(!options.output.exists());
    assert_eq!(snapshot(&fixture.project.root), before);
}

#[test]
fn invalid_declared_revision_and_source_output_overlap_are_rejected() {
    let fixture = Fixture::new();
    for revision in ["has whitespace", "has\nnewline", "non-ascii-한글", ""] {
        let mut options = fixture.options("invalid provenance");
        options.source_revision = Some(revision.into());
        assert!(export(&options).is_err(), "{revision:?}");
        assert!(!options.output.exists());
    }
    let before = snapshot(&fixture.project.root);
    let mut options = fixture.options("unused");
    options.output = fixture.project.root.join("export inside source");
    assert!(export(&options).is_err());
    assert!(!options.output.exists());
    assert_eq!(snapshot(&fixture.project.root), before);
}

#[derive(Debug, PartialEq, Eq)]
struct State {
    initial: String,
    tick: u32,
    checksum: String,
    positions: BTreeMap<String, (i64, i64)>,
}
fn state(stdout: &str) -> State {
    let initial = stdout
        .lines()
        .find_map(|line| line.strip_prefix("project initial checksum: "))
        .unwrap()
        .to_owned();
    let tick = stdout
        .lines()
        .find_map(|line| line.strip_prefix("project tick: "))
        .unwrap();
    let (tick, checksum) = tick.split_once(" checksum: ").unwrap();
    let positions = stdout
        .lines()
        .filter_map(|line| {
            let line = line.strip_prefix("project entity: ")?;
            let (guid, rest) = line.split_once(" handle:").unwrap();
            let (_, position) = rest.split_once(" position:").unwrap();
            let (x, y) = position.split_once(',').unwrap();
            Some((guid.into(), (x.parse().unwrap(), y.parse().unwrap())))
        })
        .collect();
    State {
        initial,
        tick: tick.parse().unwrap(),
        checksum: checksum.into(),
        positions,
    }
}
fn source_run(fixture: &Fixture, ticks: &str, hold: &str) -> String {
    success(
        Command::new(&fixture.runtime)
            .arg("--project")
            .arg(&fixture.project.root)
            .args(["--headless", "--ticks", ticks, "--hold", hold])
            .current_dir(fixture.root.join("empty cwd"))
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .output()
            .unwrap(),
    )
}
fn relocated_run(
    fixture: &Fixture,
    bundle: &Path,
    ticks: &str,
    hold: &str,
    capture: Option<&Path>,
) -> String {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let hide_root = std::env::var_os("ORR_EXPORT_HIDE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| repository.clone())
        .canonicalize()
        .unwrap();
    assert!(
        repository.starts_with(&hide_root),
        "hidden workspace must contain this repository"
    );
    assert!(
        !fixture.root.starts_with(&hide_root),
        "acceptance folder must be outside the hidden workspace"
    );
    let launcher = bundle.join("run-arena");
    let mut command = if std::env::var_os("ORR_REQUIRE_PROJECT_ISOLATION").is_some() {
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
        let original_binary = Path::new(env!("CARGO_BIN_EXE_arena"))
            .canonicalize()
            .unwrap();
        if !original_binary.starts_with(&hide_root) {
            command
                .arg("--tmpfs")
                .arg(original_binary.parent().unwrap());
        }
        command.args(["--", "/bin/sh", "-c", "test ! -e \"$1\" && test ! -e \"$2\" && test ! -e \"$3\" && test ! -e \"$4\" || exit 91; shift 4; exec \"$@\"", "sh"])
            .arg(repository.join("Cargo.toml"))
            .arg(fixture.project.root.join("orr.project.json"))
            .arg(&fixture.runtime)
            .arg(&original_binary)
            .arg(&launcher);
        command
    } else {
        eprintln!("source-hiding acceptance not requested; set ORR_REQUIRE_PROJECT_ISOLATION=1 to require it");
        Command::new(&launcher)
    };
    command
        .args(["--headless", "--ticks", ticks, "--hold", hold])
        .current_dir(fixture.root.join("empty cwd"))
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY");
    if let Some(path) = capture {
        command.arg("--capture").arg(path);
    }
    success(command.output().unwrap())
}
fn relocated_bundle(fixture: &Fixture) -> PathBuf {
    let report = export(&fixture.options("original export folder")).unwrap();
    let relocated = fixture.root.join("relocated game with spaces");
    fs::rename(&report.output, &relocated).unwrap();
    assert!(!report.output.exists());
    relocated
}

#[test]
fn generated_folder_relocates_with_initial_movement_fire_and_relaunch_parity() {
    let fixture = Fixture::new();
    let before = snapshot(&fixture.project.root);
    let expected: Vec<_> = [
        ("0", "idle"),
        ("2", "right"),
        ("3", "right"),
        ("3", "right,fire"),
    ]
    .into_iter()
    .map(|(ticks, hold)| (ticks, hold, source_run(&fixture, ticks, hold)))
    .collect();
    let bundle = relocated_bundle(&fixture);
    for (ticks, hold, stdout) in &expected {
        assert_eq!(&relocated_run(&fixture, &bundle, ticks, hold, None), stdout);
    }
    let initial = state(&expected[0].2);
    assert_eq!(initial.tick, 0);
    assert_eq!(initial.initial, initial.checksum);
    assert_eq!(initial.positions.len(), 2);
    assert_eq!(initial.positions["e_00000001"], (-60 * 65536, 0));
    assert_eq!(initial.positions["e_00000002"], (60 * 65536, 0));
    let moved = state(&expected[1].2);
    assert_eq!(moved.tick, 2);
    assert_eq!(moved.initial, initial.initial);
    assert_eq!(moved.positions["e_00000001"], (-48 * 65536, 0));
    assert_eq!(
        moved.positions["e_00000002"],
        initial.positions["e_00000002"]
    );
    let no_fire = state(&expected[2].2);
    let fire = state(&expected[3].2);
    assert_eq!(fire.positions, no_fire.positions);
    assert_ne!(
        fire.checksum, no_fire.checksum,
        "held fire must change the actual simulation"
    );
    assert_eq!(
        relocated_run(&fixture, &bundle, "3", "right,fire", None),
        expected[3].2
    );
    assert_eq!(
        relocated_run(&fixture, &bundle, "0", "idle", None),
        expected[0].2
    );
    assert_eq!(snapshot(&fixture.project.root), before);
    assert_eq!(hash_file(&fixture.runtime), fixture.runtime_sha256);
}

fn read_png(path: &Path) -> (u32, u32, Vec<u8>) {
    let mut reader = png17::Decoder::new(fs::File::open(path).unwrap())
        .read_info()
        .unwrap();
    let mut pixels = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut pixels).unwrap();
    assert_eq!(info.color_type, png17::ColorType::Rgba);
    assert_eq!(info.bit_depth, png17::BitDepth::Eight);
    pixels.truncate(info.buffer_size());
    (info.width, info.height, pixels)
}
fn captured_sprite_texels(
    pixels: &[u8],
    camera: &orr_render::Camera,
    position: [f32; 2],
    asset: &orr_sample::project_sprites::Asset,
    region: u32,
) -> usize {
    let region = asset.document.region(region).unwrap();
    let mut opaque = Vec::new();
    for y in region.y..region.y + region.height {
        for x in region.x..region.x + region.width {
            let offset = ((y * asset.document.atlas().width + x) * 4) as usize;
            let texel: [u8; 4] = asset.rgba[offset..offset + 4].try_into().unwrap();
            if texel[3] == 255 {
                opaque.push(texel);
            }
        }
    }
    assert!(!opaque.is_empty());
    let center = camera.world_to_screen(position, (512, 512));
    let radius = 16.0 * camera.pixels_per_unit(512, 512) + 2.0;
    let mut count = 0;
    for y in (center[1] - radius).max(0.0) as usize..(center[1] + radius).min(512.0) as usize {
        for x in (center[0] - radius).max(0.0) as usize..(center[0] + radius).min(512.0) as usize {
            let texel = &pixels[(y * 512 + x) * 4..(y * 512 + x) * 4 + 4];
            if opaque
                .iter()
                .any(|expected| texel.iter().zip(expected).all(|(a, b)| a.abs_diff(*b) <= 3))
            {
                count += 1;
            }
        }
    }
    count
}

fn target_sprite_centroid_x(pixels: &[u8], asset: &orr_sample::project_sprites::Asset) -> f32 {
    let region = asset.document.region(20).unwrap();
    let mut opaque = Vec::new();
    for y in region.y..region.y + region.height {
        for x in region.x..region.x + region.width {
            let offset = ((y * asset.document.atlas().width + x) * 4) as usize;
            let texel: [u8; 4] = asset.rgba[offset..offset + 4].try_into().unwrap();
            if texel[3] == 255 {
                opaque.push(texel);
            }
        }
    }
    let mut x_sum = 0_u32;
    let mut count = 0_u32;
    // This broad target-only rectangle includes both old and followed locations,
    // so a failed camera follow cannot pass by overlapping a narrow sample box.
    for y in 220..292 {
        for x in 330..430 {
            let texel = &pixels[(y * 512 + x) * 4..(y * 512 + x) * 4 + 4];
            if opaque
                .iter()
                .any(|expected| texel.iter().zip(expected).all(|(a, b)| a.abs_diff(*b) <= 3))
            {
                x_sum += x as u32;
                count += 1;
            }
        }
    }
    assert!(
        count > 20,
        "target silhouette was absent from the runtime capture"
    );
    x_sum as f32 / count as f32
}

#[test]
fn relocated_launcher_captures_actual_installed_sprites_and_camera_follow() {
    if std::env::var_os("ORR_REQUIRE_GPU").is_none() {
        match orr_render::orr_rhi::Wgpu::headless(orr_render::orr_rhi::WgpuOptions::default()) {
            Ok(_) => {}
            Err(error) => {
                eprintln!("SKIP optional GPU capture: {error}");
                return;
            }
        }
    }
    let fixture = Fixture::new();
    let before = snapshot(&fixture.project.root);
    let expected_initial = state(&source_run(&fixture, "0", "idle"));
    let expected_moved = state(&source_run(&fixture, "2", "right"));
    let prepared = PreparedRuntime::open(&fixture.project.root).unwrap();
    let (_, presentation) = prepared.into_parts().unwrap();
    let asset = presentation
        .assets()
        .get(&("sample-sprites".into(), "sprites.json".into()))
        .unwrap();
    let bundle = relocated_bundle(&fixture);
    let initial_png = fixture.root.join("relocated initial.png");
    let moved_png = fixture.root.join("relocated followed.png");
    let initial_stdout = relocated_run(&fixture, &bundle, "0", "idle", Some(&initial_png));
    let moved_stdout = relocated_run(&fixture, &bundle, "2", "right", Some(&moved_png));
    assert!(initial_stdout.contains("project capture adapter:"));
    assert!(moved_stdout.contains("project capture adapter:"));
    assert_eq!(state(&initial_stdout), expected_initial);
    assert_eq!(state(&moved_stdout), expected_moved);
    let (width, height, initial) = read_png(&initial_png);
    let (moved_width, moved_height, moved) = read_png(&moved_png);
    assert_eq!((width, height), (512, 512));
    assert_eq!((moved_width, moved_height), (512, 512));
    assert_ne!(initial, moved);
    let initial_camera = orr_render::Camera::new([-60.0, 0.0], 240.0);
    let moved_camera = orr_render::Camera::new([-48.0, 0.0], 240.0);
    for (label, pixels, camera, position, region) in [
        ("initial hero", &initial, &initial_camera, [-60.0, 0.0], 10),
        ("initial target", &initial, &initial_camera, [60.0, 0.0], 20),
        ("moved hero", &moved, &moved_camera, [-48.0, 0.0], 20),
        (
            "follow-shifted target",
            &moved,
            &moved_camera,
            [60.0, 0.0],
            20,
        ),
    ] {
        let count = captured_sprite_texels(pixels, camera, position, asset, region);
        assert!(
            count > 20,
            "{label} has only {count} opaque installed atlas pixels"
        );
        eprintln!("export runtime GPU {label}: {count} opaque atlas pixels");
    }
    let target_shift =
        target_sprite_centroid_x(&initial, asset) - target_sprite_centroid_x(&moved, asset);
    assert!((10.0..16.0).contains(&target_shift), "actual stationary sprite should move about 12.8 capture pixels with camera follow, got {target_shift}");
    eprintln!("export runtime GPU target capture shift: {target_shift} pixels");
    if let Some(directory) = std::env::var_os("ORR_PROJECT_CAPTURE_DIR") {
        let directory = PathBuf::from(directory);
        fs::create_dir_all(&directory).unwrap();
        fs::copy(&initial_png, directory.join("export-relocated-initial.png")).unwrap();
        fs::copy(&moved_png, directory.join("export-relocated-followed.png")).unwrap();
    }
    assert_eq!(snapshot(&fixture.project.root), before);
}

#[test]
fn complete_active_dependency_assets_are_preserved_even_without_sprite_bindings() {
    let fixture = Fixture::new();
    let root = fixture.root.join("dependency source");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("orr.package.json"), r#"{"schema":1,"name":"unused-content","version":"1.0.0","engine":"^0.0.1","capabilities":[],"dependencies":{},"files":["LICENSE.txt","unused.txt"]}"#).unwrap();
    fs::write(root.join("LICENSE.txt"), "Unused content license\n").unwrap();
    fs::write(
        root.join("unused.txt"),
        "Keep me although no sprite uses me\n",
    )
    .unwrap();
    let source = fixture.root.join("dependent source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("orr.package.json"), r#"{"schema":1,"name":"dependent-content","version":"1.0.0","engine":"^0.0.1","capabilities":[],"dependencies":{"unused-content":"1.0.0"},"files":["asset.txt"]}"#).unwrap();
    fs::write(source.join("asset.txt"), "Direct package content\n").unwrap();
    let installer = orr_package::Project::open(
        &fixture.project.root,
        orr_sample::project_runtime::compiled_runtime(),
    )
    .unwrap();
    installer
        .install_with_dependencies(&[source], &[root])
        .unwrap();
    let lock = installer.verify().unwrap();
    assert!(!lock.direct.contains_key("unused-content"));
    assert!(lock.packages.contains_key("unused-content"));
    let before = snapshot(&fixture.project.root);
    let report = export(&fixture.options("dependency bundle")).unwrap();
    for (name, package) in &lock.packages {
        assert_eq!(report.manifest.payload.packages[name], package.digest);
        for file in package
            .files
            .keys()
            .map(String::as_str)
            .chain(std::iter::once("orr.package.json"))
        {
            let relative = format!(".orr/packages/objects/{}/{file}", package.digest);
            assert_eq!(
                fs::read(report.output.join("project").join(&relative)).unwrap(),
                before[&relative]
            );
        }
    }
    assert_eq!(report.project_files, 14);
    assert_eq!(snapshot(&fixture.project.root), before);
}

#[test]
fn source_project_path_with_spaces_and_separate_empty_working_directory_is_supported() {
    let fixture = Fixture::new();
    let project = fixture.root.join("authored source with spaces");
    fs::create_dir(&project).unwrap();
    copy_tree(&fixture.project.root, &project);
    let mut options = fixture.options("spaces output");
    options.project = project;
    let report = export(&options).unwrap();
    assert_eq!(report.project_files, 9);
}

#[test]
fn a_verified_default_feature_arena_fails_the_staged_compatibility_smoke() {
    let Some(runtime) = std::env::var_os("ORR_EXPORT_DEFAULT_RUNTIME") else {
        eprintln!("SKIP default-feature runtime compatibility negative: supply a separately built, known trusted Arena in ORR_EXPORT_DEFAULT_RUNTIME");
        return;
    };
    let fixture = Fixture::new();
    let mut options = fixture.options("incompatible runtime output");
    // The acceptance lane supplies only its own separately built Arena binary.
    options.runtime = PathBuf::from(runtime);
    options.runtime_sha256 = hash_file(&options.runtime);
    let before = snapshot(&fixture.project.root);
    let error = export(&options).unwrap_err();
    assert!(error.contains("project"), "{error}");
    assert!(!options.output.exists());
    assert_eq!(snapshot(&fixture.project.root), before);
}

fn permissions_snapshot(root: &Path) -> BTreeMap<String, u32> {
    file_paths(root)
        .into_iter()
        .map(|path| {
            let mode = fs::metadata(root.join(&path)).unwrap().permissions().mode();
            (path, mode)
        })
        .collect()
}

fn rejection_preserves_sources(fixture: &Fixture, options: &ExportOptions, diagnostic: &str) {
    let bytes = snapshot(&fixture.project.root);
    let permissions = permissions_snapshot(&fixture.project.root);
    let runtime_hash = hash_file(&fixture.runtime);
    let runtime_mode = fs::metadata(&fixture.runtime).unwrap().permissions().mode();
    let message = export(options).unwrap_err();
    assert!(
        message.contains(diagnostic),
        "expected {diagnostic:?}: {message}"
    );
    assert_eq!(snapshot(&fixture.project.root), bytes);
    assert_eq!(permissions_snapshot(&fixture.project.root), permissions);
    assert_eq!(hash_file(&fixture.runtime), runtime_hash);
    assert_eq!(
        fs::metadata(&fixture.runtime).unwrap().permissions().mode(),
        runtime_mode
    );
}

#[test]
fn runtime_metadata_bounds_links_and_special_files_fail_without_source_changes() {
    let fixture = Fixture::new();
    let options = fixture.options("nonexecutable output");
    fs::set_permissions(&fixture.runtime, fs::Permissions::from_mode(0o644)).unwrap();
    rejection_preserves_sources(&fixture, &options, "executable");
    assert!(!options.output.exists());
    fs::set_permissions(&fixture.runtime, fs::Permissions::from_mode(0o755)).unwrap();

    let sparse = fixture.root.join("oversized sparse runtime");
    fs::File::create_new(&sparse)
        .unwrap()
        .set_len(512 * 1024 * 1024 + 1)
        .unwrap();
    fs::set_permissions(&sparse, fs::Permissions::from_mode(0o755)).unwrap();
    let sparse_metadata = fs::metadata(&sparse).unwrap();
    assert!(
        sparse_metadata.blocks() * 512 < sparse_metadata.len(),
        "fixture must be sparse rather than allocate the whole bound"
    );
    let mut options = fixture.options("oversized output");
    options.runtime = sparse.clone();
    // Intentionally do not read/hash this sparse fixture: its metadata must be
    // rejected before the exporter reads a 512 MiB payload or hashes it.
    rejection_preserves_sources(&fixture, &options, "bytes");
    assert!(!options.output.exists());
    assert_eq!(fs::metadata(&sparse).unwrap().len(), sparse_metadata.len());
    assert_eq!(
        fs::metadata(&sparse).unwrap().permissions().mode(),
        sparse_metadata.permissions().mode()
    );

    let link = fixture.root.join("runtime symlink");
    std::os::unix::fs::symlink(&fixture.runtime, &link).unwrap();
    let mut options = fixture.options("symlink runtime output");
    options.runtime = link.clone();
    rejection_preserves_sources(&fixture, &options, "symlink");
    assert!(!options.output.exists());
    assert_eq!(fs::read_link(link).unwrap(), fixture.runtime);

    let fifo = fixture.root.join("runtime FIFO");
    rustix::fs::mkfifoat(
        rustix::fs::CWD,
        &fifo,
        rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
    )
    .unwrap();
    let fifo_mode = fs::symlink_metadata(&fifo).unwrap().permissions().mode();
    let mut options = fixture.options("FIFO runtime output");
    options.runtime = fifo.clone();
    rejection_preserves_sources(&fixture, &options, "regular");
    assert!(!options.output.exists());
    assert_eq!(
        fs::symlink_metadata(fifo).unwrap().permissions().mode(),
        fifo_mode
    );
}

#[test]
fn dangling_destination_symlink_is_never_replaced() {
    let fixture = Fixture::new();
    let options = fixture.options("dangling output");
    let missing = fixture.root.join("missing link target");
    std::os::unix::fs::symlink(&missing, &options.output).unwrap();
    let metadata = fs::symlink_metadata(&options.output).unwrap();
    rejection_preserves_sources(&fixture, &options, "exists");
    assert_eq!(fs::read_link(&options.output).unwrap(), missing);
    let after = fs::symlink_metadata(&options.output).unwrap();
    assert_eq!(
        (metadata.dev(), metadata.ino(), metadata.mode()),
        (after.dev(), after.ino(), after.mode())
    );
    assert!(!missing.exists());
}

#[test]
fn malformed_metadata_sidecar_and_tampered_asset_fail_without_repairing_sources() {
    let fixture = Fixture::new();
    let manifest = fixture.project.root.join("orr.project.json");
    let original = fs::read(&manifest).unwrap();
    for (name, content, diagnostic) in [
        ("malformed manifest output", "{broken", "project"),
        (
            "schema one output",
            r#"{"schema":1,"engine":"^0.0.1"}"#,
            "schema-2",
        ),
    ] {
        fs::write(&manifest, content).unwrap();
        let options = fixture.options(name);
        rejection_preserves_sources(&fixture, &options, diagnostic);
        assert!(!options.output.exists());
    }
    fs::write(&manifest, original).unwrap();

    let sidecar = fixture.project.root.join("arena.sprites.json");
    let original_sidecar = fs::read(&sidecar).unwrap();
    let mut document: serde_json::Value = serde_json::from_slice(&original_sidecar).unwrap();
    document["scene"] = "different.scene.yaml".into();
    fs::copy(
        fixture.project.root.join("arena.scene.yaml"),
        fixture.project.root.join("different.scene.yaml"),
    )
    .unwrap();
    fs::write(&sidecar, serde_json::to_vec_pretty(&document).unwrap()).unwrap();
    let options = fixture.options("mismatched sidecar output");
    rejection_preserves_sources(&fixture, &options, "scene");
    assert!(!options.output.exists());
    fs::write(&sidecar, original_sidecar).unwrap();

    let atlas = fixture
        .project
        .root
        .join(format!(".orr/packages/objects/{OBJECT}/lantern_keeper.png"));
    let mut bytes = fs::read(&atlas).unwrap();
    bytes.push(0);
    fs::write(atlas, bytes).unwrap();
    let options = fixture.options("tampered asset output");
    rejection_preserves_sources(&fixture, &options, "changed");
    assert!(!options.output.exists());
}

#[test]
fn unsupported_active_capability_fails_even_when_no_sprite_uses_the_package() {
    let fixture = Fixture::new();
    let source = fixture.root.join("unsupported package source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("orr.package.json"), r#"{"schema":1,"name":"unsupported-content","version":"1.0.0","engine":"^0.0.1","capabilities":["export-test-unsupported"],"dependencies":{},"files":["unused.txt"]}"#).unwrap();
    fs::write(source.join("unused.txt"), "Unused active content\n").unwrap();
    // Setup uses an explicit synthetic host inventory to install a consistent
    // lock. Export must use its own actual compiled inventory, never this one.
    let mut setup_runtime = orr_sample::project_runtime::compiled_runtime();
    setup_runtime
        .capabilities
        .insert("export-test-unsupported".into());
    let installer = orr_package::Project::open(&fixture.project.root, setup_runtime).unwrap();
    installer.install(&[source]).unwrap();
    let options = fixture.options("unsupported capability output");
    rejection_preserves_sources(&fixture, &options, "compiled capability");
    assert!(!options.output.exists());
}

fn assert_no_export_stages(root: &Path) {
    for entry in fs::read_dir(root).unwrap() {
        let entry = entry.unwrap();
        assert!(
            !entry
                .file_name()
                .to_string_lossy()
                .starts_with(".orr-export-"),
            "failed export leaked its private stage: {}",
            entry.path().display()
        );
    }
}

fn elf_interpreter_range(bytes: &[u8]) -> std::ops::Range<usize> {
    assert!(
        bytes.len() >= 64,
        "built Arena must have a complete ELF64 header"
    );
    assert_eq!(&bytes[..7], b"\x7fELF\x02\x01\x01");
    assert_eq!(u16::from_le_bytes(bytes[18..20].try_into().unwrap()), 62);
    assert_eq!(u16::from_le_bytes(bytes[52..54].try_into().unwrap()), 64);
    let table = usize::try_from(u64::from_le_bytes(bytes[32..40].try_into().unwrap())).unwrap();
    let entry_size = usize::from(u16::from_le_bytes(bytes[54..56].try_into().unwrap()));
    let entries = usize::from(u16::from_le_bytes(bytes[56..58].try_into().unwrap()));
    assert!(
        entry_size >= 56,
        "ELF64 program headers must contain the standard fields"
    );
    assert!(
        entries > 0 && entries < 0xffff,
        "test requires ordinary program-header numbering"
    );
    let table_end = table
        .checked_add(entry_size.checked_mul(entries).unwrap())
        .unwrap();
    assert!(table >= 64 && table_end <= bytes.len());
    let mut interpreter = None;
    for index in 0..entries {
        let offset = table
            .checked_add(index.checked_mul(entry_size).unwrap())
            .unwrap();
        let header = bytes
            .get(offset..offset.checked_add(entry_size).unwrap())
            .unwrap();
        if u32::from_le_bytes(header[..4].try_into().unwrap()) != 3 {
            continue;
        }
        assert!(interpreter.is_none(), "PT_INTERP must not be ambiguous");
        let start = usize::try_from(u64::from_le_bytes(header[8..16].try_into().unwrap())).unwrap();
        let size = usize::try_from(u64::from_le_bytes(header[32..40].try_into().unwrap())).unwrap();
        let end = start.checked_add(size).unwrap();
        let path = bytes.get(start..end).unwrap();
        assert!(start >= table_end && path.len() > 1);
        assert_eq!(path[0], b'/');
        assert_eq!(path.last(), Some(&0));
        interpreter = Some(start..end);
    }
    interpreter.expect("built Arena has no PT_INTERP; loader-failure acceptance requires a dynamically linked build")
}

#[test]
fn known_built_arena_with_unavailable_loader_fails_before_runtime_code_runs() {
    let fixture = Fixture::new();
    let original = fs::read(env!("CARGO_BIN_EXE_arena")).unwrap();
    assert_eq!(
        hash_file(Path::new(env!("CARGO_BIN_EXE_arena"))),
        fixture.runtime_sha256
    );
    let interpreter = elf_interpreter_range(&original);
    let missing_loader = Path::new("/.orr-no-loader");
    assert_eq!(
        fs::symlink_metadata(missing_loader).unwrap_err().kind(),
        std::io::ErrorKind::NotFound
    );
    let replacement = missing_loader.to_str().unwrap().as_bytes();
    assert!(
        replacement.len() < interpreter.len(),
        "missing loader path must fit in the existing PT_INTERP range"
    );
    let mut unavailable_loader = original.clone();
    unavailable_loader[interpreter.clone()].fill(0);
    unavailable_loader[interpreter.start..interpreter.start + replacement.len()]
        .copy_from_slice(replacement);
    assert_eq!(
        &unavailable_loader[..interpreter.start],
        &original[..interpreter.start]
    );
    assert_eq!(
        &unavailable_loader[interpreter.end..],
        &original[interpreter.end..]
    );
    assert_ne!(
        &unavailable_loader[interpreter.clone()],
        &original[interpreter]
    );
    let runtime = fixture.root.join("known Arena with missing loader");
    fs::write(&runtime, unavailable_loader).unwrap();
    fs::set_permissions(&runtime, fs::Permissions::from_mode(0o755)).unwrap();
    let runtime_hash = hash_file(&runtime);
    let runtime_mode = fs::metadata(&runtime).unwrap().permissions().mode();
    let mut options = fixture.options("missing loader output");
    options.runtime = runtime.clone();
    options.runtime_sha256 = runtime_hash.clone();
    // Only the known source-built Arena's interpreter string was changed. The
    // missing kernel loader causes exec to fail before any runtime code starts.
    rejection_preserves_sources(&fixture, &options, "smoke launch failed");
    assert!(!options.output.exists());
    assert_no_export_stages(&fixture.root);
    assert_eq!(hash_file(&runtime), runtime_hash);
    assert_eq!(
        fs::metadata(runtime).unwrap().permissions().mode(),
        runtime_mode
    );
}

struct RestorePermissions {
    path: PathBuf,
    original: fs::Permissions,
}
impl Drop for RestorePermissions {
    fn drop(&mut self) {
        let _ = fs::set_permissions(&self.path, self.original.clone());
    }
}

#[test]
fn unreadable_scene_fails_without_changing_source_permissions_or_publishing() {
    let fixture = Fixture::new();
    let before = snapshot(&fixture.project.root);
    let permissions = permissions_snapshot(&fixture.project.root);
    let runtime_hash = hash_file(&fixture.runtime);
    let runtime_mode = fs::metadata(&fixture.runtime).unwrap().permissions().mode();
    let scene = fixture.project.root.join("arena.scene.yaml");
    let restore = RestorePermissions {
        path: scene.clone(),
        original: fs::metadata(&scene).unwrap().permissions(),
    };
    fs::set_permissions(&scene, fs::Permissions::from_mode(0o0)).unwrap();
    match fs::File::open(&scene) {
        Ok(_) => {
            drop(restore);
            assert_eq!(snapshot(&fixture.project.root), before);
            assert_eq!(permissions_snapshot(&fixture.project.root), permissions);
            eprintln!("SKIP unreadable-input negative: this OS identity can read mode000 files (for example root/CAP_DAC_OVERRIDE); run as an ordinary unprivileged user to exercise permission denial");
            return;
        }
        Err(error) => assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied),
    }
    let options = fixture.options("unreadable input output");
    let result = export(&options);
    let still_unreadable_mode = fs::metadata(&scene).unwrap().permissions().mode() & 0o777;
    drop(restore);
    let error = result.unwrap_err();
    assert!(
        error.to_ascii_lowercase().contains("permission denied"),
        "{error}"
    );
    assert_eq!(
        still_unreadable_mode, 0,
        "exporter must not repair source permissions"
    );
    assert!(!options.output.exists());
    assert_no_export_stages(&fixture.root);
    assert_eq!(snapshot(&fixture.project.root), before);
    assert_eq!(permissions_snapshot(&fixture.project.root), permissions);
    assert_eq!(hash_file(&fixture.runtime), runtime_hash);
    assert_eq!(
        fs::metadata(&fixture.runtime).unwrap().permissions().mode(),
        runtime_mode
    );
}
