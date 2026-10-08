//! Acceptance starts with the built generator, never a copied project fixture.
//!
//! Required (Linux x86-64, bubblewrap, already-built trusted binaries):
//! ORR_NEW_ARENA_BIN=/absolute/orr_new_arena
//! ORR_NEW_ARENA_RUNTIME=/absolute/arena
//! ORR_NEW_ARENA_EXPORTER=/absolute/orr_export_arena
//! cargo test -p orr_editor --features sprites --test new_arena_workflow \
//!   generated_arena_cpu_workflow -- --ignored --exact --nocapture
//! Run generated_arena_gpu_workflow the same way for mandatory GPU evidence.
//! ORR_EXPORT_HIDE_ROOT may broaden isolation to the entire build workspace;
//! ORR_PROJECT_CAPTURE_DIR retains editor/runtime/relocated-export PNG evidence.
//! Neither test silently skips missing binaries, namespace isolation, or GPU.
#![cfg(all(feature = "sprites", target_os = "linux", target_arch = "x86_64"))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

#[path = "common/saved_project.rs"]
#[allow(unused_imports)]
mod ui;

use egui::accesskit::{Action, ActionData, ActionRequest, Role};
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use orr_bridge::{Bridge, PlaySession, PlayerSlot};
use orr_editor::{
    app::{LBL_PLAY, LBL_STOP},
    editor::input::Phase,
    EditorApp,
};
use orr_fp::FP;
use orr_reflect::Guid;
use orr_sample::{
    arena_game::{Arena, ArenaConfig, ArenaInput, Bullet, PlayerTag, Position, Score},
    arena_view::arena_fire_commands,
    project_runtime::{build_id, PreparedRuntime, PLAYERS, TICK_RATE},
    project_sprites::{Document, Source},
};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

const OBJECT: &str = "952838c26cb40890a4c9b1bb9a3c25c5e2430bca3b33b4064b9d79efd7432f86";
const TEMPLATE: &str = "arena-2d-v1";

struct Workflow {
    root: tempfile::TempDir,
    generator: PathBuf,
    runtime: PathBuf,
    exporter: PathBuf,
    original_bins: Vec<PathBuf>,
    hidden_workspace: PathBuf,
}

impl Workflow {
    fn new() -> Self {
        let root = ui::tempdir();
        fs::create_dir(root.path().join("tools")).unwrap();
        fs::create_dir(root.path().join("empty cwd")).unwrap();
        fs::create_dir(root.path().join("captures")).unwrap();
        let hidden_workspace = std::env::var_os("ORR_EXPORT_HIDE_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(ui::repository)
            .canonicalize()
            .unwrap();
        assert!(ui::repository().starts_with(&hidden_workspace));
        assert!(
            !root.path().starts_with(&hidden_workspace),
            "test output must be outside hidden workspace"
        );
        let mut original_bins = Vec::new();
        let mut copied = Vec::new();
        for (variable, name) in [
            ("ORR_NEW_ARENA_BIN", "orr_new_arena"),
            ("ORR_NEW_ARENA_RUNTIME", "arena"),
            ("ORR_NEW_ARENA_EXPORTER", "orr_export_arena"),
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
            .arg(self.root.path())
            .arg(self.root.path());
        for directory in &hidden {
            command.arg("--tmpfs").arg(directory);
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
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY");
        command
    }

    fn generate(&self, name: &str, seed: &str) -> PathBuf {
        let output = self.root.path().join(name);
        assert!(!output.exists());
        let mut command = self.isolated(&self.generator, None);
        command
            .arg("--output")
            .arg(&output)
            .args(["--template", TEMPLATE, "--seed", seed]);
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
        let mut command = self.isolated(&bundle.join("run-arena"), Some(source));
        add_runtime_args(&mut command, ticks, held, capture);
        success(command.output().unwrap())
    }
}

fn add_runtime_args(command: &mut Command, ticks: u32, held: &str, capture: Option<&Path>) {
    command.args(["--headless", "--ticks", &ticks.to_string(), "--hold", held]);
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

fn document(root: &Path) -> Document {
    serde_json::from_slice(&fs::read(root.join("arena.sprites.json")).unwrap()).unwrap()
}

fn actors(root: &Path) -> [String; 2] {
    let runtime = PreparedRuntime::open(root).unwrap();
    let entities = &runtime.project().scene().scene().entities;
    ["hero", "target"].map(|name| {
        entities
            .iter()
            .find(|(_, entity)| entity.name.as_deref() == Some(name))
            .unwrap_or_else(|| panic!("generated scene is missing named actor {name}"))
            .0
            .to_string()
    })
}

fn assert_starter(root: &Path) -> [String; 2] {
    let ids = actors(root);
    assert_eq!(ids[0].len(), 34);
    assert_eq!(ids[1].len(), 34);
    assert_eq!(
        &ids[0][..26],
        &ids[1][..26],
        "actors share one 96-bit seed/version prefix"
    );
    assert_eq!(&ids[0][26..], "00000001");
    assert_eq!(&ids[1][26..], "00000002");
    assert!(ids[0] < ids[1]);
    let prepared = PreparedRuntime::open(root).unwrap();
    let frame = prepared.initial_frame();
    for (slot, guid) in ids.iter().enumerate() {
        let entity = prepared
            .index()
            .entity(&Guid::parse(guid).unwrap())
            .unwrap();
        assert_eq!(frame.get::<PlayerTag>(entity).unwrap().slot, slot as u32);
        let position = frame.get::<Position>(entity).unwrap().pos;
        assert_eq!(position.x, FP::from_int(if slot == 0 { -60 } else { 60 }));
        assert_eq!(position.y, FP::ZERO);
    }
    assert_eq!(frame.dense::<PlayerTag>().0.len(), 2);
    assert!(frame.dense::<Bullet>().0.is_empty());
    assert_eq!(frame.singleton::<Score>().kills, [0; 8]);
    let sidecar = document(root);
    assert_eq!(sidecar.version, 2);
    assert_eq!(sidecar.scene, "arena.scene.yaml");
    assert_eq!(sidecar.project, ".");
    assert_eq!(sidecar.bindings.len(), 2);
    assert_eq!(sidecar.camera_follow.as_deref(), Some(ids[0].as_str()));
    for guid in &ids {
        let binding = &sidecar.bindings[guid];
        assert_eq!(
            binding.source,
            Source::Locomotion {
                idle: "idle".into(),
                walk: "walk".into()
            }
        );
        assert_eq!(binding.package, "sample-sprites");
        assert_eq!(binding.document, "sprites.json");
        assert_eq!(binding.units_per_pixel, 2.0);
    }
    let package = orr_editor::sprite_bindings::open_project(root).unwrap();
    let lock = package.verify().unwrap();
    assert_eq!(lock.direct.len(), 1);
    assert_eq!(lock.direct["sample-sprites"], "1.0.0");
    assert_eq!(lock.packages["sample-sprites"].digest, OBJECT);
    assert!(!package
        .read_asset("sample-sprites", "LICENSE.txt")
        .unwrap()
        .is_empty());
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("orr.project.json")).unwrap()).unwrap();
    assert_eq!(manifest["schema"], 2);
    assert!(
        manifest["entry"].get("ui").is_none(),
        "the bounded starter is sprite-only"
    );
    assert!(snapshot(root)
        .keys()
        .all(|path| !path.contains("settings") && !path.contains("cache")));
    ids
}

fn assert_reproducible(work: &Workflow) -> (PathBuf, [String; 2]) {
    let a = work.generate("authored generated project", "workflow-seed");
    let b = work.generate("same seed elsewhere", "workflow-seed");
    let c = work.generate("different seed", "other-workflow-seed");
    let ids = assert_starter(&a);
    assert_eq!(assert_starter(&b), ids);
    let other_ids = assert_starter(&c);
    assert!(
        ids.iter().all(|id| !other_ids.contains(id)),
        "different seeds must have disjoint entity GUIDs"
    );
    assert_eq!(
        snapshot(&a),
        snapshot(&b),
        "same seed is byte-reproducible across output paths"
    );
    assert_ne!(
        fs::read(a.join("arena.scene.yaml")).unwrap(),
        fs::read(c.join("arena.scene.yaml")).unwrap()
    );
    let installed = |root: &Path| {
        snapshot(root)
            .into_iter()
            .filter(|(path, _)| path.starts_with(".orr/") || path == "orr.packages.lock.json")
            .collect::<BTreeMap<_, _>>()
    };
    assert_eq!(
        installed(&a),
        installed(&c),
        "seed remapping never changes installed asset bytes or hashes"
    );
    assert_eq!(
        PreparedRuntime::open(&a)
            .unwrap()
            .initial_frame()
            .checksum(),
        PreparedRuntime::open(&c)
            .unwrap()
            .initial_frame()
            .checksum(),
        "common-prefix ordered ordinals preserve handle order and Arena initial checksum"
    );
    (a, ids)
}

fn edit_position(h: &mut Harness<'_, EditorApp>, hero: &str) {
    ui::click(h, "hero");
    h.get_by(|node| node.role() == Role::TextInput && node.value().as_deref() == Some("-60"))
        .focus();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(2);
    h.get_by(|node| node.role() == Role::TextInput && node.value().as_deref() == Some("-60"))
        .type_text("-48");
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    ui::settle(h);
    assert_eq!(ui::body_position(h.state(), hero), [-48.0, 0.0]);
    assert!(h.state().editor.is_dirty());
}

fn save_scene(h: &mut Harness<'_, EditorApp>) {
    h.get_by_label("File").click();
    h.run_steps(2);
    h.get_by(|node| {
        node.role() == Role::Button
            && node
                .label()
                .is_some_and(|label| label.starts_with("Save ") && !label.starts_with("Save As"))
    })
    .click();
    ui::settle(h);
    assert!(!h.state().editor.is_dirty());
}

fn edit_sprite_follow(h: &mut Harness<'_, EditorApp>, target: &str) {
    ui::open_inspector(h, "target");
    // The two unlabeled sprite draft text boxes are the last empty inspector
    // fields. Fill them with ordinary text input, not direct panel mutation.
    let empty = h
        .get_all_by_role(Role::TextInput)
        .filter(|node| node.value().as_deref() == Some(""))
        .collect::<Vec<_>>();
    assert!(
        empty.len() >= 2,
        "package and sprite document draft inputs are present"
    );
    empty[empty.len() - 2].focus();
    h.run_steps(2);
    h.event(egui::Event::Text("sample-sprites".into()));
    ui::settle(h);
    h.get_all_by_role(Role::TextInput)
        .rev()
        .find(|node| node.value().as_deref() == Some(""))
        .unwrap()
        .focus();
    h.run_steps(2);
    h.event(egui::Event::Text("sprites.json".into()));
    ui::settle(h);
    assert_eq!(h.state().sprites.package, "sample-sprites");
    assert_eq!(h.state().sprites.document, "sprites.json");
    ui::click(h, "Load sprite document");
    h.get_by_role_and_label(Role::ComboBox, "Sprite source")
        .click();
    h.run_steps(2);
    ui::click(h, "Idle / walk");
    for (label, clip) in [("Idle clip", "walk"), ("Walk clip", "idle")] {
        h.get_by_role_and_label(Role::ComboBox, label)
            .scroll_to_me();
        h.run_steps(2);
        h.get_by_role_and_label(Role::ComboBox, label).click();
        h.run_steps(2);
        ui::click(h, clip);
    }
    let slider = h.get_by_role_and_label(Role::Slider, "world units/pixel");
    let (target_node, target_tree) = slider.accesskit_node().locate();
    h.event(egui::Event::AccessKitActionRequest(ActionRequest {
        target_node,
        target_tree,
        action: Action::SetValue,
        data: Some(ActionData::NumericValue(2.0)),
    }));
    ui::settle(h);
    assert_eq!(h.state().sprites.scale, 2.0);
    ui::click(h, "Assign sprite to selection");
    ui::click(h, "Follow selected entity");
    ui::no_error(h);
    assert_eq!(
        ui::document(h).bindings[target].source,
        Source::Locomotion {
            idle: "walk".into(),
            walk: "idle".into()
        }
    );
    assert_eq!(ui::document(h).camera_follow.as_deref(), Some(target));
    assert!(h.state().sprites.bindings.as_ref().unwrap().dirty());
}

fn editor_workflow(root: &Path, ids: &[String; 2], gpu: bool, captures: &Path) -> u64 {
    let mut h = if gpu {
        ui::open_gpu(root)
    } else {
        ui::open_headless(root)
    };
    ui::settle(&mut h);
    ui::no_error(&h);
    assert_eq!(ui::document(&h), &document(root));
    assert_eq!(ui::region(h.state(), &ids[0]), Some(10));
    assert_eq!(ui::region(h.state(), &ids[1]), Some(10));
    let original = h.state().editor.checksum();
    let initial_sidecar = fs::read(root.join("arena.sprites.json")).unwrap();
    edit_position(&mut h, &ids[0]);
    assert_ne!(h.state().editor.checksum(), original);
    save_scene(&mut h);
    let scene = fs::read(root.join("arena.scene.yaml")).unwrap();
    assert_eq!(
        fs::read(root.join("arena.sprites.json")).unwrap(),
        initial_sidecar
    );
    let checksum = h.state().editor.checksum();
    edit_sprite_follow(&mut h, &ids[1]);
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history().entries.len(), 1);
    assert_eq!(
        fs::read(root.join("arena.sprites.json")).unwrap(),
        initial_sidecar,
        "binding edit is unsaved until its separate Save"
    );
    ui::click(&mut h, "Save bindings");
    let sidecar = fs::read(root.join("arena.sprites.json")).unwrap();
    assert_ne!(sidecar, initial_sidecar);
    assert_eq!(fs::read(root.join("arena.scene.yaml")).unwrap(), scene);
    let authored = ui::document(&h).clone();
    let camera = h.state().editor.camera;
    if gpu {
        editor_capture(&mut h, captures, "generated-editor-edited", ids);
    }
    ui::click(&mut h, LBL_PLAY);
    ui::wait_for(&mut h, "generated project's Play capability", |app| {
        app.editor.can_take_control()
    });
    ui::click(&mut h, "Take control");
    ui::wait_for(
        &mut h,
        "generated project's managed keyboard input",
        |app| app.editor.input_phase() == Phase::Active,
    );
    let start = ui::body_position(h.state(), &ids[0]);
    ui::key(&h, egui::Key::ArrowRight, true);
    ui::wait_for(
        &mut h,
        "generated actor movement and authored follow",
        |app| {
            ui::moving(app, &ids[0])
                && ui::body_position(app, &ids[0])[0] > start[0]
                && app.editor.camera.center == ui::body_position(app, &ids[1])
        },
    );
    assert!(!ui::moving(h.state(), &ids[1]));
    assert!(matches!(ui::region(h.state(), &ids[0]), Some(20 | 21)));
    assert!(matches!(ui::region(h.state(), &ids[1]), Some(20 | 21)));
    if gpu {
        editor_capture(&mut h, captures, "generated-editor-playing", ids);
    }
    ui::key(&h, egui::Key::ArrowRight, false);
    ui::wait_for(&mut h, "generated actor release", |app| {
        !ui::moving(app, &ids[0])
    });
    ui::click(&mut h, LBL_STOP);
    ui::wait_stopped(&mut h, camera);
    assert_eq!(h.state().editor.checksum(), checksum);
    assert_eq!(h.state().editor.history().entries.len(), 1);
    assert_eq!(ui::document(&h), &authored);
    let replay = h
        .state_mut()
        .editor
        .host_call(
            "verify.self",
            serde_json::json!({"inputs":{"kind":"last_play"},"checks":["recording_matches"]}),
        )
        .unwrap();
    assert_eq!(replay["passed"], true);
    assert_eq!(fs::read(root.join("arena.scene.yaml")).unwrap(), scene);
    assert_eq!(fs::read(root.join("arena.sprites.json")).unwrap(), sidecar);
    drop(h);
    let mut reopened = ui::open_headless(root);
    ui::settle(&mut reopened);
    assert_eq!(ui::document(&reopened), &authored);
    assert_eq!(ui::body_position(reopened.state(), &ids[0]), [-48.0, 0.0]);
    assert_eq!(reopened.state().editor.checksum(), checksum);
    checksum
}

fn replay_runtime(root: &Path, expected: u64) {
    let before = snapshot(root);
    let doc =
        orr_remote::sample::arena_doc(&fs::read_to_string(root.join("arena.scene.yaml")).unwrap())
            .unwrap();
    assert_eq!(doc.checksum(), expected);
    let prepared = PreparedRuntime::open(root).unwrap();
    assert_eq!(prepared.initial_frame().checksum(), expected);
    assert_eq!(build_id(), orr_remote::default_build_id("Arena"));
    let mut config = doc.play_config(PLAYERS, TICK_RATE);
    config.game_id = "Arena".into();
    config.build_id = build_id();
    config.start_paused = true;
    let mut editor = PlaySession::<Arena>::from_frame(config, doc.frame()).unwrap();
    editor.set_commands_from_input(|slot, input| arena_fire_commands(u32::from(slot.0), input));
    let mut runtime = prepared.bridge().unwrap();
    let mut presentation = prepared.into_parts().unwrap().1;
    assert_eq!(
        runtime.host().session().simulation().build_hash(),
        editor.simulation().build_hash()
    );
    let mut checksums = vec![expected];
    let mut fired = false;
    for tick in 1..=180_u64 {
        let (x, y) = match (tick / 30) % 4 {
            0 => (1, 0),
            1 => (0, 1),
            2 => (-1, 0),
            _ => (0, -1),
        };
        let input = ArenaInput::new(FP::from_int(x), FP::from_int(y), tick % 13 < 3);
        runtime.set_input(PlayerSlot(0), input).unwrap();
        runtime.step(1);
        editor.set_input(PlayerSlot(0), input);
        assert!(editor.step_now().is_some());
        let checksum = runtime.host().session().frame().checksum();
        assert_eq!(
            checksum,
            editor.frame().checksum(),
            "runtime/editor tick {tick}"
        );
        assert_eq!(runtime.host().session().checksum_at(tick), Some(checksum));
        assert_eq!(editor.checksum_at(tick), Some(checksum));
        fired |= !runtime
            .host()
            .session()
            .frame()
            .dense::<Bullet>()
            .0
            .is_empty();
        checksums.push(checksum);
        presentation.update(runtime.snapshot().as_ref()).unwrap();
        assert_eq!(
            runtime.host().session().frame().checksum(),
            checksum,
            "render preparation is view-only"
        );
    }
    assert!(fired, "normal runtime input derives actual fire commands");
    let mut replay = PlaySession::<Arena>::open_replay(
        &runtime.host().session().save_replay(),
        ArenaConfig {
            player_count: PLAYERS,
        },
        build_id(),
    )
    .unwrap();
    for (tick, checksum) in checksums.into_iter().enumerate() {
        replay.seek(tick as u64).unwrap();
        assert_eq!(replay.frame().checksum(), checksum, "recorded tick {tick}");
    }
    assert_eq!(
        PreparedRuntime::open(root)
            .unwrap()
            .session()
            .unwrap()
            .frame()
            .checksum(),
        expected
    );
    assert_eq!(snapshot(root), before);
}

fn checksum(stdout: &str) -> String {
    stdout
        .lines()
        .find_map(|line| line.strip_prefix("project tick: "))
        .unwrap()
        .split_once(" checksum: ")
        .unwrap()
        .1
        .to_owned()
}

fn export_and_relocate(work: &Workflow, root: &Path, initial: u64, gpu: bool) {
    // The existing exporter intentionally packages only its admitted closure.
    // README is authoring guidance, while the scene and installed package keep
    // the full license/provenance in the portable runtime closure.
    let mut active = snapshot(root);
    assert!(active.remove("README.md").is_some());
    assert_eq!(active.len(), 9);
    let scene_license = String::from_utf8_lossy(&active["arena.scene.yaml"]);
    let license_path = format!(".orr/packages/objects/{OBJECT}/LICENSE.txt");
    let license = String::from_utf8_lossy(&active[&license_path]);
    assert!(
        license
            .lines()
            .filter(|line| !line.is_empty())
            .all(|line| scene_license.contains(line)),
        "exported scene retains the full bundled-content license"
    );
    for path in [
        ".orr/packages/cache/unused",
        ".orr/packages/objects/inactive/sentinel",
        ".orr/editor/session.json",
        "settings.json",
        "settings.json.bak",
        ".config/orrery/settings.json",
        "arbitrary-sentinel",
    ] {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"not authored active content").unwrap();
    }
    let source = snapshot(root);
    let original_output = work.root.path().join("original export");
    let mut hash_command = Command::new("sha256sum");
    hash_command.arg(&work.runtime);
    let runtime_hash = success(hash_command.output().unwrap())
        .split_whitespace()
        .next()
        .unwrap()
        .to_owned();
    let mut exporter = work.isolated(&work.exporter, None);
    exporter
        .arg("--project")
        .arg(root)
        .arg("--runtime")
        .arg(&work.runtime)
        .arg("--runtime-sha256")
        .arg(runtime_hash)
        .arg("--output")
        .arg(&original_output)
        .arg("--trusted-runtime");
    success(exporter.output().unwrap());
    let bundle = work.root.path().join("relocated game with spaces");
    fs::rename(&original_output, &bundle).unwrap();
    assert!(!original_output.exists());
    assert_eq!(
        snapshot(&bundle.join("project")),
        active,
        "export includes licenses/active assets and excludes editor, cache, and player settings"
    );
    assert_eq!(snapshot(root), source);
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(bundle.join("orr.export.json")).unwrap()).unwrap();
    assert_eq!(
        manifest["payload"]["initial_checksum"],
        format!("0x{initial:016x}")
    );
    let ids = actors(root);
    let mut outputs = Vec::new();
    for (ticks, held) in [(0, "idle"), (2, "right"), (3, "right"), (3, "right,fire")] {
        let expected = work.run_runtime(root, ticks, held, None);
        let actual = work.run_bundle(&bundle, root, ticks, held, None);
        assert_eq!(
            actual, expected,
            "relocated launcher parity for ticks={ticks}, held={held}"
        );
        outputs.push(expected);
    }
    assert_eq!(checksum(&outputs[0]), format!("0x{initial:016x}"));
    assert_ne!(
        checksum(&outputs[2]),
        checksum(&outputs[3]),
        "fire changes actual simulation"
    );
    assert_eq!(
        work.run_bundle(&bundle, root, 0, "idle", None),
        outputs[0],
        "fresh relaunch restores edited initial state"
    );
    if gpu {
        let asset = orr_editor::sprite_bindings::load_asset(root, "sample-sprites", "sprites.json")
            .unwrap();
        for (ticks, held, name) in [(0, "idle", "initial"), (2, "right", "moved")] {
            let runtime_png = work
                .root
                .path()
                .join(format!("captures/generated-runtime-{name}.png"));
            let export_png = work
                .root
                .path()
                .join(format!("captures/generated-export-{name}.png"));
            let expected = work.run_runtime(root, ticks, held, Some(&runtime_png));
            let actual = work.run_bundle(&bundle, root, ticks, held, Some(&export_png));
            assert_eq!(actual, expected);
            let runtime_image = read_png(&runtime_png);
            let export_image = read_png(&export_png);
            assert_eq!(
                runtime_image, export_image,
                "relocation preserves composed runtime GPU pixels"
            );
            let camera = orr_render::Camera::new([60.0, 0.0], 240.0);
            for (guid, position, region) in [
                (
                    &ids[0],
                    [-48.0 + 6.0 * ticks as f32, 0.0],
                    if ticks == 0 { 10 } else { 20 },
                ),
                (&ids[1], [60.0, 0.0], 20),
            ] {
                assert!(atlas_texels(&runtime_image, &camera, position, &asset, region) > 20,
                    "actual standalone and exported GPU captures must paint installed opaque atlas pixels for {guid}");
            }
        }
    }
    assert_eq!(
        snapshot(root),
        source,
        "runtime/export never rewrite source including player settings"
    );
    assert_eq!(snapshot(&bundle.join("project")), active);
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

fn write_png(path: &Path, image: &Image) {
    let mut encoder = png::Encoder::new(fs::File::create(path).unwrap(), image.0, image.1);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&image.2)
        .unwrap();
}

fn atlas_texels(
    image: &Image,
    camera: &orr_render::Camera,
    position: [f32; 2],
    asset: &orr_editor::sprite_bindings::Asset,
    id: u32,
) -> usize {
    let region = asset.document.region(id).unwrap();
    let mut opaque = Vec::<[u8; 4]>::new();
    for y in region.y..region.y + region.height {
        for x in region.x..region.x + region.width {
            let at = ((y * asset.document.atlas().width + x) * 4) as usize;
            if asset.rgba[at + 3] == 255 {
                opaque.push(asset.rgba[at..at + 4].try_into().unwrap());
            }
        }
    }
    let center = camera.world_to_screen(position, (image.0, image.1));
    let radius = 16.0 * camera.pixels_per_unit(image.0, image.1) + 2.0;
    let mut matches = 0;
    for y in (center[1] - radius).max(0.0) as u32..(center[1] + radius).min(image.1 as f32) as u32 {
        for x in
            (center[0] - radius).max(0.0) as u32..(center[0] + radius).min(image.0 as f32) as u32
        {
            let at = ((y * image.0 + x) * 4) as usize;
            matches += usize::from(opaque.iter().any(|expected| {
                expected
                    .iter()
                    .zip(&image.2[at..at + 4])
                    .all(|(a, b)| a.abs_diff(*b) <= 3)
            }));
        }
    }
    matches
}

fn editor_capture(h: &mut Harness<'_, EditorApp>, captures: &Path, name: &str, ids: &[String; 2]) {
    assert!(h.state().has_gpu());
    let asset = orr_editor::sprite_bindings::load_asset(
        h.state().sprites.bindings.as_ref().unwrap().base(),
        "sample-sprites",
        "sprites.json",
    )
    .unwrap();
    let rendered = h
        .render()
        .expect("mandatory composed EditorApp GPU readback");
    let image = (rendered.width(), rendered.height(), rendered.into_raw());
    let rect = h.state().ui.viewport_rect.unwrap();
    for guid in ids {
        let region = asset
            .document
            .region(ui::region(h.state(), guid).unwrap())
            .unwrap();
        let body = ui::body_position(h.state(), guid);
        let scale = ui::document(h).bindings[guid].units_per_pixel;
        let mut matched = 0;
        for y in 0..region.height {
            for x in 0..region.width {
                let at =
                    (((region.y + y) * asset.document.atlas().width + region.x + x) * 4) as usize;
                if asset.rgba[at + 3] != 255 {
                    continue;
                }
                let point = h.state().editor.camera.world_to_screen(
                    [
                        body[0] + (x as f32 - 7.5) * scale,
                        body[1] + (7.5 - y as f32) * scale,
                    ],
                    h.state().ui.viewport_px,
                );
                let px = (rect.min.x + point[0]).floor() as u32;
                let py = (rect.min.y + point[1]).floor() as u32;
                assert!(px < image.0 && py < image.1);
                let offset = ((py * image.0 + px) * 4) as usize;
                assert!(
                    image.2[offset..offset + 3]
                        .iter()
                        .zip(&asset.rgba[at..at + 3])
                        .all(|(a, b)| a.abs_diff(*b) <= 3),
                    "{guid}: composed atlas texel ({x},{y})"
                );
                matched += 1;
            }
        }
        assert!(matched > 80, "every actor has opaque sprite evidence");
    }
    write_png(&captures.join(format!("{name}.png")), &image);
}

fn workflow(gpu: bool) {
    let work = Workflow::new();
    let (project, ids) = assert_reproducible(&work);
    let captures = work.root.path().join("captures");
    let checksum = editor_workflow(&project, &ids, gpu, &captures);
    replay_runtime(&project, checksum);
    export_and_relocate(&work, &project, checksum, gpu);
    if let Some(directory) = std::env::var_os("ORR_PROJECT_CAPTURE_DIR") {
        let destination = PathBuf::from(directory);
        fs::create_dir_all(&destination).unwrap();
        for entry in fs::read_dir(&captures).unwrap() {
            let entry = entry.unwrap();
            fs::copy(entry.path(), destination.join(entry.file_name())).unwrap();
        }
    }
    eprintln!(
        "generated Arena {} workflow passed: edited checksum 0x{checksum:016x}",
        if gpu { "GPU" } else { "CPU" }
    );
}

#[test]
#[ignore = "mandatory acceptance requires built generator/runtime/exporter and Linux bubblewrap"]
fn generated_arena_cpu_workflow() {
    workflow(false);
}

#[test]
#[ignore = "mandatory GPU acceptance requires built binaries, Linux bubblewrap and a working wgpu adapter"]
fn generated_arena_gpu_workflow() {
    workflow(true);
}
