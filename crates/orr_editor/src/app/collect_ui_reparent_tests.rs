//! One explicit GPU-backed EditorApp scenario, not a data-only authoring test.
use super::*;

fn select_reparent(h: &mut Harness<'_, EditorApp>, label: &str, id: &str) {
    h.get_by_role_and_label(Role::ComboBox, label).click();
    h.run_steps(3);
    h.get_by_role_and_label(Role::Button, id).click();
    h.run_steps(3);
}
fn stage(h: &mut Harness<'_, EditorApp>) {
    click(h, "Stage reparent");
    assert!(h.state().collect_ui_preview.reparent.is_ready());
    assert!(
        !h.get_by_label("Apply reparent")
            .accesskit_node()
            .is_disabled()
    );
}
fn no_draft(h: &Harness<'_, EditorApp>) {
    assert!(!h.state().collect_ui_preview.reparent.is_ready());
    if h.state().collect_ui_preview.state.is_open() {
        assert_eq!(
            h.state().collect_ui_preview.state.rendered_document(),
            h.state().collect_ui.as_ref().map(|p| p.document())
        );
    } else {
        assert!(
            h.state()
                .collect_ui_preview
                .state
                .rendered_document()
                .is_none()
        );
    }
}

#[test]
#[ignore = "mandatory explicit software Vulkan actual EditorApp reparent acceptance"]
fn actual_editor_reparent_stage_cancel_apply_undo_save_reopen_play() {
    let evidence = PathBuf::from(
        std::env::var_os("ORR_COLLECT_UI_REPARENT_EVIDENCE")
            .expect("retained evidence directory required"),
    );
    assert!(evidence.is_absolute() && evidence.is_dir());
    let root = fixture();
    let mut initial = prepared(root.path()).ui().unwrap().document.clone();
    let index = initial
        .nodes
        .iter()
        .position(|node| node.id == "clip_panel")
        .unwrap();
    initial.nodes.insert(
        index,
        Node {
            id: "new_parent".into(),
            parent: None,
            kind: Kind::Container,
            screen: Screen::Title,
            anchor: [0, 0],
            offset: [0, 0],
            size: [120, 320],
        },
    );
    fs::write(root.path().join("ui.json"), initial.to_bytes().unwrap()).unwrap();
    let project = prepared(root.path());
    let font = project.ui().unwrap().font.clone();
    let original_file = fs::read(root.path().join("ui.json")).unwrap();
    let scene = fs::read(root.path().join("scene.yaml")).unwrap();
    let manifest = fs::read(root.path().join("orr.project.json")).unwrap();
    let crate::collect_ui_reparent::Proposal::Changed(candidate) =
        crate::collect_ui_reparent::propose(&initial, "clip_panel", Some("new_parent"), LANDSCAPE)
            .unwrap()
    else {
        panic!("fixture must change")
    };
    let mut setup = egui_wgpu::WgpuSetupCreateNew::without_display_handle();
    setup.instance_descriptor.backends = wgpu::Backends::VULKAN;
    setup.native_adapter_selector = Some(Arc::new(|adapters, _| {
        adapters
            .iter()
            .find(|adapter| {
                let info = adapter.get_info();
                info.backend == wgpu::Backend::Vulkan && info.device_type == wgpu::DeviceType::Cpu
            })
            .cloned()
            .ok_or_else(|| "software Vulkan adapter required".into())
    }));
    let mut h = Harness::builder()
        .with_size([1800.0, 1200.0])
        .wgpu_setup(setup.into())
        .build_eframe(|cc| app(project, cc));
    h.run_steps(5);
    let stamp = host_stamp(&h);
    let adapter = format!(
        "{:?}",
        h.state().render_state.as_ref().unwrap().adapter.get_info()
    );
    click(&mut h, "Collect UI document");
    click(&mut h, "Preview authored screens");
    select_reparent(&mut h, "Reparent source", "clip_panel");
    select_reparent(&mut h, "Reparent target", "new_parent");
    click(&mut h, "Reparent UI subtree");
    parity(&mut h, &font, &evidence, "reparent-initial");
    let before_pixels = pixels(h.state().collect_ui_preview.gpu.as_ref().unwrap());
    stage(&mut h);
    assert_eq!(h.state().collect_ui.as_ref().unwrap().document(), &initial);
    assert_eq!(
        fs::read(root.path().join("ui.json")).unwrap(),
        original_file
    );
    assert!(
        h.get_by_label("Undo UI edit")
            .accesskit_node()
            .is_disabled()
    );
    parity_document(
        &mut h,
        &font,
        &evidence,
        "reparent-staged-clipping",
        &candidate,
    );
    assert_ne!(
        pixels(h.state().collect_ui_preview.gpu.as_ref().unwrap()),
        before_pixels,
        "new parent clips the same positioned subtree"
    );
    #[cfg(all(
        target_os = "linux",
        target_arch = "x86_64",
        feature = "project-create"
    ))]
    for _ in 0..2 {
        assert!(h.state().collect_ui_project_switch_guard().is_err());
        assert!(h.state().collect_ui_preview.reparent.is_ready());
        assert_eq!(
            h.state().collect_ui_preview.state.rendered_document(),
            Some(&candidate)
        );
        assert_eq!(h.state().collect_ui.as_ref().unwrap().document(), &initial);
        assert_eq!(
            fs::read(root.path().join("ui.json")).unwrap(),
            original_file
        );
        assert!(
            h.get_by_label("Undo UI edit")
                .accesskit_node()
                .is_disabled()
        );
    }
    click(&mut h, "Cancel reparent");
    no_draft(&h);
    parity(&mut h, &font, &evidence, "reparent-cancelled");
    assert_eq!(
        pixels(h.state().collect_ui_preview.gpu.as_ref().unwrap()),
        before_pixels
    );

    // Source and target changes invalidate before Apply. A same-parent Stage
    // restores accepted rendering and creates neither draft nor Undo entry.
    stage(&mut h);
    select_reparent(&mut h, "Reparent source", "clip_label");
    no_draft(&h);
    assert!(
        h.get_by_label("Apply reparent")
            .accesskit_node()
            .is_disabled()
    );
    select_reparent(&mut h, "Reparent source", "clip_panel");
    stage(&mut h);
    select_reparent(&mut h, "Reparent target", "Viewport");
    no_draft(&h);
    click(&mut h, "Stage reparent");
    no_draft(&h);
    assert!(
        h.get_by_label("Undo UI edit")
            .accesskit_node()
            .is_disabled()
    );
    select_reparent(&mut h, "Reparent target", "new_parent");

    // Lifecycle invalidation exercises actual app frames, not Controls-only calls.
    stage(&mut h);
    h.input_mut().focused = true;
    h.event(egui::Event::WindowFocused(false));
    h.run_steps(3);
    no_draft(&h);
    h.event(egui::Event::WindowFocused(true));
    h.run_steps(3);
    stage(&mut h);
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    no_draft(&h);
    stage(&mut h);
    click(&mut h, "Portrait 360 × 640");
    no_draft(&h);
    click(&mut h, "Landscape 640 × 360");
    stage(&mut h);
    click(&mut h, "Playing");
    no_draft(&h);
    click(&mut h, "Title");
    stage(&mut h);
    click(&mut h, "Move UI position");
    no_draft(&h);
    assert!(!h.state().collect_ui_preview.reparent.reparenting);
    click(&mut h, "Reparent UI subtree");
    assert!(!h.state().collect_ui_preview.position.moving);
    stage(&mut h);
    click(&mut h, "Close screen preview");
    no_draft(&h);
    click(&mut h, "Preview authored screens");
    stage(&mut h);
    click(&mut h, "Close window");
    no_draft(&h);
    click(&mut h, "Preview authored screens");
    stage(&mut h);
    let mut changed = initial.clone();
    changed
        .nodes
        .iter_mut()
        .find(|node| node.id == "new_parent")
        .unwrap()
        .offset[0] = 1;
    h.state_mut()
        .collect_ui
        .as_mut()
        .unwrap()
        .apply_document(changed)
        .unwrap();
    h.run_steps(3);
    no_draft(&h);
    click(&mut h, "Undo UI edit");
    assert_eq!(h.state().collect_ui.as_ref().unwrap().document(), &initial);

    stage(&mut h);
    click(&mut h, "Apply reparent");
    no_draft(&h);
    assert_eq!(
        h.state().collect_ui.as_ref().unwrap().document(),
        &candidate
    );
    assert_eq!(
        fs::read(root.path().join("ui.json")).unwrap(),
        original_file
    );
    click(&mut h, "Undo UI edit");
    assert_eq!(h.state().collect_ui.as_ref().unwrap().document(), &initial);
    assert!(
        h.get_by_label("Undo UI edit")
            .accesskit_node()
            .is_disabled(),
        "one accepted reparent is exactly one Undo"
    );
    stage(&mut h);
    click(&mut h, "Apply reparent");
    click(&mut h, "Stage reparent");
    no_draft(&h); // same parent does not add a second history entry
    click(&mut h, "Save UI document");
    let saved = prepared(root.path());
    assert_eq!(saved.ui().unwrap().document, candidate);
    assert_eq!(fs::read(root.path().join("scene.yaml")).unwrap(), scene);
    assert_eq!(
        fs::read(root.path().join("orr.project.json")).unwrap(),
        manifest
    );
    assert_eq!(host_stamp(&h), stamp);
    click(&mut h, "Close screen preview");
    let context = h.ctx.clone();
    crate::project::install_collect_presentation(h.state_mut(), saved, &context);
    h.run_steps(4);
    click(&mut h, "Preview authored screens");
    parity(&mut h, &font, &evidence, "reparent-saved-reopened");

    // Fresh production runtime consumes saved nodes, including unchanged actions.
    let saved = prepared(root.path());
    let ui = saved.ui().unwrap();
    let mut runtime = CollectUi::new(ui.document.clone(), ui.font.clone()).unwrap();
    assert_eq!(runtime.document(), &candidate);
    let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(640.0, 360.0));
    let play = orr_sample::collect_ui::layout_document(runtime.document(), Screen::Title, viewport)
        .unwrap()
        .into_iter()
        .find(|entry| runtime.document().nodes[entry.index()].id == "play")
        .unwrap()
        .rect()
        .center();
    let raw = |events| egui::RawInput {
        screen_rect: Some(viewport),
        events,
        ..Default::default()
    };
    // These runtime frames probe saved actions, not rendering. Preview pixels
    // are checked above; explicitly discard this separate runtime's atlas deltas.
    let (mut output, _) = runtime.show(raw(vec![]), Hud::default());
    output.textures_delta.clear();
    let (mut output, _) = runtime.show(
        raw(vec![
            egui::Event::PointerMoved(play),
            egui::Event::PointerButton {
                pos: play,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ]),
        Hud::default(),
    );
    output.textures_delta.clear();
    let (mut output, action) = runtime.show(
        raw(vec![egui::Event::PointerButton {
            pos: play,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }]),
        Hud::default(),
    );
    output.textures_delta.clear();
    assert_eq!(
        action,
        Some(Action::Play),
        "saved authored Play action must survive"
    );
    runtime.apply(action.unwrap());
    assert_eq!(runtime.screen(), Screen::Playing);
    let (mut output, action) = runtime.show(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(640.0, 360.0),
            )),
            ..Default::default()
        },
        Hud {
            score: 2,
            phase: 0,
            best: "2".into(),
        },
    );
    output.textures_delta.clear();
    assert!(!output.shapes.is_empty());
    assert_eq!(action, None);

    select_reparent(&mut h, "Reparent source", "clip_panel");
    // Reinstallation keeps controls but never keeps a staged edit.
    if !h.state().collect_ui_preview.reparent.reparenting {
        click(&mut h, "Reparent UI subtree");
    }
    select_reparent(&mut h, "Reparent target", "Viewport");
    stage(&mut h);
    // A fresh host at the same path must not inherit a stale staged transaction.
    let replacement = prepared(root.path());
    let mut editor = crate::Editor::start(&crate::HostSpec::PreparedCollect {
        scene: replacement.path().to_owned(),
        text: replacement.scene().text().into(),
        listen: None,
        debug_hooks: false,
    })
    .unwrap();
    editor.sync();
    h.state_mut().editor = editor;
    h.run_steps(3);
    no_draft(&h);
    assert!(!h.state().collect_ui_preview.state.is_available());
    let context = h.ctx.clone();
    crate::project::install_collect_presentation(h.state_mut(), replacement, &context);
    h.run_steps(4);
    click(&mut h, "Preview authored screens");
    stage(&mut h);
    assert!(h.state_mut().editor.start_play());
    h.run_steps(3);
    no_draft(&h);
    assert!(!h.state().collect_ui_preview.reparent.reparenting);
    assert_eq!(
        h.state().collect_ui.as_ref().unwrap().document(),
        &candidate
    );
    assert!(
        h.get_by_label("Stage reparent")
            .accesskit_node()
            .is_disabled()
    );
    h.state_mut().collect_ui = None;
    h.run_steps(3);
    no_draft(&h);
    assert!(!h.state().collect_ui_preview.state.is_available());
    fs::write(
        evidence.join("reparent-acceptance.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema":1, "actual_editor_app":true, "production_preview_pixel_parity_cases":4,
            "native_os_window":false, "software_vulkan":true, "adapter":adapter, "window_pixels":[1800,1200],
            "stage_cancel_apply_undo_save_reopen":true, "saved_production_collect_ui_play":true,
            "same_parent_no_history":true, "new_parent_clipping_visible":true,
            "selection_document_focus_escape_viewport_screen_mode_close_title_x_epoch_play_panel_cancel":true,
            "saved_authored_play_button_click":true,
            "redo":"not integrated; PR182 prerequisite", "separate_runtime_process":false
        }))
        .unwrap(),
    )
    .unwrap();
}
