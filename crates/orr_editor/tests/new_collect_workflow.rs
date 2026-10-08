//! Generated CollectDodge acceptance. No fixture copy and no silent skips.
//!
//! Build trusted binaries first (orr_sample): orr_new_arena with project-create,
//! collect-dodge; collect_dodge with collect-sprites,collect-progress; and
//! orr_export_collect with project-export,collect-sprites,collect-progress.
//! Set ORR_NEW_ARENA_BIN, ORR_COLLECT_RUNTIME, ORR_COLLECT_EXPORTER to absolute
//! paths. Linux x86-64 bubblewrap namespaces must work; GPU test needs wgpu.
//! cargo test -p orr_editor --features sprites,collect-dodge --test new_collect_workflow \
//!   generated_collect_cpu_workflow -- --ignored --exact --nocapture
//! Repeat with generated_collect_gpu_workflow and generated_collect_progress_workflow.
//! Progress additionally requires ORR_PROGRESS_TEST_BIN: exact orr_sample libtest
//! built with collect-sprites,collect-progress (runs actual App win/relaunch).
//! ORR_EXPORT_HIDE_ROOT optionally masks the entire build workspace.
//! ORR_COLLECT_CAPTURES retains PNG and checksum/stdout evidence.
//! All three gates are mandatory explicit acceptance; ignored is not a pass.
#![cfg(all(
    feature = "sprites",
    feature = "collect-dodge",
    target_os = "linux",
    target_arch = "x86_64"
))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
use egui::accesskit::Role;
use egui_kittest::{Harness, kittest::Queryable};
use orr_bridge::{Bridge, BridgeConfig, InProc, PlayHost, PlayerSlot};
use orr_editor::{
    Editor, EditorApp, HostSpec,
    app::{LBL_PLAY, LBL_STOP},
    editor::input::Phase,
};
use orr_fp::FP;
use orr_reflect::{Guid, Scene};
use orr_sample::{
    collect_game::{
        COLLECTIBLE, CollectActor, CollectInput, CollectRun, HAZARD, PLAYER, PLAYING, WON,
    },
    collect_project::{PreparedProject, ProgressSupport, SpriteSupport},
    project_sprites::Document,
};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{Duration, Instant},
};
const TEMPLATE: &str = "collect-dodge-2d-v1";
const GAME_ID: &str = "12345678-1234-4234-8234-123456789abc";
const PROFILE_FIXTURE_ROOT: &str = "/tmp";
fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}
fn tempdir() -> tempfile::TempDir {
    tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap()
}
struct Workflow {
    root: tempfile::TempDir,
    profile: tempfile::TempDir,
    generator: PathBuf,
    runtime: PathBuf,
    exporter: PathBuf,
    original_bins: Vec<PathBuf>,
    hidden_workspace: PathBuf,
}

impl Workflow {
    fn new() -> Self {
        use std::os::unix::fs::PermissionsExt;
        let root = tempdir();
        fs::create_dir(root.path().join("tools")).unwrap();
        fs::create_dir(root.path().join("empty cwd")).unwrap();
        fs::create_dir(root.path().join("captures")).unwrap();
        let hidden_workspace = std::env::var_os("ORR_EXPORT_HIDE_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(repository)
            .canonicalize()
            .unwrap();
        assert!(repository().starts_with(&hidden_workspace));
        assert!(
            !root.path().starts_with(&hidden_workspace),
            "test output must be outside hidden workspace"
        );
        let mut original_bins = Vec::new();
        let mut copied = Vec::new();
        for (variable, name) in [
            ("ORR_NEW_ARENA_BIN", "orr_new_arena"),
            ("ORR_COLLECT_RUNTIME", "collect_dodge"),
            ("ORR_COLLECT_EXPORTER", "orr_export_collect"),
        ] {
            let original = PathBuf::from(
                std::env::var_os(variable)
                    .unwrap_or_else(|| panic!("mandatory acceptance requires {variable}")),
            )
            .canonicalize()
            .unwrap();
            assert!(
                original.is_file(),
                "{variable} must identify an already-built binary"
            );
            let copy = root.path().join("tools").join(name);
            fs::copy(&original, &copy).unwrap();
            original_bins.push(original);
            copied.push(copy);
        }
        Self {
            generator: copied[0].clone(),
            runtime: copied[1].clone(),
            exporter: copied[2].clone(),
            root,
            profile: tempfile::Builder::new()
                .permissions(fs::Permissions::from_mode(0o700))
                .tempdir()
                .unwrap(),
            original_bins,
            hidden_workspace,
        }
    }

    /// Namespaces mask directories only in a subprocess. Never rename shared
    /// checkout/assets or mutate another worker's workspace to establish proof.
    fn isolated(&self, executable: &Path, hide_project: Option<&Path>) -> Command {
        let mut hidden = vec![self.hidden_workspace.clone()];
        for original in &self.original_bins {
            let directory = original.parent().unwrap().to_owned();
            if !hidden.iter().any(|root| directory.starts_with(root)) {
                hidden.retain(|root| !root.starts_with(&directory));
                hidden.push(directory);
            }
        }
        if let Some(project) = hide_project {
            hidden.push(project.to_owned());
            hidden.push(self.runtime.parent().unwrap().to_owned());
        }
        for directory in &hidden {
            assert!(
                !executable.starts_with(directory),
                "isolation must not hide the relocated executable"
            );
            assert!(
                !self.root.path().starts_with(directory),
                "isolation cannot mask the portable test root"
            );
        }
        let mut command = Command::new("bwrap");
        // A root bind is nodev in a nested namespace. Preserve only the
        // already-accessible null device needed by the exporter's Stdio::null.
        command
            .args([
                "--die-with-parent",
                "--ro-bind",
                "/",
                "/",
                "--dev-bind",
                "/dev/null",
                "/dev/null",
                "--bind",
            ])
            // Host-root ancestors can be unmapped in bwrap's user namespace.
            // Use a separate private parent before restoring Work; aliasing
            // Work itself would expose the source and tools hidden below.
            .arg(self.profile.path())
            .arg(PROFILE_FIXTURE_ROOT)
            .arg("--bind")
            .arg(self.root.path())
            .arg(self.root.path());
        for directory in &hidden {
            command.arg("--tmpfs").arg(directory);
        }
        if let Some(project) = hide_project {
            let bundle = executable.parent().unwrap();
            assert!(!bundle.starts_with(project));
            command.arg("--ro-bind").arg(bundle).arg(bundle);
        }
        // Assert every masked directory is actually empty before executing code.
        command.args(["--", "/bin/sh", "-c",
            "n=$1; shift; while [ \"$n\" -gt 0 ]; do test -z \"$(ls -A -- \"$1\")\" || exit 91; shift; n=$((n-1)); done; exec \"$@\"", "sh"])
            .arg(hidden.len().to_string());
        for directory in &hidden {
            command.arg(directory);
        }
        command
            .arg(executable)
            .current_dir(self.root.path().join("empty cwd"))
            .env(
                "XDG_DATA_HOME",
                Path::new(PROFILE_FIXTURE_ROOT).join("user data"),
            )
            .env("HOME", Path::new(PROFILE_FIXTURE_ROOT).join("user home"))
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY");
        command
    }

    fn generate(&self, name: &str, seed: &str) -> PathBuf {
        let output = self.root.path().join(name);
        assert!(!output.exists());
        let mut command = self.isolated(&self.generator, None);
        command.arg("--output").arg(&output).args([
            "--template",
            TEMPLATE,
            "--seed",
            seed,
            "--game-id",
            GAME_ID,
        ]);
        success(
            command
                .output()
                .expect("run source-hidden built generator with bubblewrap"),
        );
        assert!(output.join("orr.project.json").is_file());
        output
    }

    fn run_runtime(
        &self,
        project: &Path,
        ticks: u32,
        held: &str,
        capture: Option<&Path>,
    ) -> String {
        let mut command = self.isolated(&self.runtime, None);
        command.arg("--project").arg(project);
        add_runtime_args(&mut command, ticks, held, capture);
        success(command.output().unwrap())
    }

    fn run_bundle(
        &self,
        bundle: &Path,
        source: &Path,
        ticks: u32,
        held: &str,
        capture: Option<&Path>,
    ) -> String {
        let mut command = self.isolated(&bundle.join("run-collect-dodge"), Some(source));
        add_runtime_args(&mut command, ticks, held, capture);
        success(command.output().unwrap())
    }
}

fn add_runtime_args(command: &mut Command, ticks: u32, held: &str, capture: Option<&Path>) {
    command.args(["--headless", "--ticks", &ticks.to_string()]);
    if !held.is_empty() {
        command.args(["--hold", held]);
    }
    if let Some(capture) = capture {
        command.arg("--capture").arg(capture);
    }
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

fn snapshot(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn visit(root: &Path, current: &Path, files: &mut BTreeMap<String, Vec<u8>>) {
        for entry in fs::read_dir(current).unwrap() {
            let entry = entry.unwrap();
            let kind = entry.file_type().unwrap();
            assert!(
                !kind.is_symlink(),
                "generated/exported content must not link to source"
            );
            if kind.is_dir() {
                visit(root, &entry.path(), files);
            } else {
                assert!(kind.is_file());
                files.insert(
                    entry
                        .path()
                        .strip_prefix(root)
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .into(),
                    fs::read(entry.path()).unwrap(),
                );
            }
        }
    }
    let mut files = BTreeMap::new();
    visit(root, root, &mut files);
    files
}

fn open(root: &Path) -> PreparedProject {
    PreparedProject::open_with_presentation(
        root,
        ProgressSupport::MetadataOnly,
        SpriteSupport::Supported,
    )
    .unwrap()
}
fn document(root: &Path) -> Document {
    serde_json::from_slice(&fs::read(root.join("level.sprites.json")).unwrap()).unwrap()
}
fn actors(root: &Path) -> [String; 4] {
    let p = open(root);
    let scene = Scene::parse(p.scene().text(), &orr_sample::collect_project::types()).unwrap();
    [
        "player",
        "first_collectible",
        "second_collectible",
        "hazard",
    ]
    .map(|name| {
        scene
            .entities
            .iter()
            .find(|(_, entity)| entity.name.as_deref() == Some(name))
            .unwrap()
            .0
            .to_string()
    })
}
fn assert_starter(root: &Path) -> [String; 4] {
    let p = open(root);
    let ids = actors(root);
    let frame = p.scene().frame();
    assert_eq!(frame.dense::<CollectActor>().0.len(), 4);
    for (i, (kind, ordinal, position)) in [
        (PLAYER, 0, [0, 0]),
        (COLLECTIBLE, 0, [10, 0]),
        (COLLECTIBLE, 1, [20, 0]),
        (HAZARD, 0, [0, 12]),
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(ids[i].len(), 34);
        assert_eq!(&ids[i][..26], &ids[0][..26]);
        assert_eq!(&ids[i][26..], format!("{:08x}", i + 1));
        let actor = frame
            .get::<CollectActor>(
                p.scene()
                    .index()
                    .entity(&Guid::parse(&ids[i]).unwrap())
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(actor.kind, kind);
        assert_eq!(actor.ordinal, ordinal);
        assert_eq!(actor.position.x, FP::from_int(position[0]));
        assert_eq!(actor.position.y, FP::from_int(position[1]));
        assert_eq!(actor.active, 1);
    }
    let run = frame.singleton::<CollectRun>();
    assert_eq!(
        (run.phase, run.score, run.goal, run.time_limit_ticks),
        (PLAYING, 0, 2, 600)
    );
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("orr.project.json")).unwrap()).unwrap();
    assert_eq!(manifest["schema"], 3);
    assert_eq!(
        manifest["progress"],
        serde_json::json!({"schema":1,"game_id":GAME_ID,"profile":"collect-dodge-highscore-v1"})
    );
    assert_eq!(manifest["entry"]["scene"], "level.scene.yaml");
    assert_eq!(manifest["entry"]["sprites"], "level.sprites.json");
    let doc = document(root);
    assert_eq!(doc.version, 2);
    assert_eq!(doc.scene, "level.scene.yaml");
    assert_eq!(doc.project, ".");
    assert_eq!(doc.bindings.len(), 4);
    for id in &ids {
        assert!(doc.bindings.contains_key(id));
    }
    let package = orr_editor::sprite_bindings::open_project(root).unwrap();
    let lock = package.verify().unwrap();
    assert_eq!(lock.direct.len(), 1);
    assert_eq!(lock.direct["sample-sprites"], "1.0.0");
    assert_eq!(
        lock.packages["sample-sprites"].digest,
        "952838c26cb40890a4c9b1bb9a3c25c5e2430bca3b33b4064b9d79efd7432f86",
        "generator must install the reviewed MIT atlas bytes"
    );
    let license = package.read_asset("sample-sprites", "LICENSE.txt").unwrap();
    assert!(
        String::from_utf8(license)
            .unwrap()
            .contains("Permission is hereby granted")
    );
    assert_eq!(p.sprites().unwrap().assets.len(), 1);
    ids
}
fn assert_reproducible(work: &Workflow) -> (PathBuf, [String; 4]) {
    let a = work.generate("generated project", "collect-workflow-seed");
    let b = work.generate("same seed elsewhere", "collect-workflow-seed");
    let c = work.generate("different seed", "other-collect-workflow-seed");
    let ids = assert_starter(&a);
    assert_eq!(assert_starter(&b), ids);
    let other = assert_starter(&c);
    assert!(ids.iter().all(|id| !other.contains(id)));
    assert_eq!(
        snapshot(&a),
        snapshot(&b),
        "same inputs reproduce all bytes at different output paths"
    );
    let installed = |root: &Path| {
        snapshot(root)
            .into_iter()
            .filter(|(path, _)| path.starts_with(".orr/") || path == "orr.packages.lock.json")
            .collect::<BTreeMap<_, _>>()
    };
    assert_eq!(installed(&a), installed(&c));
    assert_eq!(
        open(&a).scene().frame().checksum(),
        open(&c).scene().frame().checksum()
    );
    (a, ids)
}
fn wait(h: &mut Harness<'_, EditorApp>, label: &str, ready: impl Fn(&EditorApp) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        h.run_steps(1);
        if ready(h.state()) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{label}: {:?}",
            h.state().editor.status()
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn settle(h: &mut Harness<'_, EditorApp>) {
    wait(h, "coherent scene and presentation", |app| {
        app.editor.yard_rows_coherent()
            && app
                .editor
                .snapshot()
                .is_some_and(|s| s.timeline().is_some() == app.editor.is_playing_mode())
    });
    h.run_steps(3);
}
fn open_editor(root: &Path, gpu: bool) -> Harness<'static, EditorApp> {
    let p = open(root);
    let mut editor = Editor::start(&HostSpec::PreparedCollect {
        scene: p.path().into(),
        text: p.scene().text().into(),
        listen: None,
        debug_hooks: false,
    })
    .unwrap();
    editor.sync();
    let builder = Harness::builder().with_size([1400.0, 1000.0]);
    if gpu {
        builder
            .with_render_options(egui_wgpu::RendererOptions {
                predictable_texture_filtering: false,
                ..egui_wgpu::RendererOptions::PREDICTABLE
            })
            .wgpu()
            .build_eframe(move |cc| {
                let mut app = EditorApp::new(editor, cc.wgpu_render_state.clone());
                orr_editor::project::install_collect_presentation(&mut app, p, &cc.egui_ctx);
                app
            })
    } else {
        builder.build_eframe(move |cc| {
            let mut app = EditorApp::new(editor, None);
            orr_editor::project::install_collect_presentation(&mut app, p, &cc.egui_ctx);
            app
        })
    }
}
fn key(h: &mut Harness<'_, EditorApp>, key: egui::Key, pressed: bool) {
    h.event(egui::Event::Key {
        key,
        physical_key: Some(key),
        pressed,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
}
fn editor_workflow(root: &Path, gpu: bool, captures: &Path) -> u64 {
    let before = document(root);
    let mut h = open_editor(root, gpu);
    settle(&mut h);
    assert_eq!(
        h.state().sprites.bindings.as_ref().unwrap().document(),
        &before
    );
    assert_eq!(h.state().editor.bodies().len(), 4);
    h.get_by_label("first_collectible").click();
    wait(&mut h, "selection", |app| app.editor.inspect().is_some());
    h.run_steps(3);
    h.get_by(|n| n.role() == Role::TextInput && n.value().as_deref() == Some("10"))
        .focus();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(2);
    h.get_by(|n| n.role() == Role::TextInput && n.value().as_deref() == Some("10"))
        .type_text("12");
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    wait(&mut h, "actual inspector edit", |app| {
        app.editor.bodies().iter().any(|b| b.pos == [12.0, 0.0])
    });
    assert!(h.state().editor.is_dirty());
    h.get_by_label("File").click();
    h.run_steps(2);
    h.get_by(|n| {
        n.role() == Role::Button
            && n.label()
                .is_some_and(|s| s.starts_with("Save ") && !s.starts_with("Save As"))
    })
    .click();
    wait(&mut h, "Save", |app| !app.editor.is_dirty());
    let edited = h.state().editor.checksum();
    assert_eq!(open(root).scene().frame().checksum(), edited);
    let saved = snapshot(root);
    if gpu {
        editor_capture(&mut h, captures, "generated-collect-editor-edited", 4);
    }
    h.get_by_label(LBL_PLAY).click();
    wait(&mut h, "Play", |app| app.editor.can_take_control());
    h.get_by_label("Take control").click();
    wait(&mut h, "input claim", |app| {
        app.editor.input_phase() == Phase::Active
    });
    key(&mut h, egui::Key::D, true);
    wait(&mut h, "WON", |app| {
        app.editor.snapshot().is_some_and(|s| {
            let r = s.predicted().singleton::<CollectRun>();
            r.phase == WON && r.score == 2
        })
    });
    key(&mut h, egui::Key::D, false);
    if gpu {
        editor_capture(&mut h, captures, "generated-collect-editor-won", 2);
    }
    key(&mut h, egui::Key::Space, true);
    key(&mut h, egui::Key::Space, false);
    wait(&mut h, "fresh restart edge", |app| {
        app.editor.snapshot().is_some_and(|s| {
            let r = s.predicted().singleton::<CollectRun>();
            r.phase == PLAYING && r.score == 0
        })
    });
    let initial = open(root);
    let restarted = h.state().editor.snapshot().unwrap();
    for (entity, actor) in initial
        .scene()
        .frame()
        .dense::<CollectActor>()
        .0
        .iter()
        .zip(initial.scene().frame().dense::<CollectActor>().1.iter())
    {
        let actual = restarted.predicted().get::<CollectActor>(*entity).unwrap();
        assert_eq!(
            actual.position, actor.position,
            "restart restores every authored position"
        );
        assert_eq!(actual.active, actor.active);
    }
    if gpu {
        editor_capture(&mut h, captures, "generated-collect-editor-restarted", 4);
    }
    h.get_by_label(LBL_STOP).click();
    wait(&mut h, "Stop", |app| {
        app.editor.mode() == orr_editor::Mode::Edit
            && app
                .editor
                .snapshot()
                .is_some_and(|s| s.timeline().is_none())
    });
    assert_eq!(h.state().editor.checksum(), edited);
    assert_eq!(h.state().editor.input_phase(), Phase::Off);
    assert_eq!(snapshot(root), saved);
    drop(h);
    let mut reopened = open_editor(root, false);
    settle(&mut reopened);
    assert_eq!(reopened.state().editor.checksum(), edited);
    assert_eq!(
        reopened
            .state()
            .sprites
            .bindings
            .as_ref()
            .unwrap()
            .document(),
        &before
    );
    assert!(
        reopened
            .state()
            .editor
            .bodies()
            .iter()
            .any(|b| b.pos == [12.0, 0.0])
    );
    edited
}
type Image = (u32, u32, Vec<u8>);
fn read_png(path: &Path) -> Image {
    let mut reader = png::Decoder::new(fs::File::open(path).unwrap())
        .read_info()
        .unwrap();
    let mut bytes = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut bytes).unwrap();
    assert_eq!(info.color_type, png::ColorType::Rgba);
    assert_eq!(info.bit_depth, png::BitDepth::Eight);
    bytes.truncate(info.buffer_size());
    (info.width, info.height, bytes)
}
/// Samples every opaque installed-atlas texel at its world center. This cannot
/// pass merely because the old primitive squares or background remain visible.
fn assert_texels(
    image: &Image,
    (offset, size): ([f32; 2], (u32, u32)),
    camera: &orr_render::Camera,
    position: [f32; 2],
    scale: f32,
    asset: &orr_editor::sprite_bindings::Asset,
    region_id: u32,
) {
    let region = asset.document.region(region_id).unwrap();
    let mut checked = 0;
    for y in 0..region.height {
        for x in 0..region.width {
            let i = (((region.y + y) * asset.document.atlas().width + region.x + x) * 4) as usize;
            if asset.rgba[i + 3] != 255 {
                continue;
            }
            let at = camera.world_to_screen(
                [
                    position[0] + (x as f32 + 0.5 - region.width as f32 / 2.0) * scale,
                    position[1] + (region.height as f32 / 2.0 - y as f32 - 0.5) * scale,
                ],
                size,
            );
            let px = (offset[0] + at[0]).floor();
            let py = (offset[1] + at[1]).floor();
            assert!(
                px >= 0.0 && py >= 0.0 && px < image.0 as f32 && py < image.1 as f32,
                "opaque texel must be visible"
            );
            let j = ((py as u32 * image.0 + px as u32) * 4) as usize;
            for c in 0..3 {
                assert!(
                    image.2[j + c].abs_diff(asset.rgba[i + c]) <= 4,
                    "region {region_id} texel {x},{y} channel {c}: {} vs {}",
                    image.2[j + c],
                    asset.rgba[i + c]
                );
            }
            checked += 1;
        }
    }
    assert!(
        checked > 40,
        "each visible actor must contribute opaque atlas texels"
    );
}
fn editor_capture(h: &mut Harness<'_, EditorApp>, captures: &Path, name: &str, expected: usize) {
    settle(h);
    assert!(h.state().has_gpu());
    assert_eq!(h.ctx.pixels_per_point(), 1.0);
    let rendered = h
        .render()
        .expect("mandatory composed EditorApp GPU readback");
    rendered.save(captures.join(format!("{name}.png"))).unwrap();
    let image = (rendered.width(), rendered.height(), rendered.into_raw());
    let app = h.state();
    let rect = app.ui.viewport_rect.unwrap();
    let bindings = app.sprites.bindings.as_ref().unwrap();
    let mut count = 0;
    for (guid, binding) in &bindings.document().bindings {
        let row = app
            .editor
            .rows()
            .iter()
            .find(|r| r.guid.as_ref().is_some_and(|g| g.as_str() == guid))
            .unwrap();
        let Some(body) = app.editor.bodies().iter().find(|b| b.entity == row.entity) else {
            continue;
        };
        let asset = orr_editor::sprite_bindings::load_asset(
            bindings.base(),
            &binding.package,
            &binding.document,
        )
        .unwrap();
        let region = app
            .sprites
            .sampled_region(&app.editor, guid)
            .expect("visible actor region");
        assert_texels(
            &image,
            ([rect.min.x, rect.min.y], app.ui.viewport_px),
            &app.editor.camera,
            body.pos,
            binding.units_per_pixel,
            &asset,
            region,
        );
        count += 1;
    }
    assert_eq!(
        count, expected,
        "collected sprites hide, then return on restart"
    );
}
fn runtime_evidence(root: &Path, ticks: u32, held: &str, image: Option<&Image>) -> u64 {
    let prepared = open(root);
    let mut presentation = prepared.presentation();
    let mut bridge = InProc::new(
        PlayHost::new(prepared.scene().session().unwrap(), PlayerSlot(0)),
        BridgeConfig::default(),
    );
    let input = CollectInput {
        x: if held.contains("right") {
            FP::ONE
        } else {
            FP::ZERO
        },
        y: if held.contains("up") {
            FP::ONE
        } else {
            FP::ZERO
        },
        buttons: if held.contains("restart") {
            orr_sample::collect_game::RESTART
        } else {
            0
        },
        ..Default::default()
    };
    bridge.set_input(PlayerSlot(0), input).unwrap();
    presentation.update(bridge.snapshot().as_ref()).unwrap();
    for _ in 0..ticks {
        bridge.step(1);
        presentation.update(bridge.snapshot().as_ref()).unwrap();
    }
    let checksum = bridge.snapshot().unwrap().predicted().checksum();
    if let Some(image) = image {
        let expected = if bridge
            .snapshot()
            .unwrap()
            .predicted()
            .singleton::<CollectRun>()
            .phase
            == WON
        {
            2
        } else {
            4
        };
        assert_eq!(presentation.sprites().len(), expected);
        for draw in presentation.sprites() {
            let asset = &presentation.assets()[&draw.asset];
            let region = asset.document.region(draw.instance.region).unwrap();
            assert_texels(
                image,
                ([0.0, 0.0], (image.0, image.1)),
                &presentation.camera,
                draw.instance.position,
                draw.instance.size[0] / region.width as f32,
                asset,
                draw.instance.region,
            );
        }
    }
    checksum
}
fn export_and_relocate(work: &Workflow, root: &Path, initial: u64, gpu: bool) -> PathBuf {
    let mut active = snapshot(root);
    assert!(active.remove("README.md").is_some());
    for path in [
        ".orr/packages/cache/unused",
        ".orr/packages/objects/inactive/sentinel",
        ".orr/editor/session.json",
        "settings.json",
        "arbitrary-sentinel",
    ] {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"not active authored content").unwrap();
    }
    let source = snapshot(root);
    let original = work.root.path().join("original export");
    let hash = success(
        Command::new("sha256sum")
            .arg(&work.runtime)
            .output()
            .unwrap(),
    )
    .split_whitespace()
    .next()
    .unwrap()
    .to_owned();
    success(
        work.isolated(&work.exporter, None)
            .arg("--project")
            .arg(root)
            .arg("--runtime")
            .arg(&work.runtime)
            .args(["--runtime-sha256", &hash, "--trusted-runtime", "--output"])
            .arg(&original)
            .output()
            .unwrap(),
    );
    let bundle = work.root.path().join("relocated game with spaces");
    fs::rename(&original, &bundle).unwrap();
    assert!(!original.exists());
    assert_eq!(
        snapshot(&bundle.join("project")),
        active,
        "only active authored closure exported; license and atlas remain installed"
    );
    let bundle_before = snapshot(&bundle);
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(bundle.join("orr.export.json")).unwrap()).unwrap();
    assert_eq!(
        manifest["payload"]["initial_checksum"],
        format!("0x{initial:016x}")
    );
    for (name, ticks, held, status) in [
        ("initial", 0, "", "PLAYING | score 0/2"),
        ("partial", 8, "right", "PLAYING | score 1/2"),
        ("won", 16, "right", "WON | score 2/2"),
        ("hazard", 8, "up", "LOST: hazard"),
        ("timeout", 600, "", "LOST: time"),
        ("held-restart", 17, "right,restart", "WON | score 2/2"),
    ] {
        let runtime = work.run_runtime(root, ticks, held, None);
        let exported = work.run_bundle(&bundle, root, ticks, held, None);
        assert_eq!(
            runtime, exported,
            "source-hidden read-only relocated runtime parity: {name}"
        );
        assert!(runtime.contains(status), "{name}: {runtime}");
        let checksum = runtime_evidence(root, ticks, held, None);
        assert!(runtime.contains(&format!(
            "collect tick: {ticks} checksum: 0x{checksum:016x}"
        )));
        fs::write(
            work.root
                .path()
                .join("captures")
                .join(format!("generated-collect-{name}.txt")),
            runtime,
        )
        .unwrap();
    }
    assert_eq!(
        work.run_bundle(&bundle, root, 0, "", None),
        work.run_runtime(root, 0, "", None),
        "fresh relaunch restores edited authoring state"
    );
    if gpu {
        for (name, ticks, held) in [("initial", 0, ""), ("idle", 36, ""), ("won", 16, "right")] {
            let a = work
                .root
                .path()
                .join("captures")
                .join(format!("generated-collect-runtime-{name}.png"));
            let b = work
                .root
                .path()
                .join("captures")
                .join(format!("generated-collect-export-{name}.png"));
            let stdout = work.run_runtime(root, ticks, held, Some(&a));
            assert!(stdout.contains("software: true"));
            assert_eq!(
                stdout,
                work.run_bundle(&bundle, root, ticks, held, Some(&b))
            );
            let image = read_png(&a);
            assert_eq!(image, read_png(&b), "relocated composed GPU pixels: {name}");
            runtime_evidence(root, ticks, held, Some(&image));
        }
    }
    assert_eq!(snapshot(root), source);
    assert_eq!(snapshot(&bundle), bundle_before);
    assert!(
        !work.profile.path().join("user data").exists(),
        "headless/export must never create user progress"
    );
    bundle
}
fn retain(work: &Workflow) {
    if let Some(destination) = std::env::var_os("ORR_COLLECT_CAPTURES") {
        let destination = PathBuf::from(destination);
        fs::create_dir_all(&destination).unwrap();
        for entry in fs::read_dir(work.root.path().join("captures")).unwrap() {
            let entry = entry.unwrap();
            fs::copy(entry.path(), destination.join(entry.file_name())).unwrap();
        }
    }
}
fn workflow(gpu: bool) {
    let work = Workflow::new();
    let (project, _) = assert_reproducible(&work);
    let checksum = editor_workflow(&project, gpu, &work.root.path().join("captures"));
    export_and_relocate(&work, &project, checksum, gpu);
    retain(&work);
    eprintln!(
        "generated CollectDodge {} workflow passed, edited checksum 0x{checksum:016x}",
        if gpu { "GPU" } else { "CPU" }
    );
}
#[test]
#[ignore = "mandatory: built generator/runtime/exporter and permitted Linux bubblewrap"]
fn generated_collect_cpu_workflow() {
    workflow(false);
}
#[test]
#[ignore = "mandatory: built tools, permitted Linux bubblewrap and software GPU"]
fn generated_collect_gpu_workflow() {
    workflow(true);
}
#[test]
#[ignore = "mandatory: built progress/sprite runtime, exporter, exact sample libtest and Linux bubblewrap"]
fn generated_collect_progress_workflow() {
    let mut work = Workflow::new();
    let project = work.generate("generated progress project", "collect-progress-seed");
    assert_starter(&project);
    let checksum = open(&project).scene().frame().checksum();
    let bundle = export_and_relocate(&work, &project, checksum, false);
    let source = snapshot(&project);
    let before = snapshot(&bundle);
    let original = PathBuf::from(
        std::env::var_os("ORR_PROGRESS_TEST_BIN")
            .expect("exact sample libtest built with collect-progress,collect-sprites"),
    )
    .canonicalize()
    .unwrap();
    assert!(original.is_file());
    work.original_bins.push(original.clone());
    let harness_dir = work.root.path().join("progress harness");
    fs::create_dir(&harness_dir).unwrap();
    let harness = harness_dir.join("sample-tests");
    fs::copy(&original, &harness).unwrap();
    // isolated() makes the executable's parent read-only and masks source/tools.
    // The separately relocated project bundle must also be read-only here.
    for mode in ["win", "relaunch"] {
        let command = work.isolated(&harness, Some(&project));
        // Insert the additional bind before the existing namespace delimiter.
        let args = command
            .get_args()
            .map(|s| s.to_os_string())
            .collect::<Vec<_>>();
        let split = args.iter().position(|s| s == "--").unwrap();
        let mut isolated = Command::new(command.get_program());
        isolated
            .args(&args[..split])
            .arg("--ro-bind")
            .arg(&bundle)
            .arg(&bundle)
            .args(&args[split..]);
        isolated.current_dir(command.get_current_dir().unwrap());
        for (key, value) in command.get_envs() {
            if let Some(value) = value {
                isolated.env(key, value);
            } else {
                isolated.env_remove(key);
            }
        }
        let output = success(
            isolated
                .args([
                    "--ignored",
                    "--exact",
                    "collect_progress::tests::isolated_window_progress_child",
                    "--nocapture",
                ])
                .env("ORR_PROGRESS_TEST_PROJECT", bundle.join("project"))
                .env("ORR_PROGRESS_TEST_MODE", mode)
                .output()
                .unwrap(),
        );
        assert!(
            output.contains("1 passed"),
            "actual App helper must execute: {output}"
        );
        fs::write(
            work.root
                .path()
                .join("captures")
                .join(format!("generated-collect-progress-{mode}.txt")),
            output,
        )
        .unwrap();
    }
    let data = work.profile.path().join("user data");
    assert!(data.join("orrery/games").is_dir());
    assert!(!snapshot(&data).is_empty());
    assert_eq!(snapshot(&project), source);
    assert_eq!(snapshot(&bundle), before);
    retain(&work);
}
