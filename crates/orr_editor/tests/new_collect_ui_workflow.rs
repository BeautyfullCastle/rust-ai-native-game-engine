//! Generated authored-UI CollectDodge end-to-end acceptance. No fixture copy or silent skips.
//!
//! Build trusted binaries first: orr_sample/orr_new_arena with project-create,
//! collect-ui; collect_dodge with collect-ui,collect-sprites,collect-progress;
//! orr_export_collect with project-export,collect-ui,collect-sprites,collect-progress.
//! Set absolute ORR_NEW_ARENA_BIN, ORR_COLLECT_RUNTIME, ORR_COLLECT_EXPORTER.
//! Progress requires ORR_PROGRESS_TEST_BIN: exact orr_sample libtest built with
//! collect-ui,collect-sprites,collect-progress; its real App hook clicks authored
//! Restart before persistence, then a fresh process verifies highscore loading.
//! cargo test -p orr_editor --features collect-ui --test new_collect_ui_workflow \
//!   generated_collect_ui_cpu_workflow -- --ignored --exact --nocapture
//! Repeat for generated_collect_ui_gpu_workflow and generated_collect_ui_progress_workflow.
//! All three gates are mandatory acceptance; ignored is not a pass. Linux x86-64
//! bubblewrap namespaces must work; GPU gate needs software wgpu. Optional
//! ORR_EXPORT_HIDE_ROOT masks the entire workspace; ORR_COLLECT_CAPTURES keeps evidence.
#![cfg(all(
    feature = "sprites",
    feature = "collect-ui",
    target_os = "linux",
    target_arch = "x86_64"
))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
use egui::accesskit::Role;
use egui_kittest::{kittest::Queryable, Harness};
use orr_bridge::{Bridge, BridgeConfig, InProc, PlayHost, PlayerSlot};
use orr_editor::{
    app::{LBL_PLAY, LBL_STOP},
    editor::input::Phase,
    Editor, EditorApp, HostSpec,
};
use orr_fp::FP;
use orr_reflect::{Guid, Scene};
use orr_sample::{
    collect_game::{
        CollectActor, CollectInput, CollectRun, COLLECTIBLE, HAZARD, PLAYER, PLAYING, WON,
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
const TEMPLATE: &str = "collect-dodge-ui-2d-v1";
const GAME_ID: &str = "12345678-1234-4234-8234-123456789abc";
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
    generator: PathBuf,
    runtime: PathBuf,
    exporter: PathBuf,
    original_bins: Vec<PathBuf>,
    hidden_workspace: PathBuf,
}

impl Workflow {
    fn new() -> Self {
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
            .env("XDG_DATA_HOME", self.root.path().join("user data"))
            .env("HOME", self.root.path().join("user home"))
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
    PreparedProject::open_with_ui(
        root,
        ProgressSupport::MetadataOnly,
        SpriteSupport::Supported,
        true,
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
    assert_eq!(lock.direct.len(), 2);
    assert_eq!(lock.packages.len(), 2);
    assert_eq!(lock.direct["korean-game-ui"], "1.0.0");
    for (name, bytes) in [
        (
            "OrreryKoreanUI.otf",
            include_bytes!("../../../assets/game_ui_font/OrreryKoreanUI.otf").as_slice(),
        ),
        (
            "font-manifest.json",
            include_bytes!("../../../assets/game_ui_font/font-manifest.json").as_slice(),
        ),
        (
            "corpus.txt",
            include_bytes!("../../../assets/game_ui_font/corpus.txt").as_slice(),
        ),
        (
            "OFL.txt",
            include_bytes!("../../../assets/game_ui_font/OFL.txt").as_slice(),
        ),
        (
            "COPYRIGHT.txt",
            include_bytes!("../../../assets/game_ui_font/COPYRIGHT.txt").as_slice(),
        ),
    ] {
        assert_eq!(
            package.read_asset("korean-game-ui", name).unwrap(),
            bytes,
            "generated font package must preserve reviewed asset {name}"
        );
    }
    assert_eq!(
        manifest["entry"]["ui"],
        serde_json::json!({
            "profile":"collect-authored-v1", "document":"level.ui.json",
            "font":{"package":"korean-game-ui", "asset":"OrreryKoreanUI.otf"}
        })
    );
    let ui = p.ui().expect("generated profile must admit authored UI");
    assert_eq!(ui.path, root.join("level.ui.json"));
    assert_eq!(
        ui.document,
        orr_sample::authored_ui::Document::default_collect()
    );
    assert_eq!(
        ui.font,
        package
            .read_asset("korean-game-ui", "OrreryKoreanUI.otf")
            .unwrap()
    );
    assert!(
        PreparedProject::open_with_presentation(
            root,
            ProgressSupport::MetadataOnly,
            SpriteSupport::Supported
        )
        .is_err(),
        "a caller without explicit UI support must reject the generated profile"
    );
    assert_eq!(lock.direct["sample-sprites"], "1.0.0");
    assert_eq!(
        lock.packages["sample-sprites"].digest,
        "952838c26cb40890a4c9b1bb9a3c25c5e2430bca3b33b4064b9d79efd7432f86",
        "generator must install the reviewed MIT atlas bytes"
    );
    let license = package.read_asset("sample-sprites", "LICENSE.txt").unwrap();
    assert!(String::from_utf8(license)
        .unwrap()
        .contains("Permission is hereby granted"));
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
/// Drive the real runtime UI from the generated (and GUI-saved) document.
/// This covers presentation dispatch, separately from EditorApp's toolbar and
/// from the actual App Restart/persistence hook in the progress gate.
fn runtime_ui_navigation(root: &Path) {
    use orr_sample::{
        authored_ui::{Action, Kind, Screen},
        collect_ui::{CollectUi, Hud},
    };
    let project_before = snapshot(root);
    let prepared = open(root);
    let presentation = prepared.ui().unwrap();
    let document_before = presentation.document.clone();
    let mut ui = CollectUi::new(document_before.clone(), presentation.font.clone()).unwrap();
    let mut bridge = InProc::new(
        PlayHost::new(prepared.scene().session().unwrap(), PlayerSlot(0)),
        BridgeConfig::default(),
    );
    // Keep the actual input/simulation snapshot fixed throughout presentation
    // navigation. Dispatch alone must not inject a simulation tick or mutation.
    let input = CollectInput {
        x: FP::ONE,
        ..Default::default()
    };
    bridge.set_input(PlayerSlot(0), input).unwrap();
    let simulation_before = bridge.snapshot().unwrap().predicted().checksum();
    let input_frame = |events| egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(900.0, 900.0),
        )),
        events,
        ..Default::default()
    };
    let draw = |ui: &mut CollectUi, events| {
        let (output, action) = ui.show(
            input_frame(events),
            Hud {
                score: 0,
                phase: PLAYING,
                best: "0".into(),
            },
        );
        assert!(
            !output.shapes.is_empty(),
            "actual authored widgets must render"
        );
        output.drop_without_applying_deltas();
        action
    };
    assert_eq!(ui.screen(), Screen::Title);
    assert!(ui.blocks_controls());
    for (action, next_screen) in [
        (Action::Play, Screen::Playing),
        (Action::Menu, Screen::Menu),
        (Action::Continue, Screen::Playing),
        (Action::Menu, Screen::Menu),
        (Action::Restart, Screen::Playing),
    ] {
        assert_eq!(draw(&mut ui, vec![]), None);
        let node = ui.document().nodes.iter().find(|node| {
            node.screen == ui.screen()
                && matches!(node.kind, Kind::Button { action: selected, .. } if selected == action)
        }).expect("generated screen contains the requested authored button");
        assert!(
            node.parent.is_none(),
            "starter navigation uses viewport-root controls"
        );
        let point = egui::pos2(
            900.0 * f32::from(node.anchor[0]) / 1000.0
                + f32::from(node.offset[0])
                + f32::from(node.size[0]) / 2.0,
            900.0 * f32::from(node.anchor[1]) / 1000.0
                + f32::from(node.offset[1])
                + f32::from(node.size[1]) / 2.0,
        );
        let button = |pressed| egui::Event::PointerButton {
            pos: point,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        assert!(ui.pending_pointer_over_ui(&input_frame(vec![egui::Event::PointerMoved(point)])));
        assert_eq!(
            draw(
                &mut ui,
                vec![egui::Event::PointerMoved(point), button(true)]
            ),
            None
        );
        let dispatched = draw(&mut ui, vec![button(false)]);
        assert_eq!(dispatched, Some(action), "real authored widget dispatch");
        assert_eq!(
            draw(&mut ui, vec![button(false)]),
            None,
            "a release without a new press cannot dispatch twice"
        );
        ui.apply(dispatched.unwrap());
        assert_eq!(ui.screen(), next_screen);
        assert_eq!(ui.blocks_controls(), next_screen != Screen::Playing);
        assert_eq!(
            bridge.snapshot().unwrap().predicted().checksum(),
            simulation_before,
            "presentation navigation must not mutate the input snapshot's simulation state"
        );
        assert_eq!(ui.document(), &document_before);
    }
    assert_eq!(
        snapshot(root),
        project_before,
        "navigation is read-only authoring"
    );
}
/// Exercise the editor's actual authoring widgets; no direct panel mutation/save.
fn edit_ui(h: &mut Harness<'_, EditorApp>, root: &Path) -> orr_sample::authored_ui::Document {
    use orr_sample::authored_ui::{Document as UiDocument, Kind};
    let before = snapshot(root);
    let original = h.state().collect_ui.as_ref().unwrap().document().clone();
    h.get_by_label("Collect UI document").click();
    h.run_steps(3);
    h.get_by_role_and_label(Role::ComboBox, "UI node").click();
    h.run_steps(3);
    h.get_by_label("score").click();
    h.run_steps(3);
    let edit = |h: &mut Harness<'_, EditorApp>| {
        h.get_by(|n| n.role() == Role::TextInput && n.value().as_deref() == Some("Score: "))
            .focus();
        h.run_steps(2);
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.run_steps(2);
        h.get_by(|n| n.role() == Role::TextInput && n.value().as_deref() == Some("Score: "))
            .type_text("게임 메뉴 ");
        h.run_steps(3);
    };
    edit(h);
    let edited = h.state().collect_ui.as_ref().unwrap().document().clone();
    assert_ne!(edited, original);
    assert!(
        matches!(&edited.nodes.iter().find(|n| n.id == "score").unwrap().kind,
        Kind::Label { text, .. } if text == "게임 메뉴 ")
    );
    assert_eq!(snapshot(root), before, "UI edits stay unsaved until Save");
    h.get_by_label("Undo UI edit").click();
    h.run_steps(3);
    assert_eq!(h.state().collect_ui.as_ref().unwrap().document(), &original);
    edit(h);
    assert_eq!(h.state().collect_ui.as_ref().unwrap().document(), &edited);
    h.get_by_label("Save UI document").click();
    h.run_steps(3);
    assert_eq!(
        UiDocument::parse(&fs::read(root.join("level.ui.json")).unwrap()).unwrap(),
        edited
    );
    let mut expected = before;
    expected.insert(
        "level.ui.json".into(),
        fs::read(root.join("level.ui.json")).unwrap(),
    );
    assert_eq!(
        snapshot(root),
        expected,
        "UI Save changes only its admitted document"
    );
    assert_eq!(open(root).ui().unwrap().document, edited);
    h.get_by_label("Collect UI document").click();
    h.run_steps(3);
    edited
}
fn editor_workflow(root: &Path, gpu: bool, captures: &Path) -> u64 {
    let before = document(root);
    let mut h = open_editor(root, gpu);
    settle(&mut h);
    assert_eq!(
        h.state().sprites.bindings.as_ref().unwrap().document(),
        &before
    );
    let edited_ui = edit_ui(&mut h, root);
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
        editor_capture(&mut h, captures, "generated-collect-ui-editor-edited", 4);
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
        editor_capture(&mut h, captures, "generated-collect-ui-editor-won", 2);
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
        editor_capture(&mut h, captures, "generated-collect-ui-editor-restarted", 4);
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
        reopened.state().collect_ui.as_ref().unwrap().document(),
        &edited_ui
    );
    let runtime_ui = orr_sample::collect_ui::CollectUi::new(
        open(root).ui().unwrap().document.clone(),
        open(root).ui().unwrap().font.clone(),
    )
    .unwrap();
    assert_eq!(runtime_ui.document(), &edited_ui);
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
    assert!(reopened
        .state()
        .editor
        .bodies()
        .iter()
        .any(|b| b.pos == [12.0, 0.0]));
    runtime_ui_navigation(root);
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
    (offset, extent, size): ([f32; 2], [f32; 2], (u32, u32)),
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
            // The editor maps rounded physical viewport pixels back into its
            // possibly fractional egui rectangle. Mirror that final transform;
            // installed UI fonts can change the logical panel dimensions.
            let px = (offset[0] + at[0] * extent[0] / size.0 as f32).floor();
            let py = (offset[1] + at[1] * extent[1] / size.1 as f32).floor();
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
    let capture = captures.join(format!("{name}.png"));
    rendered.save(&capture).unwrap();
    // Preserve the composed failure image before any pixel assertion panics.
    if let Some(destination) = std::env::var_os("ORR_COLLECT_CAPTURES") {
        let destination = PathBuf::from(destination);
        fs::create_dir_all(&destination).unwrap();
        fs::copy(&capture, destination.join(format!("{name}.png"))).unwrap();
    }
    let image = (rendered.width(), rendered.height(), rendered.into_raw());
    let app = h.state();
    let rect = app.ui.viewport_rect.unwrap();
    eprintln!(
        "{name}: viewport {rect:?}, physical {:?}, camera {:?}",
        app.ui.viewport_px, app.editor.camera
    );
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
            (
                [rect.min.x, rect.min.y],
                [rect.width(), rect.height()],
                app.ui.viewport_px,
            ),
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
    let snapshot = bridge.snapshot().unwrap();
    let frame = snapshot.predicted();
    let expected = frame
        .dense::<CollectActor>()
        .1
        .iter()
        .filter(|actor| actor.active != 0)
        .count();
    assert_eq!(
        expected,
        4 - frame.singleton::<CollectRun>().score as usize,
        "this generated level has two collectibles; each awarded point hides one actor"
    );
    assert_eq!(
        presentation.sprites().len(),
        expected,
        "authoritative visible presentation draw count at tick {ticks}, held {held}"
    );
    if let Some(image) = image {
        let draws = if bridge
            .snapshot()
            .unwrap()
            .predicted()
            .singleton::<CollectRun>()
            .phase
            == orr_sample::collect_game::LOST_HAZARD
        {
            // Global draw order follows the bindings' ordered GUID map. The
            // generated hazard is last, so its opaque texels remain visible
            // above the colliding player; verify that identity rather than
            // accidentally treating an arbitrary final draw as the hazard.
            let (guid, binding) = presentation
                .document()
                .unwrap()
                .bindings
                .last_key_value()
                .unwrap();
            assert_eq!(
                guid,
                &actors(root)[3],
                "hazard must be the last authored GUID"
            );
            let entity = prepared
                .scene()
                .index()
                .entity(&Guid::parse(guid).unwrap())
                .unwrap();
            let snapshot = bridge.snapshot().unwrap();
            let actor = snapshot.predicted().get::<CollectActor>(entity).unwrap();
            assert_eq!(actor.kind, HAZARD);
            assert_eq!(actor.active, 1);
            let last = presentation.sprites().last().unwrap();
            assert_eq!(
                last.asset,
                (binding.package.clone(), binding.document.clone())
            );
            assert_eq!(
                last.instance.position,
                [
                    orr_view::fp_to_f32(actor.position.x),
                    orr_view::fp_to_f32(actor.position.y)
                ]
            );
            let orr_sample::project_sprites::Source::Region(region) = &binding.source else {
                panic!("generated hazard must use its fixed reviewed atlas region");
            };
            assert_eq!(last.instance.region, *region);
            std::slice::from_ref(last)
        } else {
            presentation.sprites()
        };
        for draw in draws {
            let asset = &presentation.assets()[&draw.asset];
            let region = asset.document.region(draw.instance.region).unwrap();
            assert_texels(
                image,
                (
                    [0.0, 0.0],
                    [image.0 as f32, image.1 as f32],
                    (image.0, image.1),
                ),
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
/// Rendering parity alone could pass if both runtimes ignored UI. Re-render
/// the generated default score text and prove the GUI-authored Korean edit
/// changes pixels in that node's own screen rectangle, with simulation fixed.
fn assert_authored_pixels(work: &Workflow, root: &Path) {
    let path = root.join("level.ui.json");
    let edited_bytes = fs::read(&path).unwrap();
    let edited = orr_sample::authored_ui::Document::parse(&edited_bytes).unwrap();
    let default = orr_sample::authored_ui::Document::default_collect();
    let score = edited.nodes.iter().find(|n| n.id == "score").unwrap();
    assert_ne!(edited, default);
    let capture = work
        .root
        .path()
        .join("captures/generated-collect-ui-default-score.png");
    fs::write(&path, default.to_bytes().unwrap()).unwrap();
    let original_stdout = work.run_runtime(root, 0, "", Some(&capture));
    fs::write(&path, edited_bytes).unwrap();
    let original = read_png(&capture);
    let authored = read_png(
        &work
            .root
            .path()
            .join("captures/generated-collect-ui-runtime-initial.png"),
    );
    assert_eq!((original.0, original.1), (authored.0, authored.1));
    let edited_stdout = fs::read_to_string(
        work.root
            .path()
            .join("captures/generated-collect-ui-initial.txt"),
    )
    .unwrap();
    let checksum = |s: &str| {
        s.lines()
            .find(|line| line.contains("collect tick:"))
            .unwrap()
            .to_owned()
    };
    assert_eq!(
        checksum(&original_stdout),
        checksum(&edited_stdout),
        "UI text cannot change simulation state"
    );
    let x = (i64::from(authored.0) * i64::from(score.anchor[0]) / 1000 + i64::from(score.offset[0]))
        as u32;
    let y = (i64::from(authored.1) * i64::from(score.anchor[1]) / 1000 + i64::from(score.offset[1]))
        as u32;
    let mut changed = 0;
    for py in y..y + u32::from(score.size[1]) {
        for px in x..x + u32::from(score.size[0]) {
            assert!(px < authored.0 && py < authored.1);
            let i = ((py * authored.0 + px) * 4) as usize;
            if authored.2[i..i + 4] != original.2[i..i + 4] {
                changed += 1;
            }
        }
    }
    assert!(
        changed > 100,
        "GUI-authored Korean score text must affect its target pixels: {changed}"
    );
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
                .join(format!("generated-collect-ui-{name}.txt")),
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
        for (name, ticks, held) in [
            ("initial", 0, ""),
            ("idle", 36, ""),
            ("won", 16, "right"),
            ("hazard", 8, "up"),
            ("timeout", 600, ""),
        ] {
            let a = work
                .root
                .path()
                .join("captures")
                .join(format!("generated-collect-ui-runtime-{name}.png"));
            let b = work
                .root
                .path()
                .join("captures")
                .join(format!("generated-collect-ui-export-{name}.png"));
            eprintln!("runtime/export GPU capture {name}: ticks={ticks}, held={held}");
            let stdout = work.run_runtime(root, ticks, held, Some(&a));
            retain(work);
            let exported = work.run_bundle(&bundle, root, ticks, held, Some(&b));
            // Persist both source and relocated captures before any assertion.
            retain(work);
            assert!(stdout.contains("software: true"));
            assert_eq!(stdout, exported);
            assert_eq!(
                fs::read(&a).unwrap(),
                fs::read(&b).unwrap(),
                "exact source-hidden relocated PNG bytes: {name}"
            );
            let image = read_png(&a);
            assert_eq!(image, read_png(&b), "relocated composed GPU pixels: {name}");
            // All unobscured actors retain exact atlas proof, including timeout.
            // Hazard collision legitimately occludes the player; runtime_evidence
            // instead proves every opaque texel of the verified frontmost hazard.
            let checksum = runtime_evidence(root, ticks, held, Some(&image));
            assert!(
                stdout.contains(&format!(
                    "collect tick: {ticks} checksum: 0x{checksum:016x}"
                )),
                "authoritative captured runtime checksum: {name}"
            );
        }
        assert_authored_pixels(work, root);
    }
    assert_eq!(snapshot(root), source);
    assert_eq!(snapshot(&bundle), bundle_before);
    assert!(
        !work.root.path().join("user data").exists(),
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
        "generated authored-UI CollectDodge {} workflow passed, edited checksum 0x{checksum:016x}",
        if gpu { "GPU" } else { "CPU" }
    );
}
#[test]
#[ignore = "mandatory: built generator/runtime/exporter and permitted Linux bubblewrap"]
fn generated_collect_ui_cpu_workflow() {
    workflow(false);
}
#[test]
#[ignore = "mandatory: built tools, permitted Linux bubblewrap and software GPU"]
fn generated_collect_ui_gpu_workflow() {
    workflow(true);
}
#[test]
#[ignore = "mandatory: built progress/sprite runtime, exporter, exact sample libtest and Linux bubblewrap"]
fn generated_collect_ui_progress_workflow() {
    let mut work = Workflow::new();
    let project = work.generate("generated progress project", "collect-progress-seed");
    assert_starter(&project);
    let checksum = editor_workflow(&project, false, &work.root.path().join("captures"));
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
                .join(format!("generated-collect-ui-progress-{mode}.txt")),
            output,
        )
        .unwrap();
    }
    let data = work.root.path().join("user data");
    assert!(data.join("orrery/games").is_dir());
    assert!(!snapshot(&data).is_empty());
    assert_eq!(snapshot(&project), source);
    assert_eq!(snapshot(&bundle), before);
    retain(&work);
}
