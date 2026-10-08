//! Generated project through the real EditorApp widgets and owned host admission.
#![cfg(all(
    feature = "navigation-project",
    feature = "project-create",
    target_os = "linux"
))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
use egui::{accesskit::Role, Key, Modifiers};
use egui_kittest::{kittest::Queryable, Harness};
use orr_editor::{
    app::{LBL_PLAY, LBL_STOP},
    Editor, EditorApp, HostSpec, Mode,
};
use orr_sample::{
    navigation_project::PreparedProject,
    project_create::{create, CreateOptions, NAVIGATION_TEMPLATE},
};

fn prepared_editor(project: &PreparedProject) -> Editor {
    let mut editor = Editor::start(&HostSpec::PreparedNavigation {
        project_root: project.root().to_path_buf(),
        reload: false,
        scene: project.path().to_path_buf(),
        text: project.scene().text().to_owned(),
        terrain_bytes: project.terrain_bytes().to_vec(),
        listen: None,
        debug_hooks: false,
    })
    .unwrap();
    editor.sync();
    editor
}
fn harness(editor: Editor) -> Harness<'static, EditorApp> {
    Harness::builder()
        .with_size([1280.0, 900.0])
        .with_step_dt(1.0 / 60.0)
        .build_eframe(move |_| EditorApp::new(editor, None))
}
fn settle(h: &mut Harness<'_, EditorApp>) {
    for _ in 0..4 {
        h.state_mut().editor.sync();
        h.run_steps(2);
    }
}
fn click(h: &mut Harness<'_, EditorApp>, label: &str) {
    h.get_by_label(label).scroll_to_me();
    h.run_steps(3);
    h.get_by_label(label).click();
    settle(h);
}
fn type_into(h: &mut Harness<'_, EditorApp>, label: &str, text: &str) {
    h.get_by_role_and_label(Role::TextInput, label)
        .scroll_to_me();
    h.run_steps(2);
    h.get_by_role_and_label(Role::TextInput, label).focus();
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run_steps(2);
    h.get_by_role_and_label(Role::TextInput, label)
        .type_text(text);
    h.run_steps(2);
    h.key_press(Key::Enter);
    settle(h);
}
fn standalone_at(project: &PreparedProject, tick: u32) -> u64 {
    let mut simulation = project.scene().simulation().unwrap();
    for tick in 1..=tick {
        simulation.step(&orr_sim::TickInputs::new(u64::from(tick), 2));
    }
    simulation.frame().checksum()
}

#[test]
fn generated_project_widget_edits_build_save_undo_reopen_and_frame_parity() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap().join("project");
    create(&CreateOptions {
        output: root.clone(),
        template: NAVIGATION_TEMPLATE.into(),
        seed: "editor-route".into(),
    })
    .unwrap();
    let project = PreparedProject::open(&root).unwrap();
    let initial = project.scene().frame().checksum();
    let mut h = harness(prepared_editor(&project));
    settle(&mut h);
    assert_eq!(h.state().editor.checksum(), initial);
    click(&mut h, "Terrain authoring");
    click(&mut h, "Point navigation");
    click(&mut h, "Open terrain");
    type_into(&mut h, "Navigation start X", "-2.75");
    type_into(&mut h, "Navigation goal X", "2.75");
    type_into(&mut h, "Navigation maximum slope", "0.75");
    type_into(&mut h, "Navigation distance per tick", "0.25");
    click(&mut h, LBL_PLAY);
    assert_eq!(h.state().editor.mode(), Mode::Edit);
    type_into(&mut h, "Navigation goal X", "0");
    click(&mut h, "Build route");
    assert!(
        h.state().navigation.error.is_some(),
        "goal lies in central hole"
    );
    assert_eq!(
        h.state().editor.checksum(),
        initial,
        "failed build preserves Frame"
    );
    type_into(&mut h, "Navigation goal X", "2.75");
    click(&mut h, "Build route");
    assert!(
        h.state().navigation.error.is_none(),
        "{:?}",
        h.state().navigation.error
    );
    let changed = h.state().editor.checksum();
    assert_ne!(changed, initial);
    assert!(h.state_mut().editor.save());
    assert!(h.state_mut().editor.undo());
    settle(&mut h);
    assert_eq!(h.state().editor.checksum(), initial);
    assert!(h.state_mut().editor.redo());
    settle(&mut h);
    assert_eq!(h.state().editor.checksum(), changed);
    click(&mut h, "Vertex");
    type_into(&mut h, "Terrain height", "0.125");
    click(&mut h, "Apply terrain height");
    click(&mut h, "Save terrain");
    click(&mut h, "Close terrain");
    click(&mut h, LBL_PLAY);
    assert_eq!(
        h.state().editor.mode(),
        Mode::Edit,
        "saved source stays stale after Close"
    );
    assert!(
        PreparedProject::open(&root).is_err(),
        "saved old pin rejects edited terrain"
    );
    click(&mut h, "Open terrain");
    click(&mut h, "Build route");
    assert!(
        h.state().navigation.error.is_none(),
        "{:?}",
        h.state().navigation.error
    );
    click(&mut h, "Cell");
    click(&mut h, "Terrain cell is a hole");
    click(&mut h, "Apply terrain hole");
    click(&mut h, LBL_PLAY);
    assert_eq!(h.state().editor.mode(), Mode::Edit);
    click(&mut h, "Save terrain");
    click(&mut h, "Build route");
    assert!(
        h.state().navigation.error.is_none(),
        "{:?}",
        h.state().navigation.error
    );
    assert!(h.state_mut().editor.save());
    settle(&mut h);
    let saved = h.state().editor.checksum();
    drop(h);
    let reopened = PreparedProject::open(&root).unwrap();
    assert_eq!(reopened.scene().frame().checksum(), saved);
    let mut h = harness(prepared_editor(&reopened));
    settle(&mut h);
    for tick in [1, 16, 40, 300] {
        h.state_mut().editor.seek(0);
        settle(&mut h);
        h.state_mut().editor.step(tick);
        settle(&mut h);
        assert_eq!(h.state().editor.snapshot().unwrap().tick(), u64::from(tick));
        assert_eq!(h.state().editor.checksum(), standalone_at(&reopened, tick));
    }
    let snapshot = h.state().editor.snapshot().unwrap();
    let navigation =
        orr_remote::navigation_yard3d::navigation_from_view(snapshot.predicted()).unwrap();
    assert_eq!(
        navigation.navigator.status(),
        orr_navigation::NavigationStatus::Arrived
    );
    assert_eq!(
        navigation.navigator.position(),
        navigation
            .graph
            .project(&navigation.terrain, navigation.spec.goal)
            .unwrap()
            .position
    );
    click(&mut h, LBL_STOP);
    assert_eq!(h.state().editor.mode(), Mode::Edit);
    assert_eq!(h.state().editor.checksum(), saved);
}

#[test]
fn prepared_initial_host_uses_owned_bytes_and_reopen_observes_current_source() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap().join("project");
    create(&CreateOptions {
        output: root.clone(),
        template: NAVIGATION_TEMPLATE.into(),
        seed: "owned".into(),
    })
    .unwrap();
    let project = PreparedProject::open(&root).unwrap();
    std::fs::write(project.terrain_path(), b"tampered after admission").unwrap();
    let mut editor = prepared_editor(&project);
    assert_eq!(editor.checksum(), project.scene().frame().checksum());
    assert!(PreparedProject::open(&root).is_err());
    assert!(
        !editor.restart(),
        "restart must re-admit current project sources"
    );
    std::fs::write(project.terrain_path(), project.terrain_bytes()).unwrap();
    std::fs::write(
        project.path(),
        include_str!("../../../scenes/navigation_blank.scene.yaml"),
    )
    .unwrap();
    assert!(
        !editor.restart(),
        "closed project must never downgrade to empty legacy staging"
    );
    std::fs::write(project.path(), project.scene().text()).unwrap();
    assert!(editor.restart());
    editor.sync();
    assert_eq!(editor.checksum(), project.scene().frame().checksum());
}

#[test]
fn generated_project_production_editor_gpu_start_mid_reached_and_resize() {
    use orr_navigation::NavigationStatus;
    use orr_remote::navigation_yard3d::navigation_from_view;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap().join("project");
    create(&CreateOptions {
        output: root.clone(),
        template: NAVIGATION_TEMPLATE.into(),
        seed: "editor-gpu".into(),
    })
    .unwrap();
    let project = PreparedProject::open(&root).unwrap();
    let mut editor = prepared_editor(&project);
    editor.camera3d = orr_sample::navigation_project_view::NavigationCamera::new(
        orr_bridge::FrameView::of(project.scene().frame()),
    )
    .unwrap()
    .orbit;
    let mut h = Harness::builder()
        .with_size([1280.0, 900.0])
        .with_step_dt(1.0 / 60.0)
        .wgpu()
        .build_eframe(|cc| {
            let state = cc
                .wgpu_render_state
                .as_ref()
                .expect("required software GPU");
            assert_eq!(
                state.adapter.get_info().device_type,
                egui_wgpu::wgpu::DeviceType::Cpu,
                "mandatory software-GPU editor acceptance"
            );
            eprintln!(
                "generated navigation editor adapter {:?}",
                state.adapter.get_info()
            );
            EditorApp::new(editor, cc.wgpu_render_state.clone())
        });
    // Keep the same production Play viewport layout at every capture. Edit
    // mode has no timeline and therefore a different framebuffer height.
    settle(&mut h);
    assert!(h.state_mut().editor.start_play());
    h.state_mut().editor.control(orr_bridge::ControlOp::Pause);
    settle(&mut h);
    assert_eq!(h.state().editor.snapshot().unwrap().tick(), 0);
    let mut prior = Vec::new();
    for (tick, label) in [(0, "start"), (24, "mid"), (300, "reached")] {
        if tick != 0 {
            h.state_mut().editor.seek(0);
            settle(&mut h);
            h.state_mut().editor.step(tick);
        }
        settle(&mut h);
        let frame = h.state().editor.snapshot().unwrap();
        let decoded = navigation_from_view(frame.predicted()).unwrap();
        assert_eq!(h.state().editor.checksum(), standalone_at(&project, tick));
        assert_eq!(
            decoded.navigator.status(),
            if tick == 300 {
                NavigationStatus::Arrived
            } else {
                NavigationStatus::Moving
            }
        );
        let gpu = h
            .state()
            .viewport3d_gpu()
            .expect("production viewport")
            .gpu();
        assert_eq!(
            gpu.model_cache_counts().0,
            2,
            "shared-pass terrain plus terrain-free overlay"
        );
        let rgba = gpu.read_rgba8();
        let size = gpu.target().size();
        let cyan = rgba
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[0] < 90 && p[1] >= 160 && p[2] >= 175 && p[2] >= p[0].saturating_add(85))
            .count();
        let magenta = rgba
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[0] >= 150 && p[2] >= 120 && p[1] < 110)
            .count();
        let terrain_pixels = rgba
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| {
                p[0] > 30
                    && p[0] < 135
                    && p[1] > p[0].saturating_add(25)
                    && p[2] < p[1]
                    && p[2] < 150
            })
            .count();
        assert!(
            terrain_pixels > 100,
            "visible terrain at {label}: {terrain_pixels}"
        );
        assert!(cyan > 10, "visible path at {label}: {cyan}");
        assert!(magenta > 10, "visible agent at {label}: {magenta}");
        if !prior.is_empty() {
            assert_ne!(rgba, prior, "visible authoritative movement");
        }
        if let Some(directory) = std::env::var_os("NAVIGATION_PROJECT_CAPTURE_DIR") {
            let directory = std::path::PathBuf::from(directory);
            std::fs::create_dir_all(&directory).unwrap();
            orr_sample::navigation_app::save_png_new(
                &directory.join(format!("editor-{label}.png")),
                &rgba,
                size,
            )
            .unwrap();
            let metadata = serde_json::json!({"tick":tick,"frame_checksum":format!("0x{:016x}",frame.predicted().checksum()),"position_raw":decoded.navigator.position().map(orr_fp::FP::raw),"terrain_revision":decoded.terrain.revision(),"graph_revision":decoded.graph.revision(),"navigator_checksum":decoded.navigator.checksum(),"source_sha":std::env::var("NAVIGATION_SOURCE_SHA").ok(),"adapter":gpu.adapter_name()});
            std::fs::write(
                directory.join(format!("editor-{label}.json")),
                serde_json::to_vec_pretty(&metadata).unwrap(),
            )
            .unwrap();
        }
        prior = rgba;
    }
    h.state_mut().editor.seek(0);
    settle(&mut h);
    assert_eq!(
        h.state().editor.checksum(),
        project.scene().frame().checksum()
    );
    let start_pixels = h.state().viewport3d_gpu().unwrap().gpu().read_rgba8();
    // Resize comes through the actual viewport and retains simulation authority.
    h.set_size(egui::vec2(900.0, 1100.0));
    settle(&mut h);
    assert_eq!(
        h.state().editor.checksum(),
        project.scene().frame().checksum()
    );
    let portrait_pixels = h.state().viewport3d_gpu().unwrap().gpu().read_rgba8();
    assert_ne!(portrait_pixels, start_pixels);
    h.set_size(egui::vec2(1280.0, 900.0));
    settle(&mut h);
    assert_eq!(
        h.state().editor.checksum(),
        project.scene().frame().checksum()
    );
    assert_eq!(
        h.state().viewport3d_gpu().unwrap().gpu().read_rgba8(),
        start_pixels,
        "resize return restores view without changing Frame"
    );
}
