//! Actual EditorApp widgets and independently driven production renderer checks.
use super::*;

fn distribution_fixture() -> tempfile::TempDir {
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
        label("gap_first", None, [0, 0], [40, 20], [31, 21]),
        label("gap_ref", None, [0, 0], [100, 60], [21, 33]),
        Node {
            kind: Kind::Button {
                text: "Distributed Play".into(),
                action: Action::Play,
            },
            ..label("gap_button", None, [0, 0], [280, 180], [61, 41])
        },
        label("gap_last", None, [0, 0], [400, 280], [41, 31]),
        Node {
            kind: Kind::Container,
            ..label("gap_outer", None, [0, 0], [10, 270], [300, 180])
        },
        Node {
            kind: Kind::Container,
            ..label("gap_inner", Some("gap_outer"), [0, 0], [10, 0], [281, 301])
        },
        label("nested_first", Some("gap_inner"), [0, 0], [8, 2], [31, 19]),
        label(
            "nested_ref",
            Some("gap_inner"),
            [333, 167],
            [0, 0],
            [21, 23],
        ),
        Node {
            kind: Kind::Button {
                text: "Distributed Quit".into(),
                action: Action::Quit,
            },
            ..label(
                "nested_button",
                Some("gap_inner"),
                [667, 333],
                [-20, 20],
                [41, 31],
            )
        },
        label(
            "nested_last",
            Some("gap_inner"),
            [0, 0],
            [220, 250],
            [31, 19],
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
    select_reference(h, "gap_first");
    select_reference(h, "gap_ref");
    toggle_targets(h, &["gap_first", "gap_button", "gap_last"]);
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

// Independent local edge measurement, not a second production layout engine.
fn assert_distributed(
    before: &Document,
    after: &Document,
    ids: &[&str],
    viewport: [u32; 2],
    axis: usize,
) {
    let nodes: Vec<_> = ids
        .iter()
        .map(|id| before.nodes.iter().find(|n| n.id == *id).unwrap())
        .collect();
    let extent = nodes[0]
        .parent
        .as_ref()
        .map_or(f64::from(viewport[axis]), |id| {
            f64::from(before.nodes.iter().find(|n| n.id == *id).unwrap().size[axis])
        });
    let edge = |n: &Node| f64::from(n.anchor[axis]) * extent / 1000.0 + f64::from(n.offset[axis]);
    let mut nodes = nodes;
    nodes.sort_by(|a, b| edge(a).partial_cmp(&edge(b)).unwrap().then(a.id.cmp(&b.id)));
    let first = nodes[0];
    let last = nodes[nodes.len() - 1];
    let gap = (edge(last)
        - edge(first)
        - nodes[..nodes.len() - 1]
            .iter()
            .map(|n| f64::from(n.size[axis]))
            .sum::<f64>())
        / (nodes.len() - 1) as f64;
    assert!(gap >= 0.0);
    let mut desired = edge(first);
    let mut previous_end = None;
    for node in &nodes {
        let actual = after.nodes.iter().find(|n| n.id == node.id).unwrap();
        assert!(
            (edge(actual) - desired).abs() <= 0.500_001,
            "{} final edge rounding",
            node.id
        );
        if let Some(end) = previous_end {
            assert!(edge(actual) >= end, "no quantized overlap");
        }
        previous_end = Some(edge(actual) + f64::from(actual.size[axis]));
        desired += f64::from(node.size[axis]) + gap;
    }
    for (old, new) in before.nodes.iter().zip(&after.nodes) {
        let mut allowed = old.clone();
        if ids.contains(&old.id.as_str()) && old.id != first.id && old.id != last.id {
            allowed.offset[axis] = new.offset[axis];
        }
        assert_eq!(&allowed, new, "only interior axis offsets may change");
    }
}

#[test]
fn actual_editor_distribution_requires_three_siblings_and_keeps_multi_select_open() {
    let root = distribution_fixture();
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
    select_reference(&mut h, "gap_ref");
    click(&mut h, "Align siblings");
    for label in ["Horizontal equal gaps", "Vertical equal gaps"] {
        assert!(h.get_by_label(label).accesskit_node().is_disabled());
    }
    toggle_target(&mut h, "gap_button");
    assert!(h
        .get_by_label("Horizontal equal gaps")
        .accesskit_node()
        .is_disabled());
    toggle_targets(&mut h, &["gap_first", "gap_last"]);
    for label in ["Horizontal equal gaps", "Vertical equal gaps"] {
        assert!(!h.get_by_label(label).accesskit_node().is_disabled());
        assert!(h
            .ctx
            .content_rect()
            .contains_rect(h.get_by_label(label).rect()));
    }
    assert_eq!(h.state().collect_ui.as_ref().unwrap().document(), &initial);
    assert_eq!(host_stamp(&h), stamp);
}

#[test]
#[ignore = "mandatory explicit software Vulkan actual EditorApp distribution acceptance"]
fn actual_editor_distribution_gpu_stages_cancels_applies_and_fresh_reopens() {
    let evidence = PathBuf::from(
        std::env::var_os("ORR_COLLECT_UI_DISTRIBUTION_EVIDENCE")
            .expect("retained evidence directory required"),
    );
    assert!(evidence.is_absolute() && evidence.is_dir());
    let root = distribution_fixture();
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
                let i = adapter.get_info();
                i.backend == wgpu::Backend::Vulkan && i.device_type == wgpu::DeviceType::Cpu
            })
            .cloned()
            .ok_or_else(|| "software Vulkan adapter required".into())
    }));
    let mut h = Harness::builder()
        .with_size([1800.0, 1200.0])
        .wgpu_setup(setup.clone().into())
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
    parity(&mut h, &font, &evidence, "distribution-original");
    let original_pixels = pixels(h.state().collect_ui_preview.gpu.as_ref().unwrap());
    let ids = ["gap_first", "gap_ref", "gap_button", "gap_last"];
    for (label, axis) in [("Horizontal equal gaps", 0), ("Vertical equal gaps", 1)] {
        let candidate = stage(&mut h, label);
        assert_distributed(&initial, &candidate, &ids, LANDSCAPE, axis);
        assert_ne!(
            candidate.nodes.iter().find(|n| n.id == "gap_ref"),
            initial.nodes.iter().find(|n| n.id == "gap_ref"),
            "interior reference moves"
        );
        parity_document(
            &mut h,
            &font,
            &evidence,
            &format!("distribution-root-{axis}"),
            &candidate,
        );
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
        click(&mut h, "Cancel alignment");
        no_alignment_draft(&h);
        assert_eq!(
            pixels(h.state().collect_ui_preview.gpu.as_ref().unwrap()),
            original_pixels
        );
    }
    parity(&mut h, &font, &evidence, "distribution-cancelled");
    let horizontal = stage(&mut h, "Horizontal equal gaps");
    click(&mut h, "Apply alignment");
    no_alignment_draft(&h);
    assert_eq!(
        h.state().collect_ui.as_ref().unwrap().document(),
        &horizontal
    );
    parity(&mut h, &font, &evidence, "distribution-applied");
    click(&mut h, "Align siblings");
    canvas_click(&mut h, [330.0, 195.0]);
    assert_eq!(h.state().collect_ui_preview.state.last_action(), None);
    canvas_click(&mut h, [265.0, 195.0]);
    assert_eq!(
        h.state().collect_ui_preview.state.last_action(),
        Some(Action::Play)
    );
    assert_eq!(host_stamp(&h), stamp);
    click(&mut h, "Undo UI edit");
    assert_eq!(h.state().collect_ui.as_ref().unwrap().document(), &initial);
    assert!(h
        .get_by_label("Undo UI edit")
        .accesskit_node()
        .is_disabled());
    click(&mut h, "Portrait 360 × 640");
    click(&mut h, "75%");
    select_reference(&mut h, "nested_ref");
    toggle_targets(&mut h, &["nested_first", "nested_button", "nested_last"]);
    click(&mut h, "Align siblings");
    let ids = ["nested_first", "nested_ref", "nested_button", "nested_last"];
    for (label, axis) in [("Horizontal equal gaps", 0), ("Vertical equal gaps", 1)] {
        let candidate = stage(&mut h, label);
        assert_distributed(&initial, &candidate, &ids, PORTRAIT, axis);
        parity_document(
            &mut h,
            &font,
            &evidence,
            &format!("distribution-nested-{axis}"),
            &candidate,
        );
        click(&mut h, "Cancel alignment");
    }
    let nested = stage(&mut h, "Vertical equal gaps");
    click(&mut h, "Apply alignment");
    click(&mut h, "Align siblings");
    canvas_click(&mut h, [200.0 * 0.75, 455.0 * 0.75]);
    assert_eq!(
        h.state().collect_ui_preview.state.last_action(),
        None,
        "ancestor clip rejects outside"
    );
    canvas_click(&mut h, [200.0 * 0.75, 445.0 * 0.75]);
    assert_eq!(
        h.state().collect_ui_preview.state.last_action(),
        Some(Action::Quit),
        "distributed nested button hits inside clip"
    );
    assert_eq!(host_stamp(&h), stamp);
    click(&mut h, "Save UI document");
    let saved = prepared(root.path());
    assert_eq!(saved.ui().unwrap().document, nested);
    assert_eq!(saved.ui().unwrap().font, font);
    for (name, bytes) in &files[1..] {
        assert_eq!(&fs::read(root.path().join(name)).unwrap(), bytes);
    }
    drop(h);
    let mut reopened = Harness::builder()
        .with_size([1800.0, 1200.0])
        .wgpu_setup(setup.into())
        .build_eframe(|cc| app(saved, cc));
    reopened.run_steps(5);
    click(&mut reopened, "Collect UI document");
    click(&mut reopened, "Preview authored screens");
    click(&mut reopened, "Portrait 360 × 640");
    click(&mut reopened, "75%");
    assert_eq!(
        reopened.state().collect_ui.as_ref().unwrap().document(),
        &nested
    );
    assert!(reopened
        .get_by_label("Undo UI edit")
        .accesskit_node()
        .is_disabled());
    parity(
        &mut reopened,
        &font,
        &evidence,
        "distribution-fresh-reopened",
    );
    fs::write(evidence.join("collect-ui-distribution-acceptance.json"),serde_json::to_vec_pretty(&serde_json::json!({"schema":1,"production_editor_app":true,"software_vulkan":true,"adapter":adapter,"window_pixels":[1800,1200],"three_siblings_required_multi_checkbox":true,"endpoints_fixed_interior_reference_moves":true,"horizontal_vertical_equal_edge_gaps":true,"staging_changes_pixels_only":true,"cancel_restores_exact_pixels":true,"apply_one_undo":true,"old_and_new_button_hit_regions":true,"nested_ancestor_clip":true,"portrait_display_scale":0.75,"host_and_project_files_unchanged_before_apply":true,"save_fresh_editor_reopen":true,"production_collect_ui_pixel_parity_cases":8,"native_os_window":false})).unwrap()).unwrap();
}
