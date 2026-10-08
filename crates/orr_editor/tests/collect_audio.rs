#![cfg(feature = "collect-audio")]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
use egui_kittest::{kittest::Queryable, Harness};
use orr_editor::{Editor, EditorApp, HostSpec};
use orr_sample::{
    collect_audio::{Document, ALTERNATE_ASSET},
    collect_audio_output::Playback,
    collect_project::{PreparedProject, ProgressSupport, SpriteSupport},
};
use std::{
    path::Path,
    time::{Duration, Instant},
};
fn fixture() -> tempfile::TempDir {
    let temp = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    std::fs::write(temp.path().join("orr.project.json"),br#"{"schema":2,"engine":"*","entry":{"game":"collect-dodge-v1","scene":"scene.yaml","audio":"audio.json"}}"#).unwrap();
    std::fs::write(
        temp.path().join("scene.yaml"),
        include_bytes!("../../../scenes/collect_dodge_v1.scene.yaml"),
    )
    .unwrap();
    std::fs::write(
        temp.path().join("audio.json"),
        Document::default_collect().to_bytes().unwrap(),
    )
    .unwrap();
    let mut runtime = orr_package::Runtime::content_only();
    runtime.capabilities.insert("collect-audio".into());
    let project = orr_package::Project::open(temp.path(), runtime).unwrap();
    project
        .install(&[Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/packages/collect-audio-v1")
            .canonicalize()
            .unwrap()])
        .unwrap();
    temp
}
fn open(path: &Path) -> PreparedProject {
    PreparedProject::open_with_audio(
        path,
        ProgressSupport::Unsupported,
        SpriteSupport::Unsupported,
        false,
        true,
    )
    .unwrap()
}
fn editor(project: &PreparedProject) -> Editor {
    let mut editor = Editor::start(&HostSpec::PreparedCollect {
        scene: project.path().into(),
        text: project.scene().text().into(),
        listen: None,
        debug_hooks: false,
    })
    .unwrap();
    editor.sync();
    editor
}
fn harness(path: &Path) -> Harness<'static, EditorApp> {
    harness_gpu(path, false)
}
fn harness_gpu(path: &Path, gpu: bool) -> Harness<'static, EditorApp> {
    let project = open(path);
    let editor = editor(&project);
    let builder = Harness::builder().with_size([1600.0, 1200.0]);
    if gpu {
        builder.wgpu().build_eframe(move |cc| {
            let mut app = EditorApp::new(editor, cc.wgpu_render_state.clone());
            orr_editor::project::install_collect_presentation(&mut app, project, &cc.egui_ctx);
            app
        })
    } else {
        builder.build_eframe(move |cc| {
            let mut app = EditorApp::new(editor, None);
            orr_editor::project::install_collect_presentation(&mut app, project, &cc.egui_ctx);
            app
        })
    }
}

fn wait(h: &mut Harness<'_, EditorApp>, ready: impl Fn(&Editor) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready(&h.state().editor) {
        assert!(Instant::now() < deadline, "{:?}", h.state().editor.status());
        h.run_steps(1);
        std::thread::sleep(Duration::from_millis(2));
    }
}
#[test]
fn actual_audio_widgets_assign_preview_undo_redo_save_reopen() {
    let temp = fixture();
    let mut h = harness(temp.path());
    h.get_by_label("Collect pickup audio").click();
    h.run_steps(2);
    h.get_by_label("Pickup clip").click();
    h.run_steps(1);
    h.get_by_label(ALTERNATE_ASSET).click();
    h.run_steps(2);
    assert_eq!(
        h.state()
            .collect_audio
            .as_ref()
            .unwrap()
            .document()
            .pickup
            .asset,
        ALTERNATE_ASSET
    );
    h.get_by(|node| {
        node.role() == egui::accesskit::Role::Slider
            && node.label().as_deref() == Some("Gain (0–1000)")
    })
    .focus();
    h.key_press(egui::Key::ArrowLeft);
    h.run_steps(1);
    assert!(h.state().collect_audio.as_ref().unwrap().document().gain < 700);
    h.get_by_label("Mute pickup").click();
    h.run_steps(1);
    assert!(h.state().collect_audio.as_ref().unwrap().document().mute);
    h.get_by_label("Undo audio").click();
    h.run_steps(1);
    assert!(!h.state().collect_audio.as_ref().unwrap().document().mute);
    h.get_by_label("Redo audio").click();
    h.run_steps(1);
    assert!(h.state().collect_audio.as_ref().unwrap().document().mute);
    h.get_by_label("Mute pickup").click();
    h.run_steps(1);
    {
        let app = h.state_mut();
        app.collect_audio
            .as_mut()
            .unwrap()
            .preview_offline(&app.editor)
            .unwrap();
    }
    h.get_by_label("Preview pickup").click();
    h.run_steps(1);
    let mut pcm = vec![0.0; 48_000];
    h.state_mut()
        .collect_audio
        .as_mut()
        .unwrap()
        .render_preview(&mut pcm)
        .unwrap();
    assert!(pcm.iter().all(|v| v.is_finite() && v.abs() <= 1.0));
    assert!(pcm.iter().any(|v| v.abs() > 0.00001));
    h.get_by_label("Stop preview").click();
    h.run_steps(1);
    assert!(h
        .state_mut()
        .collect_audio
        .as_mut()
        .unwrap()
        .render_preview(&mut pcm)
        .is_err());
    h.get_by_label("Save audio").click();
    h.run_steps(1);
    assert_eq!(
        open(temp.path()).audio().unwrap().document,
        h.state().collect_audio.as_ref().unwrap().document().clone()
    );
}
#[test]
fn actual_editor_play_receives_authoritative_pickups_once() {
    actual_play(false);
}
#[test]
#[ignore = "mandatory explicit software-GPU actual EditorApp audio acceptance"]
fn actual_audio_editor_gpu_play_and_pcm() {
    actual_play(true);
}
fn actual_play(gpu: bool) {
    let temp = fixture();
    let prepared = open(temp.path()).audio().unwrap().clone();
    let mut h = harness_gpu(temp.path(), gpu);
    *h.state_mut().editor.collect_audio_mut().unwrap() = Playback::offline(prepared).unwrap();
    for round in 1..=2 {
        h.get_by_label(orr_editor::app::LBL_PLAY).click();
        wait(&mut h, |e| e.can_take_control());
        h.get_by_label("Take control").click();
        wait(&mut h, |e| {
            e.input_phase() == orr_editor::editor::input::Phase::Active
        });
        h.event(egui::Event::Key {
            key: egui::Key::D,
            physical_key: Some(egui::Key::D),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
        wait(&mut h, |e| {
            e.snapshot().is_some_and(|s| {
                s.predicted()
                    .singleton::<orr_sample::collect_game::CollectRun>()
                    .phase
                    == orr_sample::collect_game::WON
            })
        });
        let audio = h.state_mut().editor.collect_audio_mut().unwrap();
        assert_eq!(audio.stats().started, 2 * round);
        let mut pcm = vec![0.0; 48_000];
        audio.render(&mut pcm).unwrap();
        assert!(pcm.iter().any(|v| v.abs() > 0.00001));
        h.run_steps(5);
        assert_eq!(
            h.state_mut()
                .editor
                .collect_audio_mut()
                .unwrap()
                .stats()
                .started,
            2 * round
        );
        if gpu && round == 2 {
            assert!(h.state().has_gpu());
            let adapter = h
                .state()
                .viewport_gpu()
                .expect("actual GPU viewport")
                .gpu()
                .adapter_name();
            let name = adapter.to_ascii_lowercase();
            assert!(
                name.contains("llvmpipe")
                    || name.contains("lavapipe")
                    || name.contains("swiftshader"),
                "software adapter required, got {adapter}"
            );
            println!("Collect audio editor software adapter: {adapter}");
            h.get_by_label("Collect pickup audio").click();
            h.run_steps(2);
            let image = h.render().expect("mandatory actual editor GPU render");
            assert!(image.width() >= 1400 && image.height() >= 1000);
            if let Some(path) = std::env::var_os("ORR_COLLECT_AUDIO_CAPTURES") {
                let path = std::path::PathBuf::from(path);
                std::fs::create_dir_all(&path).unwrap();
                image
                    .save(path.join("collect-audio-editor-won.png"))
                    .unwrap();
            }
        }
        h.event(egui::Event::Key {
            key: egui::Key::D,
            physical_key: Some(egui::Key::D),
            pressed: false,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
        h.get_by_label(orr_editor::app::LBL_STOP).click();
        wait(&mut h, |e| e.mode() == orr_editor::Mode::Edit);
    }
}

#[test]
fn failed_audio_save_and_same_path_restart_retire_old_panel() {
    let temp = fixture();
    let p = open(temp.path());
    let mut editor = editor(&p);
    let mut panel =
        orr_editor::collect_audio_panel::Panel::new(p.audio().unwrap().clone(), &mut editor)
            .unwrap();
    let original = std::fs::read(temp.path().join("audio.json")).unwrap();
    let mut next = panel.document().clone();
    next.gain = 123;
    panel.change(next, &mut editor).unwrap();
    std::fs::write(temp.path().join("audio.json"), b"external").unwrap();
    assert!(panel.save(&editor).is_err());
    assert_eq!(
        std::fs::read(temp.path().join("audio.json")).unwrap(),
        b"external"
    );
    std::fs::write(temp.path().join("audio.json"), &original).unwrap();
    assert!(editor.restart());
    editor.sync();
    assert!(panel.preview(&editor).is_err());
    assert!(panel.save(&editor).is_err());
    assert_eq!(
        std::fs::read(temp.path().join("audio.json")).unwrap(),
        original
    );
}

#[test]
fn audio_save_rejects_external_package_replacement_without_overwriting_sidecar() {
    let temp = fixture();
    let prepared = open(temp.path());
    let mut editor = editor(&prepared);
    let mut panel =
        orr_editor::collect_audio_panel::Panel::new(prepared.audio().unwrap().clone(), &mut editor)
            .unwrap();
    let original = std::fs::read(temp.path().join("audio.json")).unwrap();
    let mut next = panel.document().clone();
    next.pickup.asset = ALTERNATE_ASSET.into();
    panel.change(next, &mut editor).unwrap();
    let source = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let snapshot = &prepared.audio().unwrap().package;
    for (name, bytes) in &snapshot.files {
        let path = source.path().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
    let mut manifest = snapshot.locked.manifest.clone();
    manifest.version = "1.0.1".into();
    std::fs::write(
        source.path().join("orr.package.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    let mut runtime = orr_package::Runtime::content_only();
    runtime.capabilities.insert("collect-audio".into());
    orr_package::Project::open(temp.path(), runtime)
        .unwrap()
        .install(&[source.path().to_path_buf()])
        .unwrap();
    // The external update itself remains valid, but this panel's bank is stale.
    assert_eq!(
        open(temp.path()).audio().unwrap().document.pickup.asset,
        Document::default_collect().pickup.asset
    );
    assert!(panel
        .save(&editor)
        .unwrap_err()
        .contains("audio package changed"));
    assert_eq!(
        std::fs::read(temp.path().join("audio.json")).unwrap(),
        original
    );
}
