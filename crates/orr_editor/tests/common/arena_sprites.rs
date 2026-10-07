//! Package-backed fixture shared by the Arena presentation acceptance tests.
#![allow(dead_code, clippy::disallowed_types, clippy::float_arithmetic)]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use egui_kittest::{kittest::Queryable, Harness};
use orr_editor::{
    game::EditorGame,
    sprite_bindings::{Document, Source},
    Editor, EditorApp,
};
use orr_package::{Project, Runtime};

pub const HERO: &str = "e_00000001";
pub const TARGET: &str = "e_00000002";
pub const PACKAGE: &str = "sample-sprites";
pub const SCENE: &str = "schema: orr.scene/1\nsingletons:\n  Score: { kills: [0,0,0,0,0,0,0,0] }\nentities:\n  e_00000001:\n    name: hero\n    Position: { pos: [-60,0] }\n    PlayerTag: { slot: 0 }\n  e_00000002:\n    name: target\n    Position: { pos: [60,0] }\n    PlayerTag: { slot: 1 }\n";

pub struct Fixture {
    pub root: tempfile::TempDir,
    pub scene: PathBuf,
    pub sidecar: PathBuf,
}

impl Fixture {
    pub fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/sprite_demo")
            .canonicalize()
            .unwrap();
        Project::open_for_install(root.path(), Runtime::content_only().engine_version)
            .unwrap()
            .install(&[source])
            .unwrap();
        let scene = root.path().join("arena.scene.yaml");
        let sidecar = root.path().join("arena.sprites.json");
        std::fs::write(&scene, SCENE).unwrap();
        Self {
            root,
            scene,
            sidecar,
        }
    }

    pub fn editor(&self) -> Editor {
        let mut editor = Editor::open_game(&self.scene, EditorGame::Arena).unwrap();
        editor.sync();
        assert_eq!(editor.game(), EditorGame::Arena);
        assert_eq!(editor.rows().len(), 2);
        // Deliberate fixture framing: both separately authored actors are visible.
        editor.camera = orr_render::Camera::new([0.0, 0.0], 170.0);
        editor
    }

    pub fn headless(&self) -> Harness<'static, EditorApp> {
        let editor = self.editor();
        Harness::builder()
            .with_size([1500.0, 1400.0])
            .with_step_dt(1.0 / 60.0)
            .build_eframe(move |_| EditorApp::new(editor, None))
    }
}

pub fn settle(h: &mut Harness<'_, EditorApp>) {
    for _ in 0..2 {
        h.state_mut().editor.sync();
        h.run_steps(2);
    }
    assert!(
        h.state().editor.down().is_none(),
        "local Arena host remains connected"
    );
}

pub fn click(h: &mut Harness<'_, EditorApp>, label: &str) {
    h.get_by_role_and_label(egui::accesskit::Role::Button, label)
        .scroll_to_me();
    h.run_steps(2);
    h.get_by_role_and_label(egui::accesskit::Role::Button, label)
        .click();
    settle(h);
}

pub fn no_error(h: &Harness<'_, EditorApp>) {
    assert!(
        h.state().sprites.error().is_none(),
        "{:?}",
        h.state().sprites.error()
    );
}

pub fn document<'a>(h: &'a Harness<'_, EditorApp>) -> &'a Document {
    h.state().sprites.bindings.as_ref().unwrap().document()
}

pub fn configure(h: &mut Harness<'_, EditorApp>, fixture: &Fixture) {
    settle(h);
    click(h, "hero");
    click(h, "Sprite bindings (view only)");
    {
        let panel = &mut h.state_mut().sprites;
        panel.path = fixture.sidecar.to_string_lossy().into_owned();
        panel.scene = "arena.scene.yaml".into();
        panel.project = ".".into();
    }
    click(h, "Create bindings");
    no_error(h);
    {
        let panel = &mut h.state_mut().sprites;
        panel.package = PACKAGE.into();
        panel.document = "sprites.json".into();
        panel.scale = 2.0;
    }
    click(h, "Load sprite document");
    no_error(h);
}

pub fn assign(h: &mut Harness<'_, EditorApp>, entity: &str, source: Source) {
    click(h, entity);
    h.state_mut().sprites.source = source;
    click(h, "Assign sprite to selection");
    no_error(h);
}

/// Choose actual source/clip widgets. Typed drafts are reserved for fixture
/// paths, scale, and deliberately invalid-input regression setup.
pub fn two_locomotion_bindings(h: &mut Harness<'_, EditorApp>, fixture: &Fixture) {
    use egui::accesskit::Role;
    configure(h, fixture);
    h.get_by_role_and_label(Role::ComboBox, "Sprite source")
        .click();
    h.run_steps(2);
    click(h, "Idle / walk");
    assert!(matches!(
        h.state().sprites.source,
        Source::Locomotion { .. }
    ));
    click(h, "Assign sprite to selection");
    no_error(h);
    click(h, "target");
    // Intentionally different clips: a global motion/clip state cannot satisfy this.
    for (label, value) in [("Idle clip", "walk"), ("Walk clip", "idle")] {
        h.get_by_role_and_label(Role::ComboBox, label)
            .scroll_to_me();
        h.run_steps(2);
        h.get_by_role_and_label(Role::ComboBox, label).click();
        h.run_steps(2);
        h.get_by_role_and_label(Role::Button, value).click();
        settle(h);
    }
    click(h, "Assign sprite to selection");
    no_error(h);
    assert_eq!(document(h).bindings.len(), 2);
    assert_eq!(
        document(h).bindings[HERO].source,
        Source::Locomotion {
            idle: "idle".into(),
            walk: "walk".into()
        }
    );
    assert_eq!(
        document(h).bindings[TARGET].source,
        Source::Locomotion {
            idle: "walk".into(),
            walk: "idle".into()
        }
    );
}

pub fn wait_for(h: &mut Harness<'_, EditorApp>, label: &str, ready: impl Fn(&EditorApp) -> bool) {
    let until = Instant::now() + Duration::from_secs(10);
    while !ready(h.state()) {
        assert!(Instant::now() < until,
            "timed out waiting for {label}: mode={:?}, timeline={:?}, input={:?}, sprite_error={:?}",
            h.state().editor.mode(), h.state().editor.timeline(), h.state().editor.input_phase(),
            h.state().sprites.error());
        h.run_steps(1);
        std::thread::sleep(Duration::from_millis(5));
    }
}

pub fn key(h: &Harness<'_, EditorApp>, key: egui::Key, pressed: bool) {
    h.event(egui::Event::Key {
        key,
        physical_key: Some(key),
        pressed,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
}

pub fn body_position(app: &EditorApp, guid: &str) -> [f32; 2] {
    let row = app
        .editor
        .rows()
        .iter()
        .find(|row| {
            row.guid
                .as_ref()
                .is_some_and(|value| value.as_str() == guid)
        })
        .unwrap();
    app.editor
        .bodies()
        .iter()
        .find(|body| body.entity == row.entity)
        .unwrap()
        .pos
}

pub fn moving(app: &EditorApp, guid: &str) -> bool {
    app.sprites.playback.state(guid).moving
}

pub fn region(app: &EditorApp, guid: &str) -> Option<u32> {
    app.sprites.sampled_region(&app.editor, guid)
}

pub fn pan_viewport(h: &mut Harness<'_, EditorApp>) {
    let start = h.state().ui.viewport_rect.unwrap().center();
    let end = start + egui::vec2(75.0, 35.0);
    h.event(egui::Event::PointerMoved(start));
    h.run_steps(1);
    h.event(egui::Event::PointerButton {
        pos: start,
        button: egui::PointerButton::Middle,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    h.run_steps(1);
    h.event(egui::Event::PointerMoved(end));
    h.run_steps(2);
    h.event(egui::Event::PointerButton {
        pos: end,
        button: egui::PointerButton::Middle,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    h.run_steps(2);
}
