//! Windows editor windows: `File > New Collect project…` and the Arena export tab.
//! The export test runs the supplied runtime, so it is an explicit acceptance test.
#![cfg(all(
    windows,
    target_arch = "x86_64",
    feature = "arena-project-export",
    feature = "project-create",
    feature = "collect-ui"
))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

use egui::{accesskit::Role, Key};
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use orr_editor::{game::EditorGame, Editor, EditorApp};
use std::{
    fs,
    path::Path,
    time::{Duration, Instant},
};

const GAME_ID: &str = "12345678-1234-4234-8234-123456789abc";

fn software_harness(editor: Editor) -> Harness<'static, EditorApp> {
    Harness::builder()
        .with_size([1500.0, 1000.0])
        .with_max_steps(120)
        .renderer(egui_kittest::LazyRenderer::Uninitialized {
            textures_delta: Default::default(),
            builder: None,
        })
        .build_eframe(move |_| EditorApp::new(editor, None))
}

fn wait(
    h: &mut Harness<'_, EditorApp>,
    reason: &str,
    ready: impl Fn(&Harness<'_, EditorApp>) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        h.run_steps(1);
        if ready(h) {
            return;
        }
        if Instant::now() >= deadline {
            let labels: Vec<_> = ["Job ", "cannot open", "Current project retained", "Export", "Save", "Finish", "Explicitly", "Expected", "Enter", "Resolve", "Inspect"]
                .iter()
                .flat_map(|needle| h.query_all_by_label_contains(needle))
                .map(|node| format!("{:?} {:?}", node.accesskit_node().label(), node.accesskit_node().value()))
                .collect();
            panic!("{reason}: {labels:?}");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn field(h: &mut Harness<'_, EditorApp>, label: &str, value: &str) {
    h.get_by_role_and_label(Role::TextInput, label).focus();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, Key::A);
    h.run_steps(2);
    h.get_by_role_and_label(Role::TextInput, label)
        .type_text(value);
    h.run_steps(3);
    assert_eq!(
        h.get_by_role_and_label(Role::TextInput, label)
            .accesskit_node()
            .value()
            .as_deref(),
        Some(value)
    );
}

fn physics_editor(base: &Path) -> Editor {
    let scene = base.join("initial-physics.scene.yaml");
    fs::copy(orr_editor::editor::default_scene_path(), &scene).unwrap();
    let mut editor = Editor::open(&scene).unwrap();
    editor.sync();
    assert_eq!(editor.game(), EditorGame::PhysGame);
    editor
}

fn open_new_collect_dialog(h: &mut Harness<'_, EditorApp>) {
    h.get_by_label("File").click();
    h.run_steps(3);
    h.get_by_role_and_label(Role::Button, "New Collect project…")
        .click();
    h.run_steps(3);
}

#[test]
fn new_collect_project_menu_creates_and_opens_a_project_on_windows() {
    let temp = tempfile::tempdir().unwrap();
    // Project creation takes plain disk paths, not canonical \\?\ paths.
    let base = temp.path().to_owned();
    let mut h = software_harness(physics_editor(&base));
    h.run_steps(3);
    open_new_collect_dialog(&mut h);
    let project = base.join("collect-created");
    field(&mut h, "Project directory", project.to_str().unwrap());
    field(&mut h, "Seed", "windows-menu-seed");
    field(&mut h, "Game UUIDv4", GAME_ID);
    h.get_by_label("Create and open").click();
    wait(&mut h, "Collect project was not opened", |h| {
        h.state().editor.game().is_collect() || h.query_by_label_contains("cannot open").is_some()
    });
    if !h.state().editor.game().is_collect() {
        let text = h.get_all_by_label_contains("cannot open").next().map(|n| format!("{:?}", n.accesskit_node().label()));
        panic!("not opened: {text:?}");
    }
    h.run_steps(3);
    assert_eq!(
        h.state().editor.path(),
        Some(fs::canonicalize(project.join("level.scene.yaml")).unwrap())
    );
    assert!(project.join("orr.project.json").is_file());
    assert!(!h.state().editor.is_dirty());
}

#[test]
fn dot_segments_are_rejected_with_either_separator() {
    let temp = tempfile::tempdir().unwrap();
    // Project creation takes plain disk paths, not canonical \\?\ paths.
    let base = temp.path().to_owned();
    let mut h = software_harness(physics_editor(&base));
    h.run_steps(3);
    open_new_collect_dialog(&mut h);
    field(&mut h, "Seed", "seed");
    field(&mut h, "Game UUIDv4", GAME_ID);
    for path in [
        format!("{}\\..\\x", base.display()),
        format!("{}/./x", base.display()),
    ] {
        field(&mut h, "Project directory", &path);
        assert!(h
            .get_by_label("Create and open")
            .accesskit_node()
            .is_disabled());
    }
}

#[test]
#[ignore = "requires ORR_WINDOWS_EXPORT_RUNTIME: a trusted arena.exe built with --features project"]
fn arena_export_tab_exports_a_saved_arena_on_windows() {
    let runtime = std::path::PathBuf::from(
        std::env::var_os("ORR_WINDOWS_EXPORT_RUNTIME").expect("supply a built arena.exe"),
    );
    let runtime = fs::canonicalize(runtime).unwrap();
    let hash = orr_sample::windows_export::measure_runtime(&runtime)
        .unwrap()
        .sha256;
    let temp = tempfile::tempdir().unwrap();
    // Project creation takes plain disk paths, not canonical \\?\ paths.
    let base = temp.path().to_owned();
    let project = base.join("arena");
    orr_sample::project_create::create(&orr_sample::project_create::CreateOptions {
        output: project.clone(),
        template: orr_sample::project_create::TEMPLATE.into(),
        seed: "windows-arena-export".into(),
    })
    .unwrap();
    let prepared = orr_editor::project::PreparedProject::open(&project).unwrap();
    let mut editor = Editor::start(&prepared.host_spec()).unwrap();
    editor.sync();
    assert_eq!(editor.game(), EditorGame::Arena);
    let mut h = Harness::builder()
        .with_size([1500.0, 1000.0])
        .renderer(egui_kittest::LazyRenderer::Uninitialized {
            textures_delta: Default::default(),
            builder: None,
        })
        .build_eframe(move |cc| prepared.into_app(editor, None, &cc.egui_ctx).unwrap());
    h.run_steps(3);
    h.get_by_label("Arena export").click();
    h.run_steps(3);
    let output = base.join("exported arena");
    field(&mut h, "Project path", project.to_str().unwrap());
    field(&mut h, "New output path", output.to_str().unwrap());
    field(&mut h, "Runtime path", runtime.to_str().unwrap());
    field(&mut h, "Expected lowercase SHA256", &hash);
    let trust = "I trust this supplied runtime and allow smoke verification to execute it";
    h.get_by_label(trust).scroll_to_me();
    h.run_steps(3);
    h.get_by_label(trust).click();
    h.run_steps(2);
    h.get_by_label("Export saved Arena").scroll_to_me();
    h.run_steps(3);
    h.get_by_label("Export saved Arena").click();
    h.run_steps(3);
    wait(&mut h, "export did not finish", |h| {
        h.query_by_label("Arena export completed and joined")
            .is_some()
    });
    assert!(output.join("orr.export.json").is_file());
    assert!(output.join("bin").join("arena.exe").is_file());
    assert!(output.join("project").join("orr.project.json").is_file());
}
