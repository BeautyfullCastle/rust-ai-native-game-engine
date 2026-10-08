#![cfg(feature = "collect-dodge")]
#![allow(clippy::disallowed_types)]
use egui_kittest::{kittest::Queryable, Harness};
use orr_editor::editor::input::Phase;
use orr_editor::game::EditorGame;
use orr_editor::{Editor, EditorApp, HostSpec, Owner};
use orr_fp::FP;
use orr_reflect::Value;
use orr_sample::{
    collect_game::*,
    collect_project::{PreparedProject, ACTOR},
};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Project(PathBuf);
impl Project {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "collect-editor-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&p).unwrap();
        std::fs::write(
            p.join("orr.project.json"),
            include_bytes!("../../../assets/collect_dodge_project/orr.project.json"),
        )
        .unwrap();
        std::fs::write(
            p.join("level.scene.yaml"),
            include_bytes!("../../../scenes/collect_dodge_v1.scene.yaml"),
        )
        .unwrap();
        Self(p)
    }
    fn editor(&self) -> Editor {
        let p = PreparedProject::open(&self.0).unwrap();
        let spec = HostSpec::PreparedCollect {
            scene: p.path().to_path_buf(),
            text: p.scene().text().into(),
            listen: None,
            debug_hooks: false,
        };
        let mut e = Editor::start(&spec).unwrap();
        e.sync();
        e
    }
}
impl Drop for Project {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn wait(h: &mut Harness<'_, EditorApp>, label: &str, ready: impl Fn(&Editor) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while !ready(&h.state().editor) {
        assert!(
            Instant::now() < deadline,
            "{label}: {:?}",
            h.state().editor.status()
        );
        h.run_steps(1);
        std::thread::sleep(Duration::from_millis(2));
    }
}
#[test]
fn editor_authoring_save_reopen_and_runtime_initial_frame_match() {
    let project = Project::new();
    let mut ed = project.editor();
    assert_eq!(ed.game(), EditorGame::CollectDodge);
    assert_eq!(ed.bodies().len(), 4);
    let before = ed.checksum();
    assert!(ed.select_named("first_collectible"));
    ed.sync();
    assert!(ed.set_field(
        &Owner::Component(ACTOR.into()),
        "position.x",
        Value::Fixed(FP::from_int(12))
    ));
    ed.sync();
    assert_ne!(ed.checksum(), before);
    let edited = ed.checksum();
    assert!(ed.undo());
    ed.sync();
    assert_eq!(ed.checksum(), before);
    assert!(ed.redo());
    ed.sync();
    assert_eq!(ed.checksum(), edited);
    ed.host_call("scene.save", serde_json::json!({"write":true}))
        .unwrap();
    let prepared = PreparedProject::open(&project.0).unwrap();
    assert_eq!(prepared.scene().frame().checksum(), edited);
    drop(ed);
    let reopened = project.editor();
    assert_eq!(reopened.checksum(), edited);
}
#[test]
fn real_editor_app_buttons_keyboard_collect_win_and_restart() {
    let project = Project::new();
    let editor = project.editor();
    let mut h = Harness::builder()
        .with_size([1400.0, 850.0])
        .build_eframe(|_| EditorApp::new(editor, None));
    h.get_by_label(orr_editor::app::LBL_PLAY).click();
    wait(&mut h, "Play", |e| e.can_take_control());
    h.get_by_label("Take control").click();
    wait(&mut h, "input claim", |e| e.input_phase() == Phase::Active);
    h.event(egui::Event::Key {
        key: egui::Key::D,
        physical_key: Some(egui::Key::D),
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
    wait(&mut h, "win", |e| {
        e.snapshot()
            .is_some_and(|s| s.predicted().singleton::<CollectRun>().phase == WON)
    });
    h.event(egui::Event::Key {
        key: egui::Key::D,
        physical_key: Some(egui::Key::D),
        pressed: false,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
    // Deliberately press and release inside one GUI frame, shorter than a sim tick.
    h.event(egui::Event::Key {
        key: egui::Key::Space,
        physical_key: Some(egui::Key::Space),
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
    h.event(egui::Event::Key {
        key: egui::Key::Space,
        physical_key: Some(egui::Key::Space),
        pressed: false,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
    wait(&mut h, "restart", |e| {
        e.snapshot().is_some_and(|s| {
            let r = s.predicted().singleton::<CollectRun>();
            r.phase == PLAYING && r.score == 0
        })
    });
    h.get_by_label(orr_editor::app::LBL_STOP).click();
    wait(&mut h, "Stop", |e| e.mode() == orr_editor::Mode::Edit);
    assert_eq!(h.state().editor.input_phase(), Phase::Off);
}

#[test]
#[ignore = "mandatory explicit software-GPU EditorApp acceptance"]
fn collect_actual_editor_gpu_edit_save_play_reopen() {
    use egui::accesskit::Role;
    let project = Project::new();
    let editor = project.editor();
    let mut h = Harness::builder()
        .with_size([1400.0, 850.0])
        .wgpu()
        .build_eframe(|cc| EditorApp::new(editor, cc.wgpu_render_state.clone()));
    assert!(h.state().has_gpu());
    h.get_by_label("first_collectible").click();
    wait(&mut h, "selection", |e| e.inspect().is_some());
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
    wait(&mut h, "GUI authored edit", |e| {
        e.bodies().iter().any(|b| b.pos == [12.0, 0.0])
    });
    h.get_by_label("File").click();
    h.run_steps(2);
    h.get_by(|n| {
        n.role() == Role::Button
            && n.label()
                .is_some_and(|s| s.starts_with("Save ") && !s.starts_with("Save As"))
    })
    .click();
    wait(&mut h, "saved", |e| !e.is_dirty());
    let edited = h.state().editor.checksum();
    assert_eq!(
        PreparedProject::open(&project.0)
            .unwrap()
            .scene()
            .frame()
            .checksum(),
        edited
    );
    fn capture(h: &mut Harness<'_, EditorApp>, name: &str) {
        h.run_steps(2);
        let image = h.render().expect("mandatory real composed editor GPU");
        let rect = h.state().ui.viewport_rect.unwrap();
        let mut colored = 0;
        for y in rect.min.y.ceil() as u32..rect.max.y.floor() as u32 {
            for x in rect.min.x.ceil() as u32..rect.max.x.floor() as u32 {
                let p = image.get_pixel(x, y).0;
                if p[0] > 180 && p[1] < 160 && p[2] < 160 || p[2] > 180 && p[0] < 160 {
                    colored += 1;
                }
            }
        }
        assert!(
            colored > 30,
            "actual blue player/red hazard missing from viewport"
        );
        if let Some(path) = std::env::var_os("ORR_COLLECT_CAPTURES") {
            let path = PathBuf::from(path);
            std::fs::create_dir_all(&path).unwrap();
            image.save(path.join(format!("{name}.png"))).unwrap();
        }
    }
    capture(&mut h, "editor-edited");
    export_actual_editor_project(&project);
    h.get_by_label(orr_editor::app::LBL_PLAY).click();
    wait(&mut h, "Play", |e| e.can_take_control());
    h.get_by_label("Take control").click();
    wait(&mut h, "claim", |e| e.input_phase() == Phase::Active);
    h.event(egui::Event::Key {
        key: egui::Key::D,
        physical_key: Some(egui::Key::D),
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
    wait(&mut h, "WON", |e| {
        e.snapshot()
            .is_some_and(|s| s.predicted().singleton::<CollectRun>().phase == WON)
    });
    h.event(egui::Event::Key {
        key: egui::Key::D,
        physical_key: Some(egui::Key::D),
        pressed: false,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
    capture(&mut h, "editor-won");
    h.get_by_label(orr_editor::app::LBL_STOP).click();
    wait(&mut h, "Stop", |e| e.mode() == orr_editor::Mode::Edit);
    drop(h);
    let reopened = project.editor();
    assert_eq!(reopened.checksum(), edited);
}

// The exported bytes come from the project just saved through real GUI widgets.
fn export_actual_editor_project(project: &Project) {
    use std::process::Command;
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .unwrap();
    let runtime =
        PathBuf::from(std::env::var_os("ORR_COLLECT_RUNTIME").expect("mandatory built runtime"))
            .canonicalize()
            .unwrap();
    let exporter =
        PathBuf::from(std::env::var_os("ORR_COLLECT_EXPORTER").expect("mandatory built exporter"))
            .canonicalize()
            .unwrap();
    assert!(runtime.starts_with(&workspace));
    assert!(exporter.starts_with(&workspace));
    let hash = Command::new("sha256sum").arg(&runtime).output().unwrap();
    assert!(hash.status.success());
    let hash = String::from_utf8(hash.stdout)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_string();
    assert_eq!(hash.len(), 64);
    let bundle = Project(project.0.with_extension("export"));
    let result = Command::new(&exporter)
        .arg("--project")
        .arg(&project.0)
        .arg("--runtime")
        .arg(&runtime)
        .args(["--runtime-sha256", &hash, "--trusted-runtime", "--output"])
        .arg(&bundle.0)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let before = std::fs::read(project.0.join("level.scene.yaml")).unwrap();
    assert_eq!(
        before,
        std::fs::read(bundle.0.join("project/level.scene.yaml")).unwrap()
    );
    let capture = project.0.with_extension("export.png");
    let result = Command::new("bwrap")
        .args([
            "--die-with-parent",
            "--ro-bind",
            "/",
            "/",
            "--dev-bind",
            "/dev/null",
            "/dev/null",
            "--bind",
            "/tmp",
            "/tmp",
            "--tmpfs",
        ])
        .arg(&workspace)
        .arg("--tmpfs")
        .arg(&project.0)
        .arg("--ro-bind")
        .arg(&bundle.0)
        .arg(&bundle.0)
        .arg("--")
        .arg(bundle.0.join("run-collect-dodge"))
        .args([
            "--headless",
            "--ticks",
            "16",
            "--hold",
            "right",
            "--capture",
        ])
        .arg(&capture)
        .current_dir("/tmp")
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let stdout = String::from_utf8(result.stdout).unwrap();
    assert!(stdout.contains("WON | score 2/2"));
    assert!(stdout.contains("software: true"));
    let prepared = PreparedProject::open(&project.0).unwrap();
    let mut sim = prepared.scene().simulation().unwrap();
    for tick in 1..=16 {
        let mut input = orr_sim::TickInputs::new(tick, 1);
        input.set_input(
            orr_bridge::PlayerSlot(0),
            CollectInput {
                x: FP::ONE,
                ..Default::default()
            },
        );
        sim.step(&input);
    }
    assert!(stdout.contains(&format!(
        "collect tick: 16 checksum: 0x{:016x}",
        sim.checksum()
    )));
    assert_eq!(
        before,
        std::fs::read(project.0.join("level.scene.yaml")).unwrap()
    );
    if let Some(path) = std::env::var_os("ORR_COLLECT_CAPTURES") {
        std::fs::copy(
            &capture,
            PathBuf::from(path).join("editor-saved-export-won.png"),
        )
        .unwrap();
    }
    std::fs::remove_file(capture).unwrap();
}
