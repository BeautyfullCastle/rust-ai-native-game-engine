//! Actual EditorApp widgets and independently driven production renderer checks.
use super::*;

fn alignment_fixture() -> tempfile::TempDir {
    let root = fixture();
    let mut doc = Document::parse(&fs::read(root.path().join("ui.json")).unwrap()).unwrap();
    doc.nodes.retain(|node| node.screen != Screen::Title);
    let other_screens = std::mem::take(&mut doc.nodes);
    let label = |id: &str, parent: Option<&str>, anchor, offset, size| Node {
        id: id.into(),
        parent: parent.map(str::to_owned),
        kind: Kind::Label {
            text: format!("{id} · 플레이"),
            binding: None,
        },
        screen: Screen::Title,
        anchor,
        offset,
        size,
    };
    doc.nodes.extend([
        label("align_ref", None, [0, 0], [40, 40], [81, 31]),
        Node {
            kind: Kind::Button {
                text: "Aligned Play".into(),
                action: Action::Play,
            },
            ..label("align_button", None, [0, 0], [220, 110], [61, 33])
        },
        label("align_label", None, [333, 167], [10, 180], [33, 19]),
        Node {
            kind: Kind::Container,
            ..label("align_outer", None, [0, 0], [10, 270], [160, 48])
        },
        Node {
            kind: Kind::Container,
            ..label(
                "align_inner",
                Some("align_outer"),
                [0, 0],
                [10, 0],
                [141, 45],
            )
        },
        label("nested_ref", Some("align_inner"), [0, 0], [8, 2], [31, 19]),
        Node {
            kind: Kind::Button {
                text: "Nested quit".into(),
                action: Action::Quit,
            },
            ..label(
                "nested_button",
                Some("align_inner"),
                [333, 0],
                [80, 24],
                [61, 31],
            )
        },
        label(
            "nested_label",
            Some("align_inner"),
            [667, 333],
            [10, 6],
            [33, 17],
        ),
    ]);
    // Keep the inspector fixture target in the visible part of its popup.
    // AccessKit can expose clipped rows whose synthetic click cannot select.
    doc.nodes.extend(other_screens);
    fs::write(root.path().join("ui.json"), doc.to_bytes().unwrap()).unwrap();
    root
}

fn select_reference(h: &mut Harness<'_, EditorApp>, id: &str) {
    h.get_by_role_and_label(Role::ComboBox, "Alignment reference")
        .click();
    h.run_steps(3);
    h.get_by_role_and_label(Role::Button, id).click();
    h.run_steps(3);
}

fn toggle_targets(h: &mut Harness<'_, EditorApp>, ids: &[&str]) {
    click(h, "Alignment targets");
    for id in ids {
        h.get_by_role_and_label(Role::CheckBox, id).click();
        h.run_steps(3);
        // A multi-select popup must remain open while each sibling is checked.
        h.get_by_role_and_label(Role::CheckBox, id);
    }
    // Close using its own trigger, never Escape (which cancels authoring).
    click(h, "Alignment targets");
}

fn toggle_target(h: &mut Harness<'_, EditorApp>, id: &str) {
    toggle_targets(h, &[id]);
}

fn root_selection(h: &mut Harness<'_, EditorApp>) {
    // Reset explicitly: reopening retains the tool selection, so blindly
    // toggling old targets would deselect them.
    select_reference(h, "align_button");
    select_reference(h, "align_ref");
    toggle_targets(h, &["align_button", "align_label"]);
    if !h.state().collect_ui_preview.alignment.aligning {
        click(h, "Align siblings");
    }
}

#[track_caller]
fn no_alignment_draft(h: &Harness<'_, EditorApp>) {
    assert!(!h.state().collect_ui_preview.alignment.state.is_ready());
    if h.state().collect_ui_preview.state.is_open() {
        assert_eq!(
            h.state().collect_ui_preview.state.rendered_document(),
            h.state().collect_ui.as_ref().map(|p| p.document())
        );
    }
}

fn stage(h: &mut Harness<'_, EditorApp>, mode: &str) -> Document {
    click(h, mode);
    assert!(h.state().collect_ui_preview.alignment.state.is_ready());
    h.state()
        .collect_ui_preview
        .alignment
        .state
        .document()
        .unwrap()
        .clone()
}

// A local sibling edge equation is enough to measure rounding. This does not
// reproduce recursive authored layout; rendering parity uses production CollectUi.
fn assert_aligned(
    original: &Document,
    candidate: &Document,
    reference: &str,
    targets: &[&str],
    viewport: [u32; 2],
    axis: usize,
    factor: f64,
) {
    let reference = candidate.nodes.iter().find(|n| n.id == reference).unwrap();
    let extent = reference
        .parent
        .as_ref()
        .map_or(f64::from(viewport[axis]), |id| {
            f64::from(candidate.nodes.iter().find(|n| n.id == *id).unwrap().size[axis])
        });
    let edge = |n: &Node| {
        f64::from(n.anchor[axis]) * extent / 1000.0
            + f64::from(n.offset[axis])
            + f64::from(n.size[axis]) * factor
    };
    for (before, after) in original.nodes.iter().zip(&candidate.nodes) {
        let mut only_offset = before.clone();
        if targets.contains(&before.id.as_str()) {
            assert!(
                (edge(after) - edge(reference)).abs() <= 0.500_001,
                "{} alignment residual",
                before.id
            );
            only_offset.offset[axis] = after.offset[axis];
        }
        assert_eq!(
            &only_offset, after,
            "reference, other axis, hierarchy, text/actions/bindings and size stay fixed"
        );
    }
}

#[test]
fn actual_editor_alignment_widgets_have_explicit_reference_and_sibling_targets() {
    let root = alignment_fixture();
    let project = prepared(root.path());
    let initial = project.ui().unwrap().document.clone();
    let mut h = Harness::builder()
        .with_size([1800.0, 1200.0])
        .renderer(egui_kittest::LazyRenderer::Uninitialized {
            textures_delta: Default::default(),
            builder: None,
        })
        .build_eframe(|cc| app(project, cc));
    h.run_steps(4);
    let stamp = host_stamp(&h);
    click(&mut h, "Collect UI document");
    click(&mut h, "Preview authored screens");
    select_node(&mut h, "align_ref");
    h.get_by_label("Stable id: align_ref");
    assert!(h
        .get_by_label("Apply alignment")
        .accesskit_node()
        .is_disabled());
    root_selection(&mut h);
    assert!(h.state().collect_ui_preview.alignment.aligning);
    assert!(!h.state().collect_ui_preview.position.moving);
    assert!(!h.state().collect_ui_preview.resize.resizing);
    for mode in ["Left", "Center", "Right", "Top", "Middle", "Bottom"] {
        assert!(!h
            .get_by_role_and_label(Role::Button, mode)
            .accesskit_node()
            .is_disabled());
    }
    click(&mut h, "Move UI position");
    assert!(!h.state().collect_ui_preview.alignment.aligning);
    click(&mut h, "Align siblings");
    assert!(!h.state().collect_ui_preview.position.moving);
    click(&mut h, "Resize UI size");
    assert!(!h.state().collect_ui_preview.alignment.aligning);
    click(&mut h, "Align siblings");
    assert!(!h.state().collect_ui_preview.resize.resizing);
    assert_eq!(h.state().collect_ui.as_ref().unwrap().document(), &initial);
    assert_eq!(host_stamp(&h), stamp);
    h.get_by_label("Stable id: align_ref");
    edit_text(
        &mut h,
        "align_ref · 플레이",
        "Independent inspector selection",
    );
    let changed = h.state().collect_ui.as_ref().unwrap().document();
    assert!(
        matches!(&changed.nodes.iter().find(|n| n.id == "align_ref").unwrap().kind, Kind::Label {text, ..} if text == "Independent inspector selection")
    );
    assert_eq!(
        changed.nodes.iter().find(|n| n.id == "align_button"),
        initial.nodes.iter().find(|n| n.id == "align_button")
    );
}

#[test]
#[ignore = "mandatory explicit software Vulkan actual EditorApp alignment acceptance"]
fn actual_editor_alignment_gpu_stages_applies_cancels_and_reopens() {
    let evidence = PathBuf::from(
        std::env::var_os("ORR_COLLECT_UI_ALIGNMENT_EVIDENCE")
            .expect("retained evidence directory required"),
    );
    assert!(evidence.is_absolute() && evidence.is_dir());
    let root = alignment_fixture();
    let project = prepared(root.path());
    let font = project.ui().unwrap().font.clone();
    let initial = project.ui().unwrap().document.clone();
    let files: Vec<_> = ["ui.json", "scene.yaml", "orr.project.json"]
        .into_iter()
        .map(|name| (name, fs::read(root.path().join(name)).unwrap()))
        .collect();
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
    root_selection(&mut h);
    parity(&mut h, &font, &evidence, "alignment-original");
    let original_pixels = pixels(h.state().collect_ui_preview.gpu.as_ref().unwrap());
    for label in [
        "Align siblings",
        "Alignment reference",
        "Alignment targets",
        "Left",
        "Center",
        "Right",
        "Top",
        "Middle",
        "Bottom",
        "Apply alignment",
        "Cancel alignment",
    ] {
        assert!(
            h.ctx
                .content_rect()
                .contains_rect(h.get_by_label(label).rect()),
            "visible alignment control {label}"
        );
    }
    let aligned = stage(&mut h, "Left");
    assert_aligned(
        &initial,
        &aligned,
        "align_ref",
        &["align_button", "align_label"],
        LANDSCAPE,
        0,
        0.0,
    );
    assert_eq!(
        h.state().collect_ui_preview.alignment.state.reference_id(),
        Some("align_ref")
    );
    assert_eq!(
        h.state()
            .collect_ui_preview
            .alignment
            .state
            .selected_ids()
            .unwrap()
            .len(),
        3
    );
    parity_document(&mut h, &font, &evidence, "alignment-staged-root", &aligned);
    assert_ne!(
        pixels(h.state().collect_ui_preview.gpu.as_ref().unwrap()),
        original_pixels
    );
    assert_eq!(h.state().collect_ui.as_ref().unwrap().document(), &initial);
    assert_eq!(host_stamp(&h), stamp);
    for (name, bytes) in &files {
        assert_eq!(&fs::read(root.path().join(name)).unwrap(), bytes);
    }
    assert!(h
        .get_by_label("Undo UI edit")
        .accesskit_node()
        .is_disabled());
    select_move_node(&mut h, "align_button");
    select_resize_node(&mut h, "align_button");
    assert_eq!(
        h.state().collect_ui_preview.state.rendered_document(),
        Some(&aligned),
        "inactive selectors preserve alignment"
    );
    click(&mut h, "Cancel alignment");
    no_alignment_draft(&h);
    parity(&mut h, &font, &evidence, "alignment-cancelled-root");
    assert_eq!(
        pixels(h.state().collect_ui_preview.gpu.as_ref().unwrap()),
        original_pixels
    );

    for interruption in 0..10 {
        stage(&mut h, "Left");
        match interruption {
            0 => {
                h.key_press(egui::Key::Escape);
                h.run_steps(3);
            }
            1 => {
                h.event(egui::Event::WindowFocused(false));
                h.run_steps(3);
                no_alignment_draft(&h);
                h.event(egui::Event::WindowFocused(true));
                h.run_steps(3);
            }
            2 => {
                h.input_mut().focused = false;
                h.run_steps(3);
                no_alignment_draft(&h);
                h.input_mut().focused = true;
                h.run_steps(3);
            }
            3 => {
                click(&mut h, "Menu");
                click(&mut h, "Title");
            }
            4 => {
                click(&mut h, "75%");
                click(&mut h, "100%");
            }
            5 => {
                click(&mut h, "Portrait 360 × 640");
                click(&mut h, "Landscape 640 × 360");
            }
            6 => {
                click(&mut h, "Move UI position");
                no_alignment_draft(&h);
                click(&mut h, "Align siblings");
            }
            7 => {
                toggle_target(&mut h, "align_label");
                no_alignment_draft(&h);
                toggle_target(&mut h, "align_label");
            }
            8 => {
                select_reference(&mut h, "align_button");
                no_alignment_draft(&h);
                root_selection(&mut h);
            }
            _ => {
                click(&mut h, "Close screen preview");
                click(&mut h, "Preview authored screens");
                if !h.state().collect_ui_preview.alignment.aligning {
                    click(&mut h, "Align siblings");
                }
            }
        }
        no_alignment_draft(&h);
        assert_eq!(h.state().collect_ui.as_ref().unwrap().document(), &initial);
        assert!(h
            .get_by_label("Undo UI edit")
            .accesskit_node()
            .is_disabled());
    }
    // An unrelated accepted edit invalidates the entire pinned snapshot.
    stage(&mut h, "Left");
    let mut changed = initial.clone();
    changed
        .nodes
        .iter_mut()
        .find(|node| node.id == "score")
        .unwrap()
        .size[0] += 1;
    h.state_mut()
        .collect_ui
        .as_mut()
        .unwrap()
        .apply_document(changed)
        .unwrap();
    h.run_steps(3);
    no_alignment_draft(&h);
    click(&mut h, "Undo UI edit");

    // Selecting alignment inputs while another tool owns a released draft must
    // not discard that draft or alter mutually exclusive mode flags.
    click(&mut h, "Move UI position");
    position_drag(&mut h, [20.0, 20.0]);
    let position_candidate = h
        .state()
        .collect_ui_preview
        .state
        .rendered_document()
        .unwrap()
        .clone();
    select_reference(&mut h, "align_label");
    toggle_target(&mut h, "align_button");
    assert_eq!(
        h.state().collect_ui_preview.state.rendered_document(),
        Some(&position_candidate)
    );
    assert!(h.state().collect_ui_preview.position.state.is_ready());
    click(&mut h, "Cancel position");
    click(&mut h, "Resize UI size");
    resize_drag(&mut h, [20.0, 20.0]);
    let resize_candidate = h
        .state()
        .collect_ui_preview
        .state
        .rendered_document()
        .unwrap()
        .clone();
    select_reference(&mut h, "align_ref");
    toggle_targets(&mut h, &["align_button", "align_label"]);
    assert_eq!(
        h.state().collect_ui_preview.state.rendered_document(),
        Some(&resize_candidate)
    );
    assert!(h.state().collect_ui_preview.resize.state.is_ready());
    click(&mut h, "Cancel size");
    click(&mut h, "Align siblings");

    stage(&mut h, "Left");
    click(&mut h, "Apply alignment");
    no_alignment_draft(&h);
    assert_eq!(h.state().collect_ui.as_ref().unwrap().document(), &aligned);
    parity(&mut h, &font, &evidence, "alignment-applied-root");
    click(&mut h, "Align siblings");
    canvas_click(&mut h, [250.0, 125.0]);
    assert_eq!(
        h.state().collect_ui_preview.state.last_action(),
        None,
        "old button location misses"
    );
    canvas_click(&mut h, [65.0, 125.0]);
    assert_eq!(
        h.state().collect_ui_preview.state.last_action(),
        Some(Action::Play),
        "new aligned button hits"
    );
    assert_eq!(host_stamp(&h), stamp);
    click(&mut h, "Undo UI edit");
    assert_eq!(h.state().collect_ui.as_ref().unwrap().document(), &initial);
    assert!(
        h.get_by_label("Undo UI edit")
            .accesskit_node()
            .is_disabled(),
        "one Apply is one undo"
    );

    click(&mut h, "Portrait 360 × 640");
    click(&mut h, "75%");
    select_reference(&mut h, "nested_ref");
    toggle_targets(&mut h, &["nested_button", "nested_label"]);
    click(&mut h, "Align siblings");
    for (mode, axis, factor) in [
        ("Left", 0, 0.0),
        ("Center", 0, 0.5),
        ("Right", 0, 1.0),
        ("Top", 1, 0.0),
        ("Middle", 1, 0.5),
        ("Bottom", 1, 1.0),
    ] {
        let candidate = stage(&mut h, mode);
        assert_aligned(
            &initial,
            &candidate,
            "nested_ref",
            &["nested_button", "nested_label"],
            PORTRAIT,
            axis,
            factor,
        );
        parity_document(
            &mut h,
            &font,
            &evidence,
            &format!("alignment-nested-{}", mode.to_lowercase()),
            &candidate,
        );
        click(&mut h, "Cancel alignment");
    }
    let nested = stage(&mut h, "Left");
    click(&mut h, "Apply alignment");
    click(&mut h, "Align siblings");
    canvas_click(&mut h, [45.0 * 0.75, 322.0 * 0.75]);
    assert_eq!(
        h.state().collect_ui_preview.state.last_action(),
        None,
        "outside nested parent clip misses"
    );
    canvas_click(&mut h, [45.0 * 0.75, 303.0 * 0.75]);
    assert_eq!(
        h.state().collect_ui_preview.state.last_action(),
        Some(Action::Quit),
        "aligned nested button within both ancestor clips hits"
    );
    assert_eq!(host_stamp(&h), stamp);
    click(&mut h, "Save UI document");
    let saved = prepared(root.path());
    assert_eq!(saved.ui().unwrap().document, nested);
    assert_eq!(saved.ui().unwrap().font, font);
    for (name, bytes) in &files[1..] {
        assert_eq!(&fs::read(root.path().join(name)).unwrap(), bytes);
    }
    click(&mut h, "Close screen preview");
    let context = h.ctx.clone();
    crate::project::install_collect_presentation(h.state_mut(), saved, &context);
    h.run_steps(4);
    click(&mut h, "Preview authored screens");
    parity(&mut h, &font, &evidence, "alignment-saved-reopened");
    root_selection(&mut h);
    stage(&mut h, "Left");
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
    no_alignment_draft(&h);
    assert!(!h.state().collect_ui_preview.state.is_available());
    let context = h.ctx.clone();
    crate::project::install_collect_presentation(h.state_mut(), replacement, &context);
    h.run_steps(4);
    click(&mut h, "Preview authored screens");
    root_selection(&mut h);
    stage(&mut h, "Left");
    assert!(h.state_mut().editor.start_play());
    h.run_steps(3);
    no_alignment_draft(&h);
    assert!(!h.state().collect_ui_preview.alignment.aligning);
    assert!(h
        .get_by_label("Align siblings")
        .accesskit_node()
        .is_disabled());
    assert_eq!(h.state().collect_ui.as_ref().unwrap().document(), &nested);
    h.state_mut().collect_ui = None;
    h.run_steps(3);
    no_alignment_draft(&h);
    fs::write(
        evidence.join("collect-ui-alignment-acceptance.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema":1, "production_editor_app":true, "software_vulkan":true, "adapter":adapter,
            "window_pixels":[1800,1200], "reference_fixed_multiple_checkbox_targets":true,
            "staging_changes_pixels_only":true, "cancel_restores_exact_pixels":true,
            "apply_one_undo":true, "old_and_new_button_hit_regions":true,
            "nested_ancestor_clip":true, "portrait_display_scale":0.75,
            "fractional_anchors_odd_sizes_six_modes_half_pixel_residual":true,
            "escape_focus_snapshot_focus_event_context_selection_mode_epoch_play_cancel":true,
            "inactive_alignment_selection_preserves_other_mode_drafts":true,
            "host_and_project_files_unchanged_before_apply":true, "save_reopen":true,
            "production_collect_ui_pixel_parity_cases":11, "native_os_window":false
        }))
        .unwrap(),
    )
    .unwrap();
}
