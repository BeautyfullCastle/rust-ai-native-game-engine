use super::*;
use egui::accesskit::Role;
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use orr_sample::{
    authored_ui::{Action, Document, Kind, Node},
    collect_project::{PreparedProject, ProgressSupport, SpriteSupport},
    collect_ui::{CollectUi, Hud},
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir_in(fs::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
    fs::write(
        dir.path().join("scene.yaml"),
        include_bytes!("../../../../scenes/collect_dodge_v1.scene.yaml"),
    )
    .unwrap();
    let mut document = Document::default_collect();
    document.nodes.extend([
        Node {
            id: "clip_panel".into(),
            parent: None,
            kind: Kind::Container,
            screen: Screen::Title,
            anchor: [0, 0],
            offset: [10, 270],
            size: [160, 48],
        },
        Node {
            id: "clip_label".into(),
            parent: Some("clip_panel".into()),
            kind: Kind::Label {
                text: "플레이 · Clipped label beyond parent".into(),
                binding: None,
            },
            screen: Screen::Title,
            anchor: [0, 0],
            offset: [8, 2],
            size: [300, 18],
        },
        Node {
            id: "clip_button".into(),
            parent: Some("clip_panel".into()),
            kind: Kind::Button {
                text: "Clip button".into(),
                action: Action::Quit,
            },
            screen: Screen::Title,
            anchor: [0, 0],
            offset: [120, 24],
            size: [100, 30],
        },
    ]);
    fs::write(dir.path().join("ui.json"), document.to_bytes().unwrap()).unwrap();
    fs::write(dir.path().join("orr.project.json"),serde_json::to_vec(&serde_json::json!({"schema":2,"engine":"*","entry":{"game":"collect-dodge-v1","scene":"scene.yaml","ui":{"profile":"collect-authored-v1","document":"ui.json","font":{"package":"korean-game-ui","asset":"OrreryKoreanUI.otf"}}}})).unwrap()).unwrap();
    orr_package::Project::open(dir.path(), orr_package::Runtime::content_only())
        .unwrap()
        .install(&[PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/game_ui_font")
            .canonicalize()
            .unwrap()])
        .unwrap();
    dir
}
fn prepared(root: &Path) -> PreparedProject {
    PreparedProject::open_with_ui(
        root,
        ProgressSupport::Unsupported,
        SpriteSupport::Unsupported,
        true,
    )
    .unwrap()
}
fn app(project: PreparedProject, cc: &eframe::CreationContext<'_>) -> EditorApp {
    let mut editor = crate::Editor::start(&crate::HostSpec::PreparedCollect {
        scene: project.path().to_owned(),
        text: project.scene().text().into(),
        listen: None,
        debug_hooks: false,
    })
    .unwrap();
    editor.sync();
    let mut app = EditorApp::new(editor, cc.wgpu_render_state.clone());
    crate::project::install_collect_presentation(&mut app, project, &cc.egui_ctx);
    app
}
fn click(h: &mut Harness<'_, EditorApp>, label: &str) {
    h.get_by_label(label).click();
    h.run_steps(4);
}
fn select_node(h: &mut Harness<'_, EditorApp>, name: &str) {
    h.get_by_role_and_label(Role::ComboBox, "UI node").click();
    h.run_steps(3);
    h.get_by_role_and_label(Role::Button, name).click();
    h.run_steps(3);
}
fn edit_text(h: &mut Harness<'_, EditorApp>, old: &str, new: &str) {
    h.get_by(|node| node.role() == Role::TextInput && node.value().as_deref() == Some(old))
        .focus();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(2);
    h.get_by(|node| node.role() == Role::TextInput && node.value().as_deref() == Some(old))
        .type_text(new);
    h.run_steps(4);
}
fn pointer(h: &mut Harness<'_, EditorApp>, point: egui::Pos2, pressed: bool) {
    h.event(egui::Event::PointerMoved(point));
    h.event(egui::Event::PointerButton {
        pos: point,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    });
    h.run_steps(2);
}
fn canvas_click(h: &mut Harness<'_, EditorApp>, point: [f32; 2]) {
    let rect = h.state().collect_ui_preview.canvas.unwrap();
    let point = rect.min + egui::vec2(point[0], point[1]);
    pointer(h, point, true);
    pointer(h, point, false);
}
fn host_stamp(
    h: &Harness<'_, EditorApp>,
) -> (
    u64,
    crate::model::History,
    crate::Mode,
    crate::editor::input::Phase,
) {
    let editor = &h.state().editor;
    (
        editor.checksum(),
        editor.history().clone(),
        editor.mode(),
        editor.input_phase(),
    )
}

#[test]
fn actual_editor_preview_controls_preserve_host_and_scene_epoch() {
    let root = fixture();
    let project = prepared(root.path());
    let font = project.ui().unwrap().font.clone();
    let mut h = Harness::builder()
        .with_size([1800.0, 1200.0])
        .renderer(egui_kittest::LazyRenderer::Uninitialized {
            textures_delta: Default::default(),
            builder: None,
        })
        .build_eframe(|cc| app(project, cc));
    h.run_steps(4);
    let stamp = host_stamp(&h);
    let scene = fs::read(root.path().join("scene.yaml")).unwrap();
    let original = fs::read(root.path().join("ui.json")).unwrap();
    click(&mut h, "Collect UI document");
    click(&mut h, "Preview authored screens");
    h.get_by_label("A GPU is required to draw the authored-screen preview.");
    select_node(&mut h, "title");
    edit_text(
        &mut h,
        "Collect and Dodge · 플레이",
        "Unsaved authored title",
    );
    assert_eq!(
        h.state().collect_ui_preview.state.document(),
        h.state().collect_ui.as_ref().map(|panel| panel.document())
    );
    assert_eq!(fs::read(root.path().join("ui.json")).unwrap(), original);
    click(&mut h, "Portrait 360 × 640");
    click(&mut h, "Terminal");
    assert_eq!(
        h.state().collect_ui_preview.state.settings().viewport,
        PORTRAIT
    );
    assert_eq!(host_stamp(&h), stamp);
    click(&mut h, "Undo UI edit");
    assert_eq!(
        h.state()
            .collect_ui
            .as_ref()
            .unwrap()
            .document()
            .to_bytes()
            .unwrap(),
        original
    );
    edit_text(&mut h, "Collect and Dodge · 플레이", "Saved authored title");
    click(&mut h, "Save UI document");
    assert_eq!(
        prepared(root.path()).ui().unwrap().document,
        *h.state().collect_ui.as_ref().unwrap().document()
    );
    assert_eq!(prepared(root.path()).ui().unwrap().font, font);
    assert_eq!(fs::read(root.path().join("scene.yaml")).unwrap(), scene);
    assert_eq!(host_stamp(&h), stamp);
    click(&mut h, "Close screen preview");
    assert!(!h.state().collect_ui_preview.state.is_open());
    click(&mut h, "Preview authored screens");
    assert!(h.state().collect_ui_preview.state.is_open());
    // A replacement Editor can have the same scene path but a new load epoch.
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
    assert!(!h.state().collect_ui_preview.state.is_available());
    assert!(!h.state().collect_ui_preview.state.is_open());
}

fn pixels(gpu: &PreviewGpu) -> Vec<u8> {
    let row = gpu.size[0] * 4;
    let padded = row.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let buffer = gpu.host.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Collect preview test readback"),
        size: u64::from(padded) * u64::from(gpu.size[1]),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.host.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &gpu.texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(gpu.size[1]),
            },
        },
        wgpu::Extent3d {
            width: gpu.size[0],
            height: gpu.size[1],
            depth_or_array_layers: 1,
        },
    );
    gpu.host.queue.submit([encoder.finish()]);
    let (sender, receiver) = std::sync::mpsc::channel();
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            sender.send(result).unwrap();
        });
    gpu.host
        .device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(Duration::from_secs(20)),
        })
        .unwrap();
    receiver
        .recv_timeout(Duration::from_secs(20))
        .unwrap()
        .unwrap();
    let mapped = buffer.slice(..).get_mapped_range().unwrap();
    mapped
        .chunks_exact(padded as usize)
        .flat_map(|row_bytes| row_bytes[..row as usize].iter().copied())
        .collect()
}
fn write_png(path: &Path, size: [u32; 2], bytes: &[u8]) {
    let mut encoder = png::Encoder::new(fs::File::create(path).unwrap(), size[0], size[1]);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(bytes)
        .unwrap();
}
fn parity(h: &mut Harness<'_, EditorApp>, font: &[u8], evidence: &Path, name: &str) {
    let document = h.state().collect_ui.as_ref().unwrap().document().clone();
    parity_document(h, font, evidence, name, &document);
}
fn parity_document(
    h: &mut Harness<'_, EditorApp>,
    font: &[u8],
    evidence: &Path,
    name: &str,
    document: &Document,
) {
    // Clear transient hover; the oracle starts with no pointer and a fresh UI.
    h.event(egui::Event::PointerGone);
    h.run_steps(20);
    let preview = &h.state().collect_ui_preview;
    let settings = preview.state.settings().clone();
    let actual = pixels(preview.gpu.as_ref().unwrap());
    let mut production = CollectUi::new(document.clone(), font.to_vec()).unwrap();
    match settings.screen {
        Screen::Title => {}
        Screen::Playing | Screen::Terminal => production.apply(Action::Play),
        Screen::Menu => production.apply(Action::Menu),
    }
    let mut oracle = PreviewGpu::new(
        h.state().render_state.as_ref().unwrap(),
        settings.viewport,
        0,
    );
    for _ in 0..4 {
        let mut raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(settings.viewport[0] as f32, settings.viewport[1] as f32),
            )),
            ..Default::default()
        };
        raw.viewports
            .entry(egui::ViewportId::ROOT)
            .or_default()
            .native_pixels_per_point = Some(1.0);
        let hud = Hud {
            score: settings.score,
            phase: match settings.screen {
                Screen::Playing => 0,
                Screen::Terminal => settings.phase.max(1),
                _ => settings.phase,
            },
            best: settings.best.clone(),
        };
        let (output, action) = production.show(raw, hud);
        assert_eq!(action, None);
        oracle.render(Frame {
            context: production.context.clone(),
            output,
            action,
            generation: 0,
        });
    }
    let expected = pixels(&oracle);
    if actual != expected {
        write_png(
            &evidence.join(format!("{name}-failed-actual.png")),
            settings.viewport,
            &actual,
        );
        write_png(
            &evidence.join(format!("{name}-failed-oracle.png")),
            settings.viewport,
            &expected,
        );
        let first = actual.iter().zip(&expected).position(|(a, b)| a != b);
        panic!("independently driven production CollectUi pixels differ for {name}; first byte: {first:?}");
    }
    assert!(
        actual
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|pixel| pixel[0] > 180 && pixel[1] > 180 && pixel[2] > 180)
            .count()
            > 100,
        "readable text must contribute real pixels"
    );
    write_png(
        &evidence.join(format!("{name}-canvas.png")),
        settings.viewport,
        &actual,
    );
    h.render()
        .unwrap()
        .save(evidence.join(format!("{name}-editor.png")))
        .unwrap();
    // Full width controls and the entire canvas fit from the first default layout.
    let screen = h.ctx.content_rect();
    for label in [
        "Title",
        "Playing",
        "Menu",
        "Terminal",
        "Landscape 640 × 360",
        "Portrait 360 × 640",
        "Sample score",
        "Sample best",
        "Close screen preview",
        "Move UI position",
        "Move node ID",
        "Apply position",
        "Cancel position",
        "Resize UI size",
        "Resize node ID",
        "Apply size",
        "Cancel size",
        "100%",
        "75%",
    ] {
        assert!(
            screen.contains_rect(h.get_by_label(label).rect()),
            "full visible control {label}"
        );
    }
    assert!(screen.contains_rect(h.state().collect_ui_preview.canvas.unwrap()));
}

#[test]
#[ignore = "mandatory explicit software Vulkan actual EditorApp preview acceptance"]
fn actual_editor_preview_gpu_matches_production_and_never_dispatches() {
    let evidence = PathBuf::from(
        std::env::var_os("ORR_COLLECT_UI_PREVIEW_EVIDENCE")
            .expect("retained evidence directory required"),
    );
    assert!(evidence.is_absolute() && evidence.is_dir());
    let root = fixture();
    let project = prepared(root.path());
    let font = project.ui().unwrap().font.clone();
    let initial = project.ui().unwrap().document.clone();
    let original_file = fs::read(root.path().join("ui.json")).unwrap();
    let original_scene = fs::read(root.path().join("scene.yaml")).unwrap();
    let original_manifest = fs::read(root.path().join("orr.project.json")).unwrap();
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
    parity(&mut h, &font, &evidence, "title-default-landscape");
    select_node(&mut h, "title");
    edit_text(
        &mut h,
        "Collect and Dodge · 플레이",
        "Preview unsaved · 플레이",
    );
    select_node(&mut h, "play");
    edit_text(&mut h, "플레이", "Play preview");
    assert_eq!(
        fs::read(root.path().join("ui.json")).unwrap(),
        original_file
    );
    parity(&mut h, &font, &evidence, "title-unsaved-landscape");
    canvas_click(&mut h, [320.0, 182.0]);
    assert_eq!(
        h.state().collect_ui_preview.state.last_action(),
        Some(Action::Play)
    );
    assert_eq!(
        h.state().collect_ui_preview.state.settings().screen,
        Screen::Title
    );
    assert_eq!(host_stamp(&h), stamp);
    // Clipped-away portion is not clickable, even though inside the canvas.
    canvas_click(&mut h, [200.0, 305.0]);
    assert_eq!(
        h.state().collect_ui_preview.state.last_action(),
        Some(Action::Play)
    );
    canvas_click(&mut h, [145.0, 305.0]);
    assert_eq!(
        h.state().collect_ui_preview.state.last_action(),
        Some(Action::Quit)
    );
    click(&mut h, "Portrait 360 × 640");
    parity(&mut h, &font, &evidence, "title-unsaved-portrait");
    // Press inside, leave, return, and release: the old press is cancelled.
    let rect = h.state().collect_ui_preview.canvas.unwrap();
    pointer(&mut h, rect.min + egui::vec2(180.0, 182.0), true);
    h.event(egui::Event::PointerMoved(rect.min - egui::vec2(3.0, 3.0)));
    h.run_steps(2);
    pointer(&mut h, rect.min + egui::vec2(180.0, 182.0), false);
    assert_eq!(h.state().collect_ui_preview.state.last_action(), None);
    click(&mut h, "Playing");
    parity(&mut h, &font, &evidence, "playing-portrait");
    canvas_click(&mut h, [180.0, 118.0]);
    assert_eq!(
        h.state().collect_ui_preview.state.last_action(),
        Some(Action::Menu)
    );
    assert_eq!(host_stamp(&h), stamp);
    click(&mut h, "Menu");
    parity(&mut h, &font, &evidence, "menu-portrait");
    canvas_click(&mut h, [180.0, 174.0]);
    assert_eq!(
        h.state().collect_ui_preview.state.last_action(),
        Some(Action::Restart)
    );
    click(&mut h, "Terminal");
    click(&mut h, "Landscape 640 × 360");
    parity(&mut h, &font, &evidence, "terminal-landscape");
    canvas_click(&mut h, [320.0, 214.0]);
    assert_eq!(
        h.state().collect_ui_preview.state.last_action(),
        Some(Action::Restart)
    );
    assert_eq!(host_stamp(&h), stamp);
    click(&mut h, "Undo UI edit");
    click(&mut h, "Undo UI edit");
    assert_eq!(h.state().collect_ui.as_ref().unwrap().document(), &initial);
    select_node(&mut h, "title");
    edit_text(&mut h, "Collect and Dodge · 플레이", "Saved preview title");
    click(&mut h, "Save UI document");
    let saved = prepared(root.path());
    assert_eq!(
        saved.ui().unwrap().document,
        *h.state().collect_ui.as_ref().unwrap().document()
    );
    assert_eq!(
        fs::read(root.path().join("scene.yaml")).unwrap(),
        original_scene
    );
    assert_eq!(
        fs::read(root.path().join("orr.project.json")).unwrap(),
        original_manifest
    );
    assert_eq!(host_stamp(&h), stamp);
    click(&mut h, "Close screen preview");
    assert!(h.state().collect_ui_preview.gpu.is_none());
    let context = h.ctx.clone();
    crate::project::install_collect_presentation(h.state_mut(), saved, &context);
    // Reinstallation resets the panel selection and changes its widget layout.
    h.run_steps(4);
    click(&mut h, "Preview authored screens");
    click(&mut h, "Title");
    parity(&mut h, &font, &evidence, "title-saved-reopened");
    h.state_mut().collect_ui = None;
    h.run_steps(3);
    assert!(h.state().collect_ui_preview.gpu.is_none());
    assert!(!h.state().collect_ui_preview.state.is_available());
    fs::write(evidence.join("collect-ui-preview-acceptance.json"), serde_json::to_vec_pretty(&serde_json::json!({
        "schema":1, "production_editor_app":true, "window_pixels":[1800,1200], "software_vulkan":true, "adapter":adapter,
        "full_default_width_controls":true, "unsaved_label_and_button_visible":true,
        "landscape_and_portrait":true, "production_collect_ui_pixel_parity_cases":7,
        "clipping_and_admitted_font":true, "observed_play_menu_restart_quit":true,
        "pointer_leave_cancelled":true, "host_checksum_history_mode_input_unchanged":true,
        "ui_undo_save_reopen":true, "scene_manifest_unchanged":true,
        "close_and_missing_panel_release_gpu":true, "native_os_window":false
    })).unwrap()).unwrap();
}

fn select_move_node(h: &mut Harness<'_, EditorApp>, id: &str) {
    h.get_by_role_and_label(Role::ComboBox, "Move node ID")
        .click();
    h.run_steps(3);
    h.get_by_role_and_label(Role::Button, id).click();
    h.run_steps(3);
}
fn move_pointer(h: &mut Harness<'_, EditorApp>, point: egui::Pos2) {
    h.event(egui::Event::PointerMoved(point));
    h.run_steps(3);
}
fn position_drag(h: &mut Harness<'_, EditorApp>, delta: [f32; 2]) {
    let rect = h.state().collect_ui_preview.canvas.unwrap();
    let start = rect.min + rect.size() * 0.5;
    pointer(h, start, true);
    assert!(h.state().collect_ui_preview.position.state.is_dragging());
    let generation = h.state().collect_ui_preview.state.generation();
    let end = start + egui::vec2(delta[0], delta[1]);
    move_pointer(h, end);
    assert_eq!(
        h.state().collect_ui_preview.canvas,
        Some(rect),
        "status must not move canvas"
    );
    assert_eq!(
        h.state().collect_ui_preview.state.generation(),
        generation,
        "motion retains font/context"
    );
    pointer(h, end, false);
    assert!(h.state().collect_ui_preview.position.state.is_ready());
    assert_eq!(
        h.state().collect_ui_preview.state.generation(),
        generation + 1,
        "release builds candidate once"
    );
}
#[track_caller]
fn assert_no_position_draft(h: &Harness<'_, EditorApp>) {
    let preview = &h.state().collect_ui_preview;
    assert!(!preview.position.state.is_dragging());
    assert!(!preview.position.state.is_ready());
    if preview.state.is_open() {
        assert_eq!(
            preview.state.rendered_document(),
            h.state().collect_ui.as_ref().map(|panel| panel.document())
        );
    }
}

#[test]
fn actual_editor_position_controls_keep_explicit_selection_separate() {
    let root = fixture();
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
    select_node(&mut h, "title");
    click(&mut h, "Preview authored screens");
    select_move_node(&mut h, "play");
    click(&mut h, "Move UI position");
    assert!(h.state().collect_ui_preview.position.moving);
    assert!(h
        .get_by_label("Apply position")
        .accesskit_node()
        .is_disabled());
    assert!(h
        .get_by_label("Cancel position")
        .accesskit_node()
        .is_disabled());
    assert_eq!(h.state().collect_ui.as_ref().unwrap().document(), &initial);
    assert_eq!(host_stamp(&h), stamp);
    // The reserved inspector's selected title remains independently editable.
    edit_text(
        &mut h,
        "Collect and Dodge · 플레이",
        "Independent title selection",
    );
    assert!(
        matches!(&h.state().collect_ui.as_ref().unwrap().document().nodes[0].kind, Kind::Label { text, .. } if text == "Independent title selection")
    );
    assert_eq!(
        h.state().collect_ui.as_ref().unwrap().document().nodes[2],
        initial.nodes[2]
    );
}

#[test]
#[ignore = "mandatory explicit software Vulkan actual EditorApp position acceptance"]
fn actual_editor_position_gpu_stages_applies_and_cancels_offsets() {
    let evidence = PathBuf::from(
        std::env::var_os("ORR_COLLECT_UI_POSITION_EVIDENCE")
            .expect("retained evidence directory required"),
    );
    assert!(evidence.is_absolute() && evidence.is_dir());
    let root = fixture();
    let project = prepared(root.path());
    let font = project.ui().unwrap().font.clone();
    let initial = project.ui().unwrap().document.clone();
    let original_file = fs::read(root.path().join("ui.json")).unwrap();
    let scene = fs::read(root.path().join("scene.yaml")).unwrap();
    let manifest = fs::read(root.path().join("orr.project.json")).unwrap();
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
    select_move_node(&mut h, "play");
    click(&mut h, "Move UI position");
    parity(&mut h, &font, &evidence, "position-initial-title");
    let initial_pixels = pixels(h.state().collect_ui_preview.gpu.as_ref().unwrap());
    let rect = h.state().collect_ui_preview.canvas.unwrap();
    let start = rect.min + egui::vec2(320.0, 180.0);
    pointer(&mut h, start, true);
    let generation = h.state().collect_ui_preview.state.generation();
    move_pointer(&mut h, start + egui::vec2(32.0, 32.0));
    move_pointer(&mut h, start + egui::vec2(64.0, 64.0));
    assert!(h.state().collect_ui_preview.position.state.is_dragging());
    assert_eq!(h.state().collect_ui_preview.canvas, Some(rect));
    assert_eq!(h.state().collect_ui_preview.state.generation(), generation);
    assert_eq!(
        pixels(h.state().collect_ui_preview.gpu.as_ref().unwrap()),
        initial_pixels
    );
    assert_eq!(h.state().collect_ui.as_ref().unwrap().document(), &initial);
    assert_eq!(host_stamp(&h), stamp);
    h.render()
        .unwrap()
        .save(evidence.join("position-drag-ghost-editor.png"))
        .unwrap();
    pointer(&mut h, start + egui::vec2(64.0, 64.0), false);
    assert_eq!(
        h.state().collect_ui_preview.state.generation(),
        generation + 1
    );
    let mut moved = initial.clone();
    let play = moved
        .nodes
        .iter_mut()
        .find(|node| node.id == "play")
        .unwrap();
    play.offset[0] += 64;
    play.offset[1] += 64;
    assert_eq!(
        h.state().collect_ui_preview.position.state.document(),
        Some(&moved)
    );
    assert_eq!(h.state().collect_ui.as_ref().unwrap().document(), &initial);
    assert_eq!(h.state().collect_ui_preview.state.last_action(), None);
    assert_eq!(
        fs::read(root.path().join("ui.json")).unwrap(),
        original_file
    );
    parity_document(&mut h, &font, &evidence, "position-staged-title", &moved);
    assert_ne!(
        pixels(h.state().collect_ui_preview.gpu.as_ref().unwrap()),
        initial_pixels
    );
    click(&mut h, "Cancel position");
    assert_no_position_draft(&h);
    parity(&mut h, &font, &evidence, "position-cancelled-title");
    assert_eq!(
        pixels(h.state().collect_ui_preview.gpu.as_ref().unwrap()),
        initial_pixels
    );
    assert!(h
        .get_by_label("Undo UI edit")
        .accesskit_node()
        .is_disabled());

    // Every interruption cancels an uncommitted candidate and restores accepted
    // pixels; no stale release or Apply can resurrect it.
    for interruption in 0..5 {
        position_drag(&mut h, [64.0, 64.0]);
        match interruption {
            0 => {
                h.key_press(egui::Key::Escape);
                h.run_steps(3);
            }
            1 => {
                h.event(egui::Event::WindowFocused(false));
                h.run_steps(3);
                h.event(egui::Event::WindowFocused(true));
                h.run_steps(3);
            }
            2 => {
                click(&mut h, "Menu");
                click(&mut h, "Title");
            }
            3 => {
                click(&mut h, "75%");
                click(&mut h, "100%");
            }
            _ => {
                click(&mut h, "Close screen preview");
                click(&mut h, "Preview authored screens");
                if !h.state().collect_ui_preview.position.moving {
                    click(&mut h, "Move UI position");
                }
            }
        }
        assert!(
            !h.state().collect_ui_preview.position.state.is_ready(),
            "interruption {interruption} must cancel the released candidate"
        );
        assert_no_position_draft(&h);
        assert_eq!(h.state().collect_ui.as_ref().unwrap().document(), &initial);
        assert!(h
            .get_by_label("Undo UI edit")
            .accesskit_node()
            .is_disabled());
    }
    let rect = h.state().collect_ui_preview.canvas.unwrap();
    let start = rect.center();
    pointer(&mut h, start, true);
    assert!(h.state().collect_ui_preview.position.state.is_dragging());
    move_pointer(&mut h, rect.min - egui::vec2(2.0, 2.0));
    assert_no_position_draft(&h);
    pointer(&mut h, start, false);
    assert_no_position_draft(&h);
    assert!(h
        .get_by_label("Undo UI edit")
        .accesskit_node()
        .is_disabled());
    // A concurrent accepted edit cancels the full-document pin, even if the
    // selected node itself was not changed.
    position_drag(&mut h, [64.0, 64.0]);
    let mut changed = initial.clone();
    changed.nodes[0].offset[0] += 1;
    h.state_mut()
        .collect_ui
        .as_mut()
        .unwrap()
        .apply_document(changed.clone())
        .unwrap();
    h.run_steps(3);
    assert_no_position_draft(&h);
    assert_eq!(
        h.state().collect_ui_preview.state.rendered_document(),
        Some(&changed)
    );
    click(&mut h, "Undo UI edit");
    assert_eq!(h.state().collect_ui.as_ref().unwrap().document(), &initial);

    position_drag(&mut h, [64.0, 64.0]);
    click(&mut h, "Apply position");
    assert_no_position_draft(&h);
    assert_eq!(h.state().collect_ui.as_ref().unwrap().document(), &moved);
    parity(&mut h, &font, &evidence, "position-applied-title");
    click(&mut h, "Move UI position");
    canvas_click(&mut h, [320.0, 180.0]);
    assert_eq!(
        h.state().collect_ui_preview.state.last_action(),
        None,
        "old button location must not act"
    );
    canvas_click(&mut h, [384.0, 250.0]);
    assert_eq!(
        h.state().collect_ui_preview.state.last_action(),
        Some(Action::Play)
    );
    assert_eq!(host_stamp(&h), stamp);
    click(&mut h, "Undo UI edit");
    assert_eq!(h.state().collect_ui.as_ref().unwrap().document(), &initial);
    assert!(
        h.get_by_label("Undo UI edit")
            .accesskit_node()
            .is_disabled(),
        "one Apply adds exactly one undo entry"
    );

    click(&mut h, "Portrait 360 × 640");
    click(&mut h, "75%");
    select_move_node(&mut h, "clip_panel");
    click(&mut h, "Move UI position");
    position_drag(&mut h, [30.0, 15.0]);
    let mut nested = initial.clone();
    let container = nested
        .nodes
        .iter_mut()
        .find(|node| node.id == "clip_panel")
        .unwrap();
    container.offset[0] += 40;
    container.offset[1] += 20;
    assert_eq!(
        h.state().collect_ui_preview.position.state.document(),
        Some(&nested)
    );
    parity_document(
        &mut h,
        &font,
        &evidence,
        "position-portrait-container",
        &nested,
    );
    click(&mut h, "Apply position");
    assert_eq!(h.state().collect_ui.as_ref().unwrap().document(), &nested);
    assert_eq!(
        fs::read(root.path().join("ui.json")).unwrap(),
        original_file
    );
    click(&mut h, "Save UI document");
    let saved = prepared(root.path());
    assert_eq!(saved.ui().unwrap().document, nested);
    assert_eq!(saved.ui().unwrap().font, font);
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
    parity(&mut h, &font, &evidence, "position-saved-reopened");
    // Reopening the same path with a new scene-load epoch invalidates a staged
    // transaction before any late Apply can reach the replacement editor.
    select_move_node(&mut h, "play");
    if !h.state().collect_ui_preview.position.moving {
        click(&mut h, "Move UI position");
    }
    position_drag(&mut h, [20.0, 20.0]);
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
    assert_no_position_draft(&h);
    assert!(!h.state().collect_ui_preview.state.is_available());
    assert_eq!(h.state().collect_ui.as_ref().unwrap().document(), &nested);
    let context = h.ctx.clone();
    crate::project::install_collect_presentation(h.state_mut(), replacement, &context);
    h.run_steps(4);
    click(&mut h, "Preview authored screens");
    // Explicit Play and absent-panel lifecycle boundaries.
    select_move_node(&mut h, "play");
    if !h.state().collect_ui_preview.position.moving {
        click(&mut h, "Move UI position");
    }
    position_drag(&mut h, [20.0, 20.0]);
    assert!(h.state_mut().editor.start_play());
    h.run_steps(3);
    assert_no_position_draft(&h);
    assert!(!h.state().collect_ui_preview.position.moving);
    assert!(h
        .get_by_label("Move UI position")
        .accesskit_node()
        .is_disabled());
    assert_eq!(h.state().collect_ui.as_ref().unwrap().document(), &nested);
    h.state_mut().collect_ui = None;
    h.run_steps(3);
    assert!(!h.state().collect_ui_preview.state.is_available());
    assert_no_position_draft(&h);
    fs::write(evidence.join("collect-ui-position-acceptance.json"), serde_json::to_vec_pretty(&serde_json::json!({
        "schema":1, "production_editor_app":true, "software_vulkan":true, "adapter":adapter, "window_pixels":[1800,1200],
        "position_ghost_without_context_rebuild":true, "release_candidate_build_once":true,
        "staging_changes_pixels_only":true, "cancel_restores_exact_pixels":true,
        "apply_one_undo":true, "old_and_new_action_hit_locations":true,
        "portrait_display_scale":0.75, "parent_moves_descendants_without_rewriting_them":true,
        "escape_focus_screen_resize_close_stale_document_play_cancel":true,
        "pointer_leave_late_release_cancel":true, "same_path_scene_epoch_cancel":true,
        "host_unchanged_during_authoring":true, "save_reopen":true,
        "production_collect_ui_pixel_parity_cases":6, "full_visible_controls":true,
        "native_os_window":false
    })).unwrap()).unwrap();
}

fn select_resize_node(h: &mut Harness<'_, EditorApp>, id: &str) {
    h.get_by_role_and_label(Role::ComboBox, "Resize node ID")
        .click();
    h.run_steps(3);
    h.get_by_role_and_label(Role::Button, id).click();
    h.run_steps(3);
}
fn resize_drag(h: &mut Harness<'_, EditorApp>, delta: [f32; 2]) {
    let rect = h.state().collect_ui_preview.canvas.unwrap();
    let start = rect.min + rect.size() * 0.5;
    pointer(h, start, true);
    assert!(h.state().collect_ui_preview.resize.state.is_dragging());
    let generation = h.state().collect_ui_preview.state.generation();
    let end = start + egui::vec2(delta[0], delta[1]);
    move_pointer(h, end);
    assert_eq!(
        h.state().collect_ui_preview.canvas,
        Some(rect),
        "status must not move canvas"
    );
    assert_eq!(
        h.state().collect_ui_preview.state.generation(),
        generation,
        "motion retains font/context"
    );
    pointer(h, end, false);
    assert!(h.state().collect_ui_preview.resize.state.is_ready());
    assert_eq!(
        h.state().collect_ui_preview.state.generation(),
        generation + 1,
        "release builds candidate once"
    );
}
#[track_caller]
fn assert_no_resize_draft(h: &Harness<'_, EditorApp>) {
    let preview = &h.state().collect_ui_preview;
    assert!(!preview.resize.state.is_dragging());
    assert!(!preview.resize.state.is_ready());
    if preview.state.is_open() {
        assert_eq!(
            preview.state.rendered_document(),
            h.state().collect_ui.as_ref().map(|panel| panel.document())
        );
    }
}

#[test]
fn actual_editor_resize_controls_keep_explicit_selection_separate() {
    let root = fixture();
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
    select_node(&mut h, "title");
    click(&mut h, "Preview authored screens");
    select_resize_node(&mut h, "play");
    click(&mut h, "Resize UI size");
    assert!(h.state().collect_ui_preview.resize.resizing);
    assert!(!h.state().collect_ui_preview.position.moving);
    click(&mut h, "Move UI position");
    assert!(h.state().collect_ui_preview.position.moving);
    assert!(!h.state().collect_ui_preview.resize.resizing);
    click(&mut h, "Resize UI size");
    assert!(h.state().collect_ui_preview.resize.resizing);
    assert!(!h.state().collect_ui_preview.position.moving);
    assert!(h.get_by_label("Apply size").accesskit_node().is_disabled());
    assert!(h.get_by_label("Cancel size").accesskit_node().is_disabled());
    assert_eq!(h.state().collect_ui.as_ref().unwrap().document(), &initial);
    assert_eq!(host_stamp(&h), stamp);
    // The reserved inspector's selected title remains independently editable.
    edit_text(
        &mut h,
        "Collect and Dodge · 플레이",
        "Independent title selection",
    );
    assert!(
        matches!(&h.state().collect_ui.as_ref().unwrap().document().nodes[0].kind, Kind::Label { text, .. } if text == "Independent title selection")
    );
    assert_eq!(
        h.state().collect_ui.as_ref().unwrap().document().nodes[2],
        initial.nodes[2]
    );
}

#[test]
#[ignore = "mandatory explicit software Vulkan actual EditorApp resize acceptance"]
fn actual_editor_resize_gpu_stages_applies_and_cancels_sizes() {
    let evidence = PathBuf::from(
        std::env::var_os("ORR_COLLECT_UI_RESIZE_EVIDENCE")
            .expect("retained evidence directory required"),
    );
    assert!(evidence.is_absolute() && evidence.is_dir());
    let root = fixture();
    // Three authored child anchors exercise production parent-relative layout.
    let mut anchored = Document::parse(&fs::read(root.path().join("ui.json")).unwrap()).unwrap();
    for (anchor, offset) in [(0, [0, 0]), (500, [-15, -8]), (1000, [-30, -16])] {
        anchored.nodes.push(Node {
            id: format!("resize_anchor_{anchor}"),
            parent: Some("clip_panel".into()),
            kind: Kind::Label {
                text: format!("{anchor}"),
                binding: None,
            },
            screen: Screen::Title,
            anchor: [anchor, anchor],
            offset,
            size: [30, 16],
        });
    }
    fs::write(root.path().join("ui.json"), anchored.to_bytes().unwrap()).unwrap();
    let project = prepared(root.path());
    let font = project.ui().unwrap().font.clone();
    let initial = project.ui().unwrap().document.clone();
    let original_file = fs::read(root.path().join("ui.json")).unwrap();
    let scene = fs::read(root.path().join("scene.yaml")).unwrap();
    let manifest = fs::read(root.path().join("orr.project.json")).unwrap();
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
    select_resize_node(&mut h, "play");
    click(&mut h, "Resize UI size");
    parity(&mut h, &font, &evidence, "resize-initial-title");
    let initial_pixels = pixels(h.state().collect_ui_preview.gpu.as_ref().unwrap());
    let rect = h.state().collect_ui_preview.canvas.unwrap();
    let start = rect.min + egui::vec2(320.0, 180.0);
    pointer(&mut h, start, true);
    let generation = h.state().collect_ui_preview.state.generation();
    move_pointer(&mut h, start + egui::vec2(32.0, 32.0));
    move_pointer(&mut h, start + egui::vec2(64.0, 64.0));
    assert!(h.state().collect_ui_preview.resize.state.is_dragging());
    assert_eq!(h.state().collect_ui_preview.canvas, Some(rect));
    assert_eq!(h.state().collect_ui_preview.state.generation(), generation);
    assert_eq!(
        pixels(h.state().collect_ui_preview.gpu.as_ref().unwrap()),
        initial_pixels
    );
    assert_eq!(h.state().collect_ui.as_ref().unwrap().document(), &initial);
    assert_eq!(host_stamp(&h), stamp);
    h.render()
        .unwrap()
        .save(evidence.join("resize-drag-ghost-editor.png"))
        .unwrap();
    pointer(&mut h, start + egui::vec2(64.0, 64.0), false);
    assert_eq!(
        h.state().collect_ui_preview.state.generation(),
        generation + 1
    );
    let mut moved = initial.clone();
    let play = moved
        .nodes
        .iter_mut()
        .find(|node| node.id == "play")
        .unwrap();
    play.size[0] += 64;
    play.size[1] += 64;
    assert_eq!(
        h.state().collect_ui_preview.resize.state.document(),
        Some(&moved)
    );
    assert_eq!(h.state().collect_ui.as_ref().unwrap().document(), &initial);
    assert_eq!(h.state().collect_ui_preview.state.last_action(), None);
    assert_eq!(
        fs::read(root.path().join("ui.json")).unwrap(),
        original_file
    );
    select_move_node(&mut h, "title");
    assert_eq!(
        h.state().collect_ui_preview.state.rendered_document(),
        Some(&moved),
        "inactive Move selector must not clear resize staging"
    );
    parity_document(&mut h, &font, &evidence, "resize-staged-title", &moved);
    assert_ne!(
        pixels(h.state().collect_ui_preview.gpu.as_ref().unwrap()),
        initial_pixels
    );
    click(&mut h, "Cancel size");
    assert_no_resize_draft(&h);
    parity(&mut h, &font, &evidence, "resize-cancelled-title");
    assert_eq!(
        pixels(h.state().collect_ui_preview.gpu.as_ref().unwrap()),
        initial_pixels
    );
    assert!(h
        .get_by_label("Undo UI edit")
        .accesskit_node()
        .is_disabled());

    // Every interruption cancels an uncommitted candidate and restores accepted
    // pixels; no stale release or Apply can resurrect it.
    for interruption in 0..8 {
        resize_drag(&mut h, [64.0, 64.0]);
        match interruption {
            0 => {
                h.key_press(egui::Key::Escape);
                h.run_steps(3);
            }
            1 => {
                h.event(egui::Event::WindowFocused(false));
                h.run_steps(3);
                h.event(egui::Event::WindowFocused(true));
                h.run_steps(3);
            }
            2 => {
                click(&mut h, "Menu");
                click(&mut h, "Title");
            }
            3 => {
                click(&mut h, "75%");
                click(&mut h, "100%");
            }
            4 => {
                h.input_mut().focused = false;
                h.run_steps(3);
                assert_no_resize_draft(&h);
                h.input_mut().focused = true;
                h.run_steps(3);
            }
            5 => {
                click(&mut h, "Move UI position");
                assert_no_resize_draft(&h);
                click(&mut h, "Resize UI size");
            }
            6 => {
                select_resize_node(&mut h, "title");
                assert_no_resize_draft(&h);
                select_resize_node(&mut h, "play");
            }
            _ => {
                click(&mut h, "Close screen preview");
                click(&mut h, "Preview authored screens");
                if !h.state().collect_ui_preview.resize.resizing {
                    click(&mut h, "Resize UI size");
                }
            }
        }
        assert!(
            !h.state().collect_ui_preview.resize.state.is_ready(),
            "interruption {interruption} must cancel the released candidate"
        );
        assert_no_resize_draft(&h);
        assert_eq!(h.state().collect_ui.as_ref().unwrap().document(), &initial);
        assert!(h
            .get_by_label("Undo UI edit")
            .accesskit_node()
            .is_disabled());
    }
    let rect = h.state().collect_ui_preview.canvas.unwrap();
    let start = rect.center();
    pointer(&mut h, start, true);
    assert!(h.state().collect_ui_preview.resize.state.is_dragging());
    move_pointer(&mut h, rect.min - egui::vec2(2.0, 2.0));
    assert_no_resize_draft(&h);
    pointer(&mut h, start, false);
    assert_no_resize_draft(&h);
    assert!(h
        .get_by_label("Undo UI edit")
        .accesskit_node()
        .is_disabled());
    // A concurrent accepted edit cancels the full-document pin, even if the
    // selected node itself was not changed.
    resize_drag(&mut h, [64.0, 64.0]);
    let mut changed = initial.clone();
    changed.nodes[0].size[0] += 1;
    h.state_mut()
        .collect_ui
        .as_mut()
        .unwrap()
        .apply_document(changed.clone())
        .unwrap();
    h.run_steps(3);
    assert_no_resize_draft(&h);
    assert_eq!(
        h.state().collect_ui_preview.state.rendered_document(),
        Some(&changed)
    );
    click(&mut h, "Undo UI edit");
    assert_eq!(h.state().collect_ui.as_ref().unwrap().document(), &initial);

    resize_drag(&mut h, [64.0, 64.0]);
    click(&mut h, "Apply size");
    assert_no_resize_draft(&h);
    assert_eq!(h.state().collect_ui.as_ref().unwrap().document(), &moved);
    parity(&mut h, &font, &evidence, "resize-applied-title");
    click(&mut h, "Resize UI size");
    canvas_click(&mut h, [560.0, 180.0]);
    assert_eq!(
        h.state().collect_ui_preview.state.last_action(),
        None,
        "outside the enlarged button must not act"
    );
    canvas_click(&mut h, [520.0, 220.0]);
    assert_eq!(
        h.state().collect_ui_preview.state.last_action(),
        Some(Action::Play)
    );
    assert_eq!(host_stamp(&h), stamp);
    click(&mut h, "Undo UI edit");
    assert_eq!(h.state().collect_ui.as_ref().unwrap().document(), &initial);
    assert!(
        h.get_by_label("Undo UI edit")
            .accesskit_node()
            .is_disabled(),
        "one Apply adds exactly one undo entry"
    );

    click(&mut h, "Portrait 360 × 640");
    click(&mut h, "75%");
    select_resize_node(&mut h, "clip_panel");
    click(&mut h, "Resize UI size");
    resize_drag(&mut h, [30.0, 15.0]);
    let mut nested = initial.clone();
    let container = nested
        .nodes
        .iter_mut()
        .find(|node| node.id == "clip_panel")
        .unwrap();
    container.size[0] += 40;
    container.size[1] += 20;
    assert_eq!(
        h.state().collect_ui_preview.resize.state.document(),
        Some(&nested)
    );
    parity_document(
        &mut h,
        &font,
        &evidence,
        "resize-portrait-container",
        &nested,
    );
    click(&mut h, "Apply size");
    assert_eq!(h.state().collect_ui.as_ref().unwrap().document(), &nested);
    assert_eq!(
        fs::read(root.path().join("ui.json")).unwrap(),
        original_file
    );
    click(&mut h, "Resize UI size");
    canvas_click(&mut h, [220.0 * 0.75, 305.0 * 0.75]);
    assert_eq!(
        h.state().collect_ui_preview.state.last_action(),
        None,
        "outside the resized parent clip"
    );
    canvas_click(&mut h, [190.0 * 0.75, 305.0 * 0.75]);
    assert_eq!(
        h.state().collect_ui_preview.state.last_action(),
        Some(Action::Quit),
        "newly revealed child hit region"
    );
    assert_eq!(host_stamp(&h), stamp);
    click(&mut h, "Save UI document");
    let saved = prepared(root.path());
    assert_eq!(saved.ui().unwrap().document, nested);
    assert_eq!(saved.ui().unwrap().font, font);
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
    parity(&mut h, &font, &evidence, "resize-saved-reopened");
    // Reopening the same path with a new scene-load epoch invalidates a staged
    // transaction before any late Apply can reach the replacement editor.
    select_resize_node(&mut h, "play");
    if !h.state().collect_ui_preview.resize.resizing {
        click(&mut h, "Resize UI size");
    }
    resize_drag(&mut h, [20.0, 20.0]);
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
    assert_no_resize_draft(&h);
    assert!(!h.state().collect_ui_preview.state.is_available());
    assert_eq!(h.state().collect_ui.as_ref().unwrap().document(), &nested);
    let context = h.ctx.clone();
    crate::project::install_collect_presentation(h.state_mut(), replacement, &context);
    h.run_steps(4);
    click(&mut h, "Preview authored screens");
    // Explicit Play and absent-panel lifecycle boundaries.
    select_resize_node(&mut h, "play");
    if !h.state().collect_ui_preview.resize.resizing {
        click(&mut h, "Resize UI size");
    }
    resize_drag(&mut h, [20.0, 20.0]);
    assert!(h.state_mut().editor.start_play());
    h.run_steps(3);
    assert_no_resize_draft(&h);
    assert!(!h.state().collect_ui_preview.resize.resizing);
    assert!(h
        .get_by_label("Resize UI size")
        .accesskit_node()
        .is_disabled());
    assert_eq!(h.state().collect_ui.as_ref().unwrap().document(), &nested);
    h.state_mut().collect_ui = None;
    h.run_steps(3);
    assert!(!h.state().collect_ui_preview.state.is_available());
    assert_no_resize_draft(&h);
    fs::write(evidence.join("collect-ui-resize-acceptance.json"), serde_json::to_vec_pretty(&serde_json::json!({
        "schema":1, "production_editor_app":true, "software_vulkan":true, "adapter":adapter, "window_pixels":[1800,1200],
        "resize_ghost_without_context_rebuild":true, "release_candidate_build_once":true,
        "staging_changes_pixels_only":true, "cancel_restores_exact_pixels":true,
        "apply_one_undo":true, "expanded_button_and_parent_clipped_hit_regions":true, "child_anchors":[0,500,1000],
        "portrait_display_scale":0.75, "parent_resizes_child_clip_without_rewriting_descendants":true,
        "escape_focus_screen_resize_close_stale_document_play_cancel":true, "focus_snapshot_cancel":true, "staged_mode_selection_cancel":true, "inactive_move_selection_preserves_resize":true,
        "pointer_leave_late_release_cancel":true, "same_path_scene_epoch_cancel":true,
        "host_unchanged_during_authoring":true, "save_reopen":true,
        "production_collect_ui_pixel_parity_cases":6, "full_visible_controls":true,
        "native_os_window":false
    })).unwrap()).unwrap();
}

#[path = "collect_ui_alignment_acceptance_tests.rs"]
mod alignment_acceptance;

#[path = "collect_ui_distribution_acceptance_tests.rs"]
mod distribution_acceptance;
