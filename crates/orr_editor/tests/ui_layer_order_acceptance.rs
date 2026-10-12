//! Actual EditorApp lifecycle and saved CollectUi layer-order acceptance.
//!
//! Requires Linux and `collect-ui,project-create`. The fixture is generated in-process,
//! admitted normally, then installed through Panel's validated atomic-save API.
//! Every layer-order/history operation under test uses the actual editor widgets.
//! No production API, fixture binary, GPU, export, or existing test is replaced.
#![cfg(all(feature = "collect-ui", feature = "project-create", target_os = "linux"))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

use egui::accesskit::Role;
use egui_kittest::{kittest::Queryable, Harness};
use orr_bridge::PlayerSlot;
use orr_editor::{
    app::{LBL_PLAY, LBL_STOP},
    collect_ui_panel::Panel,
    Editor, EditorApp, HostSpec, Mode,
};
use orr_fp::FP;
use orr_sample::{
    authored_ui::{Action, Document, Kind, Node, Screen},
    collect_game::{CollectInput, PLAYING, WON},
    collect_project::{PreparedProject, ProgressSupport, SpriteSupport},
    collect_ui::{CollectUi, Hud},
    project_create::{create_collect, CreateOptions},
};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const FRONT: &str = "Bring subtree to front";
const BACK: &str = "Send subtree to back";
const UNDO: &str = "Undo UI edit";
const REDO: &str = "Redo UI edit";
const SAVE: &str = "Save UI document";
const UI_FILE: &str = "level.ui.json";

fn node(id: &str, parent: Option<&str>, kind: Kind, offset: [i16; 2], size: [u16; 2]) -> Node {
    Node {
        id: id.into(),
        parent: parent.map(str::to_owned),
        kind,
        screen: Screen::Title,
        anchor: [0, 0],
        offset,
        size,
    }
}

fn button(text: &str, action: Action) -> Kind {
    Kind::Button { text: text.into(), action }
}

fn clipping_document() -> Document {
    Document {
        schema: 1,
        nodes: vec![
            node("lower", None, button("Play", Action::Play), [20, 20], [180, 100]),
            node("cover", None, Kind::Container, [40, 20], [80, 80]),
            // The child extends to x=180 but is clipped at the parent's x=120.
            node("child", Some("cover"), button("Restart", Action::Restart), [40, 30], [100, 30]),
            node("passive", None, Kind::Label { text: "Overlay".into(), binding: None }, [20, 20], [180, 100]),
        ],
    }
}

fn keyboard_document() -> Document {
    let mut document = Document {
        schema: 1,
        nodes: vec![
            node("lower", None, button("Play", Action::Play), [20, 20], [100, 40]),
            node("other", None, button("Quit", Action::Quit), [140, 20], [100, 40]),
            // The lower button is focusable and pointer-exposed at x=30,
            // but its center x=70 belongs to this later filled container.
            node("cover", None, Kind::Container, [60, 20], [60, 40]),
            node("passive", None, Kind::Label { text: "Overlay".into(), binding: None }, [20, 20], [100, 40]),
        ],
    };
    for (id, screen, text, action) in [
        ("playing", Screen::Playing, "Menu", Action::Menu),
        ("menu", Screen::Menu, "Continue", Action::Continue),
        ("terminal", Screen::Terminal, "Restart", Action::Restart),
    ] {
        let mut n = node(id, None, button(text, action), [20, 20], [100, 40]);
        n.screen = screen;
        document.nodes.push(n);
    }
    document
}

/// An explicit expected ID sequence, not the implementation's reorder algorithm.
fn in_order(original: &Document, ids: &[&str]) -> Document {
    assert_eq!(ids.len(), original.nodes.len());
    let document = Document {
        schema: original.schema,
        nodes: ids.iter().map(|id| {
            original.nodes.iter().find(|node| node.id == *id).unwrap().clone()
        }).collect(),
    };
    document.validate().unwrap();
    document
}

fn open(root: &Path) -> PreparedProject {
    PreparedProject::open_with_ui(root, ProgressSupport::MetadataOnly, SpriteSupport::Supported, true)
        .unwrap()
}

fn project_files(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, directory: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            let kind = entry.file_type().unwrap();
            assert!(!kind.is_symlink(), "fixture must not link to an outside source");
            if kind.is_dir() {
                visit(root, &entry.path(), files);
            } else {
                assert!(kind.is_file());
                files.insert(entry.path().strip_prefix(root).unwrap().to_owned(), fs::read(entry.path()).unwrap());
            }
        }
    }
    let mut files = BTreeMap::new();
    visit(root, root, &mut files);
    files
}

fn wait(h: &mut Harness<'_, EditorApp>, label: &str, ready: impl Fn(&EditorApp) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        h.run_steps(1);
        if ready(h.state()) {
            return;
        }
        assert!(Instant::now() < deadline, "{label}: {:?}", h.state().editor.status());
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn open_editor(root: &Path) -> Harness<'static, EditorApp> {
    let prepared = open(root);
    let mut editor = Editor::start(&HostSpec::PreparedCollect {
        scene: prepared.path().into(),
        text: prepared.scene().text().into(),
        listen: None,
        debug_hooks: false,
    }).unwrap();
    editor.sync();
    let mut h = Harness::builder().with_size([1800.0, 1600.0]).build_eframe(move |cc| {
        let mut app = EditorApp::new(editor, None);
        orr_editor::project::install_collect_presentation(&mut app, prepared, &cc.egui_ctx);
        app
    });
    h.run_steps(3);
    wait(&mut h, "admitted editor in Edit", |app| {
        app.editor.mode() == Mode::Edit
            && app.editor.yard_rows_coherent()
            && app.editor.snapshot().is_some_and(|snapshot| snapshot.timeline().is_none())
    });
    click(&mut h, "Collect UI document");
    h
}

struct Fixture {
    editor: Harness<'static, EditorApp>,
    // Drop the host/panel before removing their admitted directory.
    _directory: tempfile::TempDir,
    root: PathBuf,
}

impl Fixture {
    fn new(document: Document) -> Self {
        let directory = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
        let root = directory.path().join("generated layer order project");
        create_collect(&CreateOptions {
            output: root.clone(),
            template: "collect-dodge-ui-2d-v1".into(),
            seed: "ui-layer-order-acceptance".into(),
        }, "12345678-1234-4234-8234-123456789abc").unwrap();
        // Fixture setup only: preserve the actual generator's manifest, package
        // admission, font and atomic save boundary. Reopen to clear setup history.
        let before = project_files(&root);
        let mut prepared = open(&root);
        let mut panel = Panel::new(prepared.take_ui().unwrap());
        panel.apply_document(document.clone()).unwrap();
        panel.save().unwrap();
        drop(panel);
        let mut expected = before;
        expected.insert(PathBuf::from(UI_FILE), document.to_bytes().unwrap());
        assert_eq!(project_files(&root), expected);
        assert_eq!(open(&root).ui().unwrap().document, document);
        let editor = open_editor(&root);
        Self { editor, _directory: directory, root }
    }

    fn document(&self) -> &Document {
        self.editor.state().collect_ui.as_ref().unwrap().document()
    }

    fn select(&mut self, id: &str) {
        self.editor.get_by_role_and_label(Role::ComboBox, "UI node").click();
        self.editor.run_steps(3);
        click(&mut self.editor, id);
        self.editor.get_by_label(&format!("Stable id: {id}"));
    }

    fn assert_document(&self, expected: &Document) {
        assert_eq!(self.document(), expected, "full node fields and exact draw order");
        self.editor.get_by_label("Stable id: cover");
    }

    fn save_reopen(&mut self, expected: &Document) {
        self.assert_document(expected);
        let mut files = project_files(&self.root);
        click(&mut self.editor, SAVE);
        files.insert(PathBuf::from(UI_FILE), expected.to_bytes().unwrap());
        assert_eq!(project_files(&self.root), files, "Save changes only the admitted UI document");
        let prepared = open(&self.root);
        assert_eq!(prepared.ui().unwrap().document, *expected);
        let runtime = runtime(&self.root);
        assert_eq!(runtime.document(), expected, "production admission reloads the saved order");
    }
}

fn click(h: &mut Harness<'_, EditorApp>, label: &str) {
    h.get_by_label(label).click();
    h.run_steps(3);
}

fn pointer(point: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos: point,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

/// Use separate raw pointer frames at each observed control's rectangle and
/// independently assert the actual accessibility-disabled state.
fn attempt_disabled_click(h: &mut Harness<'_, EditorApp>, label: &str) {
    let control = h.get_by_label(label);
    assert!(control.accesskit_node().is_disabled(), "{label} is disabled by real Play mode");
    let point = control.rect().center();
    assert!(h.ctx.viewport_rect().contains(point), "control is in the viewport: {label}");
    h.event(egui::Event::PointerMoved(point));
    h.run_steps(1);
    h.event(pointer(point, true));
    h.run_steps(1);
    h.event(pointer(point, false));
    h.run_steps(3);
}

#[test]
fn editor_play_layer_order_readonly() {
    let original = clipping_document();
    let back = in_order(&original, &["cover", "child", "lower", "passive"]);
    let front = in_order(&original, &["lower", "passive", "cover", "child"]);
    let mut f = Fixture::new(original.clone());
    f.select("cover");
    click(&mut f.editor, BACK);
    f.assert_document(&back);
    click(&mut f.editor, FRONT);
    f.assert_document(&front);
    click(&mut f.editor, UNDO);
    f.save_reopen(&back);
    // Both stacks contain a real reorder. Disabled Undo/Redo cannot pass merely
    // because one history stack happened to be empty before entering Play.
    for label in [FRONT, BACK, UNDO, REDO, SAVE] {
        assert!(!f.editor.get_by_label(label).accesskit_node().is_disabled());
    }
    let saved = project_files(&f.root);
    let scene_checksum = f.editor.state().editor.checksum();
    click(&mut f.editor, LBL_PLAY);
    wait(&mut f.editor, "actual Play", |app| {
        app.editor.is_playing_mode()
            && app.editor.snapshot().is_some_and(|snapshot| snapshot.timeline().is_some())
    });
    f.editor.get_by_label("UI authoring is read-only during Play");
    for label in [FRONT, BACK, UNDO, REDO, SAVE] {
        attempt_disabled_click(&mut f.editor, label);
        f.assert_document(&back);
        assert_eq!(project_files(&f.root), saved);
    }
    click(&mut f.editor, LBL_STOP);
    wait(&mut f.editor, "actual Stop", |app| {
        app.editor.mode() == Mode::Edit
            && app.editor.snapshot().is_some_and(|snapshot| snapshot.timeline().is_none())
    });
    assert_eq!(f.editor.state().editor.checksum(), scene_checksum);
    // Observable full history replay establishes unchanged stacks and selection
    // without exposing private history or mutating it through a test-only API.
    for (command, expected) in [(REDO, &front), (UNDO, &back), (UNDO, &original)] {
        click(&mut f.editor, command);
        f.assert_document(expected);
        assert_eq!(project_files(&f.root), saved, "history does not autosave");
    }
    assert!(f.editor.get_by_label(UNDO).accesskit_node().is_disabled());
    for expected in [&back, &front] {
        click(&mut f.editor, REDO);
        f.assert_document(expected);
    }
    assert!(f.editor.get_by_label(REDO).accesskit_node().is_disabled());
    click(&mut f.editor, BACK);
    f.save_reopen(&back);
    let mut reopened = open_editor(&f.root);
    assert_eq!(reopened.state().collect_ui.as_ref().unwrap().document(), &back);
    reopened.get_by_role_and_label(Role::ComboBox, "UI node").click();
    reopened.run_steps(3);
    click(&mut reopened, "cover");
    click(&mut reopened, FRONT);
    assert_eq!(reopened.state().collect_ui.as_ref().unwrap().document(), &front);
}

fn runtime(root: &Path) -> CollectUi {
    let prepared = open(root);
    let ui = prepared.ui().unwrap();
    CollectUi::new(ui.document.clone(), ui.font.clone()).unwrap()
}

fn draw(ui: &mut CollectUi, events: Vec<egui::Event>, phase: u32) -> Option<Action> {
    let (output, action) = ui.show(egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 900.0))),
        events,
        ..Default::default()
    }, Hud { score: 0, phase, best: "0".into() });
    assert!(!output.shapes.is_empty(), "real authored widgets must paint");
    output.drop_without_applying_deltas();
    action
}

fn runtime_click(ui: &mut CollectUi, point: egui::Pos2, phase: u32) -> Option<Action> {
    assert_eq!(draw(ui, vec![egui::Event::PointerMoved(point), pointer(point, true)], phase), None);
    draw(ui, vec![pointer(point, false)], phase)
}

fn assert_clipping(root: &Path, cover_in_front: bool) {
    let mut ui = runtime(root);
    assert_eq!(draw(&mut ui, vec![], PLAYING), None);
    for (point, when_front, explanation) in [
        (egui::pos2(100.0, 60.0), Some(Action::Restart), "visible descendant intersection"),
        (egui::pos2(150.0, 60.0), Some(Action::Play), "clipped-away descendant"),
        (egui::pos2(60.0, 30.0), None, "filled parent-only region"),
        (egui::pos2(30.0, 30.0), Some(Action::Play), "passive label over exposed lower button"),
    ] {
        let expected = if cover_in_front { when_front } else { Some(Action::Play) };
        assert_eq!(runtime_click(&mut ui, point, PLAYING), expected, "{explanation}");
        // Returned commands are deliberately not applied: dispatch acceptance
        // observes one unchanged Title layout, not a synthetic screen reset.
        assert_eq!(ui.screen(), Screen::Title);
    }
}

fn frame_trace(root: &Path) -> Vec<(Vec<u8>, u64)> {
    let prepared = open(root);
    let mut simulation = prepared.scene().simulation().unwrap();
    let mut trace = vec![(simulation.frame().to_bytes(), simulation.checksum())];
    for tick in 1..=90 {
        let mut input = orr_sim::TickInputs::new(tick, 1);
        input.set_input(PlayerSlot(0), CollectInput {
            x: if tick <= 60 { FP::ONE } else { FP::ZERO },
            ..Default::default()
        });
        simulation.step(&input);
        trace.push((simulation.frame().to_bytes(), simulation.checksum()));
    }
    trace
}

#[test]
fn layer_order_clipping_and_container_occlusion() {
    let original = clipping_document();
    let back = in_order(&original, &["cover", "child", "lower", "passive"]);
    let front = in_order(&original, &["lower", "passive", "cover", "child"]);
    let mut f = Fixture::new(original.clone());
    f.select("cover");
    let authoritative = frame_trace(&f.root);
    f.save_reopen(&original);
    assert_clipping(&f.root, true);
    for (command, expected, cover_in_front) in [
        (BACK, &back, false),
        (FRONT, &front, true),
        (UNDO, &back, false),
        (REDO, &front, true),
    ] {
        let before = project_files(&f.root);
        click(&mut f.editor, command);
        f.assert_document(expected);
        assert_eq!(project_files(&f.root), before, "reorder and history do not autosave");
        f.save_reopen(expected);
        assert_clipping(&f.root, cover_in_front);
        assert_eq!(frame_trace(&f.root), authoritative, "all Frame bytes/checksums at ticks 0..=90");
    }
}

fn key(key: egui::Key, pressed: bool) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

fn assert_keyboard(root: &Path, expected: Option<Action>) {
    for activation in [egui::Key::Enter, egui::Key::Space] {
        let mut ui = runtime(root);
        for _ in 0..2 {
            assert_eq!(draw(&mut ui, vec![], PLAYING), None);
        }
        assert_eq!(draw(&mut ui, vec![key(egui::Key::Tab, true)], PLAYING), None);
        assert!(ui.context.memory(|memory| memory.focused().is_some()), "native Tab focuses a real button");
        assert_eq!(draw(&mut ui, vec![key(egui::Key::Tab, false), key(activation, true)], PLAYING), expected);
        assert_eq!(draw(&mut ui, vec![key(activation, false)], PLAYING), None);
        assert_eq!(runtime_click(&mut ui, egui::pos2(30.0, 30.0), PLAYING), Some(Action::Play),
            "the exposed region remains clickable even when the center is occluded");
    }
}

fn assert_pointer_cancellation(root: &Path) {
    let exposed = egui::pos2(30.0, 30.0);
    let other = egui::pos2(160.0, 30.0);
    let mut ui = runtime(root);
    assert_eq!(draw(&mut ui, vec![], PLAYING), None);
    assert_eq!(draw(&mut ui, vec![pointer(exposed, true)], PLAYING), None);
    assert_eq!(draw(&mut ui, vec![pointer(other, false)], PLAYING), None,
        "a release over a different stable button cannot retarget the pending press");
    assert_eq!(runtime_click(&mut ui, other, PLAYING), Some(Action::Quit));

    for cancellation in [egui::Event::PointerGone, egui::Event::WindowFocused(false)] {
        assert_eq!(draw(&mut ui, vec![pointer(exposed, true)], PLAYING), None);
        assert_eq!(draw(&mut ui, vec![cancellation], PLAYING), None);
        assert_eq!(draw(&mut ui, vec![egui::Event::WindowFocused(true), pointer(exposed, false)], PLAYING), None);
        assert_eq!(runtime_click(&mut ui, exposed, PLAYING), Some(Action::Play), "new press works after cancellation");
    }

    assert_eq!(draw(&mut ui, vec![pointer(exposed, true)], PLAYING), None);
    ui.apply(Action::Menu);
    assert_eq!(ui.screen(), Screen::Menu);
    assert_eq!(draw(&mut ui, vec![pointer(exposed, false)], PLAYING), None, "screen change cancels ownership");
    assert_eq!(runtime_click(&mut ui, exposed, PLAYING), Some(Action::Continue));

    ui.apply(Action::Play);
    assert_eq!(draw(&mut ui, vec![], PLAYING), None);
    assert_eq!(draw(&mut ui, vec![pointer(exposed, true)], PLAYING), None);
    assert_eq!(draw(&mut ui, vec![pointer(exposed, false)], WON), None,
        "authoritative terminal transition cannot dispatch a stale release");
    assert_eq!(ui.screen(), Screen::Terminal);
    assert_eq!(runtime_click(&mut ui, exposed, WON), Some(Action::Restart));

    assert_eq!(draw(&mut ui, vec![pointer(exposed, true)], WON), None);
    // Public host-layout invalidation, without writing CollectUi's internals.
    ui.invalidate_pointer_layout();
    assert_eq!(draw(&mut ui, vec![pointer(exposed, false)], WON), None);
    assert_eq!(runtime_click(&mut ui, exposed, WON), Some(Action::Restart));
}

#[test]
fn layer_order_keyboard_focus_and_pointer_ownership() {
    let original = keyboard_document();
    let back = in_order(&original, &["cover", "lower", "other", "passive", "playing", "menu", "terminal"]);
    let front = in_order(&original, &["lower", "other", "passive", "cover", "playing", "menu", "terminal"]);
    let mut f = Fixture::new(original.clone());
    f.select("cover");
    f.save_reopen(&original);
    assert_keyboard(&f.root, None);
    assert_pointer_cancellation(&f.root);
    for (command, expected, keyboard_action) in [
        (BACK, &back, Some(Action::Play)),
        (FRONT, &front, None),
        (UNDO, &back, Some(Action::Play)),
        (REDO, &front, None),
    ] {
        click(&mut f.editor, command);
        f.save_reopen(expected);
        assert_keyboard(&f.root, keyboard_action);
        assert_pointer_cancellation(&f.root);
    }
}
