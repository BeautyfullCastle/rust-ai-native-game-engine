//! Isolated production-module consumer, not an OS-window or exported-App driver.
use egui::{Event, PointerButton, Pos2, Rect};
use orr_bridge::Bridge;
use orr_render::{
    orr_rhi::{Rhi, TextureFormat, Wgpu, WgpuOptions},
    OffscreenTarget,
};
use orr_sample::{
    game_ui::{Action, GameUi, Hud, Screen},
    game_ui_gpu::{GpuOverlay, PendingOverlay},
    project_compositor::ProjectCompositor,
    project_runtime::PreparedRuntime,
};
use std::{fs, path::Path};

fn frame(
    ui: &mut GameUi,
    pending: &mut PendingOverlay,
    time: &mut f64,
    events: Vec<Event>,
    hud: Hud,
) -> Option<Action> {
    *time += 0.5;
    let (output, action) = ui.show(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(512.0, 512.0))),
            time: Some(*time),
            events,
            ..Default::default()
        },
        hud,
    );
    pending.push(output).unwrap();
    action
}
fn click(ui: &mut GameUi, pending: &mut PendingOverlay, time: &mut f64, action: Action, hud: Hud) {
    for _ in 0..3 {
        assert!(frame(ui, pending, time, vec![], hud).is_none());
    }
    let point = ui
        .buttons
        .iter()
        .find(|(a, _)| *a == action)
        .expect("actual preset widget")
        .1
        .center();
    assert!(frame(
        ui,
        pending,
        time,
        vec![
            Event::PointerMoved(point),
            Event::PointerButton {
                pos: point,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Default::default()
            }
        ],
        hud
    )
    .is_none());
    assert_eq!(
        frame(
            ui,
            pending,
            time,
            vec![Event::PointerButton {
                pos: point,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Default::default()
            }],
            hud
        ),
        Some(action)
    );
    // Apply only the action emitted by the real widget, never assign screen state.
    assert!(ui.apply(action));
    for _ in 0..3 {
        assert!(frame(ui, pending, time, vec![], hud).is_none());
    }
}
fn write_png(path: &Path, bytes: &[u8]) {
    let mut encoder = png::Encoder::new(fs::File::create_new(path).unwrap(), 512, 512);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(bytes)
        .unwrap();
}
fn text_pixels(no_text: &[u8], with_text: &[u8], region: [usize; 4]) -> (usize, usize) {
    let mut changed = 0;
    let mut bright = 0;
    for y in region[1]..region[3] {
        for x in region[0]..region[2] {
            let i = (y * 512 + x) * 4;
            if no_text[i..i + 4] != with_text[i..i + 4] {
                changed += 1;
                if with_text[i..i + 3].iter().all(|v| *v > 100) {
                    bright += 1;
                }
            }
        }
    }
    (changed, bright)
}
fn visible_preset(no_text: &[u8], with_text: &[u8], region: [usize; 4]) -> bool {
    let (changed, bright) = text_pixels(no_text, with_text, region);
    changed > 100 && bright > 30
}
// Suppress only the actual production HUD label's glyph color in a cloned
// frame. The TextShape override wins over explicit RichText colors while
// retaining the very same galley, positions, backgrounds, borders and controls.
fn suppress_hud_text(shape: &mut egui::Shape, bounds: &mut Option<Rect>) -> usize {
    match shape {
        egui::Shape::Text(text) if text.galley.job.text.starts_with("오러리 투기장 |") => {
            let mut count = 0;
            let mut rect = Rect::NOTHING;
            'title: for row in &text.galley.rows {
                for glyph in &row.glyphs {
                    if glyph.chr == '|' {
                        break 'title;
                    }
                    if ('\u{ac00}'..='\u{d7a3}').contains(&glyph.chr) {
                        rect = rect.union(
                            glyph
                                .logical_rect()
                                .translate(row.pos.to_vec2() + text.pos.to_vec2()),
                        );
                        count += 1;
                    }
                }
            }
            assert_eq!(
                count, 6,
                "actual HUD title's six Korean glyph layout entries"
            );
            *bounds = Some(rect.expand(1.0));
            text.override_text_color = Some(egui::Color32::TRANSPARENT);
            1
        }
        egui::Shape::Vec(shapes) => shapes
            .iter_mut()
            .map(|shape| suppress_hud_text(shape, bounds))
            .sum(),
        _ => 0,
    }
}
pub fn consume(bundle: &Path, evidence: &Path) {
    let project = bundle.join("project");
    let prepared = PreparedRuntime::open(&project).unwrap();
    let (seed, mut presentation, font) = prepared.into_launch_parts();
    let font = font.expect("bundle's admitted saved Korean preset");
    let admitted_ui = font.clone();
    let mut ui = GameUi::from_font(font.font, true).unwrap();
    let bridge = seed.bridge().unwrap();
    let snapshot = bridge.snapshot().unwrap();
    presentation.update(Some(&snapshot)).unwrap();
    assert!(!presentation.sprites().is_empty());
    let hud = Hud {
        tick: snapshot.tick(),
        verified_tick: snapshot.tick(),
        rollbacks: 0,
    };
    let gpu = Wgpu::headless(WgpuOptions::default()).expect("required GPU adapter");
    fs::write(evidence.join("ui-consumer-adapter.json"), serde_json::to_vec_pretty(&serde_json::json!({"name":gpu.adapter_name(), "software":gpu.is_software(), "coverage":"production modules, not OS-window input"})).unwrap()).unwrap();
    let format = TextureFormat::Rgba8UnormSrgb;
    let target = OffscreenTarget::new(&gpu, 512, 512, format);
    let mut compositor =
        ProjectCompositor::new(gpu.clone(), format, presentation.assets()).unwrap();
    compositor
        .draw(
            target.render_view(),
            (512, 512),
            presentation.shapes(),
            presentation.sprites(),
            &presentation.camera,
        )
        .unwrap();
    let no_ui = target.read_rgba8();
    write_png(&evidence.join("ui-negative-no-overlay.png"), &no_ui);
    korean_glyph_masks(
        &gpu,
        &mut compositor,
        &target,
        &presentation,
        admitted_ui,
        evidence,
    );
    let mut overlay = GpuOverlay::new(&gpu, format);
    let mut pending = PendingOverlay::default();
    let mut time = 0.0;
    for _ in 0..3 {
        assert!(frame(&mut ui, &mut pending, &mut time, vec![], hud).is_none());
    }
    let mut capture = |name: &str, ui: &GameUi, pending: &mut PendingOverlay| {
        compositor
            .draw_with_overlay(
                target.render_view(),
                (512, 512),
                presentation.shapes(),
                presentation.sprites(),
                &presentation.camera,
                |rhi, view, size| pending.paint(&mut overlay, rhi, view, size, &ui.context),
            )
            .unwrap();
        let pixels = target.read_rgba8();
        write_png(&evidence.join(name), &pixels);
        pixels
    };
    assert_eq!(ui.screen(), Screen::Title);
    let title = capture("ui-title.png", &ui, &mut pending);
    assert_ne!(title, no_ui);
    click(&mut ui, &mut pending, &mut time, Action::Play, hud);
    assert_eq!(ui.screen(), Screen::Playing);
    time += 0.5;
    let (playing_frame, action) = ui.show(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(512.0, 512.0))),
            time: Some(time),
            ..Default::default()
        },
        hud,
    );
    assert!(action.is_none());
    let playing_buttons = ui.buttons.clone();
    let mut control_frame = playing_frame.clone();
    let mut title_bounds = None;
    let suppressed: usize = control_frame
        .shapes
        .iter_mut()
        .map(|clipped| suppress_hud_text(&mut clipped.shape, &mut title_bounds))
        .sum();
    assert_eq!(suppressed, 1);
    let bounds = title_bounds.expect("production Korean HUD text shape");
    let region = [
        (bounds.min.x.floor().max(0.0) as usize).min(512),
        (bounds.min.y.floor().max(0.0) as usize).min(512),
        (bounds.max.x.ceil().max(0.0) as usize).min(512),
        (bounds.max.y.ceil().max(0.0) as usize).min(512),
    ];
    assert!(region[2] > region[0] && region[3] > region[1]);
    pending.push(playing_frame).unwrap();
    let playing = capture("ui-playing.png", &ui, &mut pending);
    // No UI layout is rerun: this is the same frame with only the matched
    // label's glyph color overridden, independently of label style defaults.
    let no_text_buttons = ui.buttons.clone();
    assert_eq!(no_text_buttons, playing_buttons);
    pending.push(control_frame).unwrap();
    let no_text = capture("ui-negative-no-text.png", &ui, &mut pending);
    assert!(
        visible_preset(&no_text, &playing, region),
        "production Korean title region must differ from identical-geometry textless control"
    );
    assert!(
        !visible_preset(&no_text, &no_text, region),
        "the same text oracle must reject its missing-text control"
    );
    let rectangles = |buttons: &[(Action, egui::Rect)]| {
        buttons.iter().map(|(action, rect)| serde_json::json!({"action":format!("{action:?}"),"min":[rect.min.x,rect.min.y],"max":[rect.max.x,rect.max.y]})).collect::<Vec<_>>()
    };
    let (changed, bright) = text_pixels(&no_text, &playing, region);
    fs::write(evidence.join("ui-text-oracle.json"), serde_json::to_vec_pretty(&serde_json::json!({
        "positive":"ui-playing.png","control":"ui-negative-no-text.png","no_overlay":"ui-negative-no-overlay.png",
        "positive_sha256":super::evidence::digest(&evidence.join("ui-playing.png")),"control_sha256":super::evidence::digest(&evidence.join("ui-negative-no-text.png")),"no_overlay_sha256":super::evidence::digest(&evidence.join("ui-negative-no-overlay.png")),
        "width":512,"height":512,"region_xyxy":region,"changed_pixels":changed,"bright_different_pixels":bright,
        "minimum_changed_exclusive":100,"minimum_bright_exclusive":30,"bright_channel_threshold_exclusive":100,
        "positive_buttons":rectangles(&playing_buttons),"control_buttons":rectangles(&no_text_buttons),"matching_layout":true,"control_method":"clone exact output and override only production HUD TextShape glyph color", "suppressed_text_shapes":suppressed,"region_text":"오러리 투기장",
        "control_against_itself":false,"control_text_color_rgba":[0,0,0,0],
        "claim":"visible text presence only, not OCR or every Korean glyph coverage; parser/corpus and installation byte identity are separate"
    })).unwrap()).unwrap();
    click(&mut ui, &mut pending, &mut time, Action::Menu, hud);
    assert_eq!(ui.screen(), Screen::Menu);
    let menu = capture("ui-menu.png", &ui, &mut pending);
    assert_ne!(menu, playing);
    click(&mut ui, &mut pending, &mut time, Action::Continue, hud);
    assert_eq!(ui.screen(), Screen::Playing);
    let continued = capture("ui-continued.png", &ui, &mut pending);
    assert!(visible_preset(&no_text, &continued, region));
    click(&mut ui, &mut pending, &mut time, Action::Menu, hud);
    click(&mut ui, &mut pending, &mut time, Action::Restart, hud);
    assert_eq!(ui.screen(), Screen::Playing);
    let restarted = capture("ui-restart-action.png", &ui, &mut pending);
    assert!(visible_preset(&no_text, &restarted, region));
    // App::dispatch_ui_action owns simulation restart and is private. This
    // consumer proves button dispatch + UI, not that separate bridge-reset route.
    println!(
        "UI controls and visible negative passed; Restart button/UI only, not App bridge reset"
    );
}

fn korean_glyph_masks(
    gpu: &Wgpu,
    compositor: &mut ProjectCompositor<Wgpu>,
    target: &OffscreenTarget<Wgpu>,
    presentation: &orr_sample::project_runtime::ProjectPresentation,
    prepared_ui: orr_sample::project::PreparedUi,
    evidence: &Path,
) {
    // Separate font-glyph oracle, not a claim about the preset's layout. Every
    // font byte comes from the relocated admitted bundle, never a test include.
    use sha2::{Digest, Sha256};
    let font_sha256: String = Sha256::digest(&prepared_ui.font)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let font_bytes = prepared_ui.font.len();
    let descriptor = prepared_ui.descriptor;
    assert_eq!(
        descriptor.profile,
        orr_package::ProjectUiProfile::ArenaKoreanV1
    );
    assert!(descriptor.document.is_none());
    let glyph_ui = GameUi::from_font(prepared_ui.font, true).unwrap();
    compositor
        .draw(
            target.render_view(),
            (512, 512),
            presentation.shapes(),
            presentation.sprites(),
            &presentation.camera,
        )
        .unwrap();
    let before = target.read_rgba8();
    write_png(&evidence.join("ui-glyphs-before.png"), &before);
    let mut widths = Vec::new();
    let output = glyph_ui.context.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(512.0, 512.0))),
            ..Default::default()
        },
        |root| {
            let painter = root.ctx().layer_painter(egui::LayerId::background());
            for (i, text) in ["오", "러", "리", "메", "뉴"].iter().enumerate() {
                let rect = painter.text(
                    egui::pos2(10.0 + i as f32 * 42.0, 100.0),
                    egui::Align2::LEFT_TOP,
                    text,
                    egui::FontId::proportional(30.0),
                    egui::Color32::WHITE,
                );
                widths.push(rect.width());
            }
        },
    );
    assert!(widths.iter().all(|width| *width > 10.0));
    let mut overlay = GpuOverlay::new(gpu, TextureFormat::Rgba8UnormSrgb);
    overlay
        .paint(
            gpu,
            target.render_view(),
            (512, 512),
            &glyph_ui.context,
            output,
        )
        .unwrap();
    let after = target.read_rgba8();
    write_png(&evidence.join("ui-glyphs-after.png"), &after);
    let masks: Vec<Vec<bool>> = (0..5)
        .map(|i| {
            (0..40)
                .flat_map(|y| (0..40).map(move |x| (x, y)))
                .map(|(x, y)| {
                    let offset = ((100 + y) * 512 + 10 + i * 42 + x) * 4;
                    before[offset..offset + 4] != after[offset..offset + 4]
                })
                .collect()
        })
        .collect();
    let counts: Vec<_> = masks
        .iter()
        .map(|mask| mask.iter().filter(|value| **value).count())
        .collect();
    assert!(counts.iter().all(|count| *count > 40));
    assert!(
        all_masks_distinct(&masks),
        "all five Korean masks must be pairwise distinct, not repeated tofu"
    );
    let outside = (300 * 512 + 300) * 4;
    assert_eq!(before[outside..outside + 4], after[outside..outside + 4]);
    fs::write(evidence.join("ui-glyph-oracle.json"),serde_json::to_vec_pretty(&serde_json::json!({
        "before":"ui-glyphs-before.png","after":"ui-glyphs-after.png","before_sha256":super::evidence::digest(&evidence.join("ui-glyphs-before.png")),"after_sha256":super::evidence::digest(&evidence.join("ui-glyphs-after.png")),"width":512,"height":512,"glyphs":["오","러","리","메","뉴"],"regions_xywh":(0..5).map(|i| [10+i*42,100,40,40]).collect::<Vec<_>>(),"changed_pixels":counts,"minimum_changed_exclusive":40,"all_masks_pairwise_distinct":true,"pair_comparisons":10,"unchanged_pixel_xy":[300,300],"font_source":"relocated bundle admission","font_sha256":font_sha256,"font_bytes":font_bytes,"descriptor":descriptor,"claim":"five distinct glyph masks, separate from actual preset text oracle"
    })).unwrap()).unwrap();
}

fn all_masks_distinct(masks: &[Vec<bool>]) -> bool {
    masks
        .iter()
        .enumerate()
        .all(|(i, left)| masks[i + 1..].iter().all(|right| left != right))
}

#[test]
fn korean_mask_oracle_rejects_nonadjacent_repeats() {
    let a = vec![true, false, false, false, false];
    let b = vec![false, true, false, false, false];
    let repeated = vec![a.clone(), b.clone(), a.clone(), b.clone(), a.clone()];
    assert!(
        repeated.windows(2).all(|pair| pair[0] != pair[1]),
        "the old adjacent-only check is insufficient"
    );
    assert!(!all_masks_distinct(&repeated));
    assert!(all_masks_distinct(&[
        a,
        b,
        vec![false, false, true, false, false],
        vec![false, false, false, true, false],
        vec![false, false, false, false, true]
    ]));
}
